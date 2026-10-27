//! Windows Recycle Bin delete and restore through `IFileOperation`.
//!
//! Every call runs on its own short-lived COM STA thread so the shell never
//! touches the caller's (GTK) thread. Deletion goes to the Recycle Bin or
//! fails; a permanent-delete fallback is never silent: only fixed local drives
//! are accepted, and `FOF_WANTNUKEWARNING` makes the shell ask instead of
//! destroying an item the bin cannot hold.
#![cfg(windows)]

pub mod activation;
pub mod context_menu;
pub mod desktop;
pub mod file_manager;
pub mod job;
pub mod launch;
mod operation_result;
mod recycle_bin;
pub mod shell_window;
pub mod volume;
pub mod window;
pub use recycle_bin::{
    RecycleBinEntry, RecycleBinListing, delete_recycled, list_recycled, original_path,
};

pub use job::{
    JobLimits, JobObject, SandboxedChild, SandboxedOutput, run_in_job, spawn_streaming_in_job,
};
pub use volume::{
    DriveType, VolumeInfo, eject_volume, enumerate_volumes, flush_volume, path_to_dos_device,
};

use std::{
    cell::RefCell,
    fs::File,
    io,
    os::windows::ffi::OsStrExt,
    os::windows::io::AsRawHandle,
    path::{Path, PathBuf},
    rc::Rc,
    thread,
};

/// Return the stable volume serial and file-index identity assigned by NTFS or
/// the underlying Windows file system.
///
/// The handle is opened only for identity inspection; callers still perform
/// their own metadata/reparse checks before using the result for caching.
pub fn file_identity(path: &Path) -> io::Result<(u32, u64)> {
    use windows::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let file = File::open(path)?;
    let handle = windows::Win32::Foundation::HANDLE(file.as_raw_handle() as *mut _);
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `handle` remains valid for the duration of this call and `info`
    // points to writable storage of the documented structure size.
    unsafe { GetFileInformationByHandle(handle, &mut info) }
        .map_err(|error| io::Error::other(format!("GetFileInformationByHandle failed: {error}")))?;
    let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Ok((info.dwVolumeSerialNumber, index))
}

use windows::{
    Win32::{
        Foundation::{E_FAIL, HWND},
        System::Com::{
            CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
            CoUninitialize,
        },
        UI::Shell::{
            BHID_EnumItems, FOLDERID_RecycleBinFolder, FileOperation, IEnumShellItems,
            IFileOperation, IFileOperationProgressSink, IFileOperationProgressSink_Impl,
            IShellItem, KF_FLAG_DEFAULT, SHCreateItemFromParsingName, SHGetKnownFolderItem,
            SIGDN_FILESYSPATH,
        },
    },
    core::{HRESULT, HSTRING, PCWSTR, Ref, implement},
};

const FOF_NOCONFIRMATION: u32 = 0x0010;
const FOF_ALLOWUNDO: u32 = 0x0040;
const FOF_NOERRORUI: u32 = 0x0400;
const FOF_NOCONFIRMMKDIR: u32 = 0x0200;
const FOF_WANTNUKEWARNING: u32 = 0x4000;
const FOFX_SHOWELEVATIONPROMPT: u32 = 0x0004_0000;
const FOFX_RECYCLEONDELETE: u32 = 0x0008_0000;
/// `HRESULT_FROM_WIN32(ERROR_FILE_EXISTS)`.
const FILE_EXISTS: HRESULT = HRESULT(0x8007_0050_u32 as i32);

/// An item that was moved into the Recycle Bin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecycledItem {
    /// Where the item lived before deletion.
    pub original: PathBuf,
    /// Its current `C:\$Recycle.Bin\<SID>\$R...` identity.
    pub recycled: PathBuf,
}

fn io_error(context: &str, error: &windows::core::Error) -> io::Error {
    io::Error::new(
        operation_result::error(error.code()).kind(),
        format!("{context}: {error}"),
    )
}

/// Return whether the current Windows token owns `path`.
///
/// Provider manifests and executables are user-installed inputs. Requiring
/// the owner SID to match the current token prevents a different local user
/// from planting a provider in a shared configuration directory. Fail closed
/// when Windows cannot read either security descriptor.
pub fn path_owned_by_current_user(path: &Path) -> bool {
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree, WIN32_ERROR},
        Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT},
        Security::{
            EqualSid, GetSecurityDescriptorOwner, GetTokenInformation, OWNER_SECURITY_INFORMATION,
            PSECURITY_DESCRIPTOR, PSID, TOKEN_QUERY, TOKEN_USER, TokenUser,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let mut owner = PSID::default();
    // SAFETY: The path is terminated and every output points to live storage.
    let status = unsafe {
        GetNamedSecurityInfoW(
            windows::core::PCWSTR(wide.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some((&mut owner) as *mut _),
            None,
            None,
            None,
            &mut descriptor,
        )
    };
    if status != WIN32_ERROR(0) || owner.0.is_null() || descriptor.0.is_null() {
        return false;
    }

    let mut token = HANDLE::default();
    // SAFETY: The process pseudo-handle is valid and token is writable.
    let token_opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if token_opened.is_err() {
        // SAFETY: GetNamedSecurityInfoW allocated descriptor with LocalAlloc.
        unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
        return false;
    }

    let mut required = 0u32;
    // SAFETY: A null buffer queries the allocation size for the owned token.
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut required) };
    if required == 0 {
        // SAFETY: This branch owns both allocations and releases each once.
        unsafe {
            let _ = CloseHandle(token);
            LocalFree(Some(HLOCAL(descriptor.0)));
        }
        return false;
    }
    let mut buffer = vec![0usize; (required as usize).div_ceil(size_of::<usize>())];
    // SAFETY: The aligned buffer contains at least required writable bytes.
    let token_result = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            required,
            &mut required,
        )
    };
    if token_result.is_err() {
        // SAFETY: This branch owns both allocations and releases each once.
        unsafe {
            let _ = CloseHandle(token);
            LocalFree(Some(HLOCAL(descriptor.0)));
        }
        return false;
    }
    // SAFETY: TokenUser initialized TOKEN_USER in the aligned, still-live buffer.
    let token_user = unsafe { &*(buffer.as_ptr() as *const TOKEN_USER) };
    let mut descriptor_owner = PSID::default();
    let mut defaulted = windows::core::BOOL(0);
    // SAFETY: The descriptor is live and both outputs are writable.
    let owner_result =
        unsafe { GetSecurityDescriptorOwner(descriptor, &mut descriptor_owner, &mut defaulted) };
    let matches = owner_result.is_ok()
        && !descriptor_owner.0.is_null()
        // SAFETY: Both SIDs are backed by live Windows-initialized allocations.
        && unsafe { EqualSid(descriptor_owner, token_user.User.Sid).is_ok() };
    // SAFETY: This function owns the token and descriptor and releases each once.
    unsafe {
        let _ = CloseHandle(token);
        LocalFree(Some(HLOCAL(descriptor.0)));
    }
    matches
}

/// Run `work` on a fresh COM STA thread and return its result.
fn with_sta<T: Send + 'static>(
    work: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> io::Result<T> {
    thread::Builder::new()
        .name("strata-recycle-bin".to_owned())
        .spawn(move || {
            // SAFETY: balanced with CoUninitialize below on the same thread.
            let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            if init.is_err() {
                return Err(io_error(
                    "COM initialization failed",
                    &windows::core::Error::from(init),
                ));
            }
            let result = work();
            // SAFETY: matches the successful CoInitializeEx above.
            unsafe { CoUninitialize() };
            result
        })?
        .join()
        .map_err(|_| io::Error::other("Recycle Bin worker panicked"))?
}

fn shell_item(path: &Path) -> io::Result<IShellItem> {
    let wide = HSTRING::from(path.as_os_str());
    // SAFETY: `wide` is a valid NUL-terminated string for the whole call.
    unsafe { SHCreateItemFromParsingName(&wide, None) }
        .map_err(|error| io_error("cannot open shell item", &error))
}

fn item_path(item: &IShellItem) -> io::Result<PathBuf> {
    // SAFETY: `item` is a live COM object; the returned string is freed below.
    let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }
        .map_err(|error| io_error("shell item has no file path", &error))?;
    // SAFETY: GetDisplayName returned a valid NUL-terminated wide string.
    let text = unsafe { name.to_string() };
    // SAFETY: the string was allocated by the shell with CoTaskMemAlloc.
    unsafe { CoTaskMemFree(Some(name.0.cast())) };
    text.map(PathBuf::from)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "shell path is not Unicode"))
}

#[derive(Default)]
struct Outcomes {
    recycled: Vec<RecycledItem>,
    failure: Option<HRESULT>,
}

#[implement(IFileOperationProgressSink)]
struct Sink {
    outcomes: Rc<RefCell<Outcomes>>,
    /// Reject any move whose destination already exists (restore never replaces).
    refuse_existing_destination: bool,
    /// Permanent deletion does not produce a Recycle Bin item to identify.
    expect_recycled_item: bool,
}

#[allow(
    non_snake_case,
    reason = "COM sink methods are named by the Win32 interface"
)]
impl IFileOperationProgressSink_Impl for Sink_Impl {
    fn StartOperations(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn FinishOperations(&self, result: HRESULT) -> windows::core::Result<()> {
        if result.is_err() {
            self.outcomes.borrow_mut().failure.get_or_insert(result);
        }
        Ok(())
    }
    fn PreRenameItem(
        &self,
        _flags: u32,
        _item: Ref<'_, IShellItem>,
        _name: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostRenameItem(
        &self,
        _flags: u32,
        _item: Ref<'_, IShellItem>,
        _name: &PCWSTR,
        _result: HRESULT,
        _new: Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreMoveItem(
        &self,
        _flags: u32,
        _item: Ref<'_, IShellItem>,
        destination: Ref<'_, IShellItem>,
        name: &PCWSTR,
    ) -> windows::core::Result<()> {
        if !self.refuse_existing_destination {
            return Ok(());
        }
        let folder = destination
            .ok()
            .map_err(|_| windows::core::Error::from(E_FAIL))?;
        let folder = item_path(folder).map_err(|_| windows::core::Error::from(E_FAIL))?;
        // SAFETY: the shell passes a valid NUL-terminated name for this call.
        let name = unsafe { name.to_string() }.map_err(|_| windows::core::Error::from(E_FAIL))?;
        if folder.join(name).symlink_metadata().is_ok() {
            return Err(windows::core::Error::from(FILE_EXISTS));
        }
        Ok(())
    }
    fn PostMoveItem(
        &self,
        _flags: u32,
        _item: Ref<'_, IShellItem>,
        _destination: Ref<'_, IShellItem>,
        _name: &PCWSTR,
        result: HRESULT,
        _new: Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        if result.is_err() {
            self.outcomes.borrow_mut().failure.get_or_insert(result);
        }
        Ok(())
    }
    fn PreCopyItem(
        &self,
        _flags: u32,
        _item: Ref<'_, IShellItem>,
        _destination: Ref<'_, IShellItem>,
        _name: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostCopyItem(
        &self,
        _flags: u32,
        _item: Ref<'_, IShellItem>,
        _destination: Ref<'_, IShellItem>,
        _name: &PCWSTR,
        _result: HRESULT,
        _new: Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreDeleteItem(&self, _flags: u32, _item: Ref<'_, IShellItem>) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostDeleteItem(
        &self,
        _flags: u32,
        item: Ref<'_, IShellItem>,
        result: HRESULT,
        created: Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        let mut outcomes = self.outcomes.borrow_mut();
        if result.is_err() {
            outcomes.failure.get_or_insert(result);
            return Ok(());
        }
        if !self.expect_recycled_item {
            return Ok(());
        }
        // A missing "newly created" item means the shell destroyed the item
        // instead of recycling it; report that instead of claiming success.
        let (Ok(original), Ok(recycled)) = (item.ok(), created.ok()) else {
            outcomes.failure.get_or_insert(E_FAIL);
            return Ok(());
        };
        match (item_path(original), item_path(recycled)) {
            (Ok(original), Ok(recycled)) => {
                outcomes.recycled.push(RecycledItem { original, recycled })
            }
            _ => {
                outcomes.failure.get_or_insert(E_FAIL);
            }
        }
        Ok(())
    }
    fn PreNewItem(
        &self,
        _flags: u32,
        _destination: Ref<'_, IShellItem>,
        _name: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostNewItem(
        &self,
        _flags: u32,
        _destination: Ref<'_, IShellItem>,
        _name: &PCWSTR,
        _template: &PCWSTR,
        _attributes: u32,
        _result: HRESULT,
        _new: Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn UpdateProgress(&self, _total: u32, _so_far: u32) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResetTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
}

fn new_operation(flags: u32) -> io::Result<IFileOperation> {
    // NOERRORUI otherwise suppresses the Shell's normal elevation path, turning
    // operations Explorer can authorize into an immediate access-denied error.
    // Windows still asks for consent; this does not grant or force elevation.
    let flags = flags | FOFX_SHOWELEVATIONPROMPT;
    // SAFETY: standard in-proc COM activation on an initialized STA thread.
    let operation: IFileOperation =
        unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL) }
            .map_err(|error| io_error("cannot create IFileOperation", &error))?;
    // SAFETY: `operation` is a live COM object.
    unsafe { operation.SetOperationFlags(windows::Win32::UI::Shell::FILEOPERATION_FLAGS(flags)) }
        .map_err(|error| io_error("cannot set operation flags", &error))?;
    // SAFETY: a null owner window is valid; the shell parents any dialog itself.
    unsafe { operation.SetOwnerWindow(HWND::default()) }
        .map_err(|error| io_error("cannot set owner window", &error))?;
    Ok(operation)
}

fn perform(operation: &IFileOperation, outcomes: &Rc<RefCell<Outcomes>>) -> io::Result<()> {
    // SAFETY: `operation` is live; this blocks this private STA thread only.
    let performed = unsafe { operation.PerformOperations() };
    // SAFETY: as above.
    let aborted = unsafe { operation.GetAnyOperationsAborted() }.map(|flag| flag.as_bool());
    operation_result::resolve(performed, aborted, outcomes.borrow().failure)
}

/// Classifies the drive root without probing the target path.
pub fn is_fixed_local_drive(path: &Path) -> bool {
    use windows::Win32::Storage::FileSystem::GetDriveTypeW;
    const DRIVE_FIXED: u32 = 3;
    let mut components = path.components();
    let Some(std::path::Component::Prefix(prefix)) = components.next() else {
        return false;
    };
    let std::path::Prefix::Disk(letter) = prefix.kind() else {
        return false;
    };
    let root = HSTRING::from(format!("{}:\\", letter.to_ascii_uppercase() as char));
    // SAFETY: `root` is a valid NUL-terminated drive root.
    unsafe { GetDriveTypeW(&root) == DRIVE_FIXED }
}

/// Move `paths` to the Recycle Bin, returning each item's bin identity.
///
/// Only absolute paths on fixed local drives are accepted. The call fails
/// rather than permanently deleting.
pub fn recycle(paths: &[PathBuf]) -> io::Result<Vec<RecycledItem>> {
    for path in paths {
        if !path.is_absolute() || !is_fixed_local_drive(path) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "the Recycle Bin is only used for absolute paths on fixed local drives",
            ));
        }
    }
    let paths = paths.to_vec();
    with_sta(move || {
        let outcomes = Rc::new(RefCell::new(Outcomes::default()));
        let sink: IFileOperationProgressSink = Sink {
            outcomes: outcomes.clone(),
            refuse_existing_destination: false,
            expect_recycled_item: true,
        }
        .into();
        let operation = new_operation(
            FOFX_RECYCLEONDELETE
                | FOF_ALLOWUNDO
                | FOF_WANTNUKEWARNING
                | FOF_NOERRORUI
                | FOF_NOCONFIRMATION,
        )?;
        for path in &paths {
            let item = shell_item(path)?;
            // SAFETY: both COM objects are live for the call.
            unsafe { operation.DeleteItem(&item, &sink) }
                .map_err(|error| io_error("cannot queue delete", &error))?;
        }
        perform(&operation, &outcomes)?;
        let recycled = std::mem::take(&mut outcomes.borrow_mut().recycled);
        if recycled.len() != paths.len() {
            return Err(io::Error::other(
                "the shell did not report every item as recycled",
            ));
        }
        Ok(recycled)
    })
}

/// Permanently delete absolute paths through the Windows Shell on a private
/// COM STA thread. The caller must have shown an explicit confirmation.
pub fn permanently_delete(paths: &[PathBuf]) -> io::Result<()> {
    if paths.iter().any(|path| !path.is_absolute()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "permanent deletion requires absolute paths",
        ));
    }
    let paths = paths.to_vec();
    with_sta(move || {
        let outcomes = Rc::new(RefCell::new(Outcomes::default()));
        let sink: IFileOperationProgressSink = Sink {
            outcomes: outcomes.clone(),
            refuse_existing_destination: false,
            expect_recycled_item: false,
        }
        .into();
        let operation = new_operation(FOF_NOCONFIRMATION | FOF_NOERRORUI)?;
        for path in &paths {
            let item = shell_item(path)?;
            // SAFETY: both COM objects are live for the call.
            unsafe { operation.DeleteItem(&item, &sink) }
                .map_err(|error| io_error("cannot queue permanent delete", &error))?;
        }
        perform(&operation, &outcomes)?;
        Ok(())
    })
}

fn find_in_bin(recycled: &Path) -> io::Result<IShellItem> {
    // SAFETY: standard shell known-folder lookup with no token.
    let bin: IShellItem =
        unsafe { SHGetKnownFolderItem(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, None) }
            .map_err(|error| io_error("cannot open the Recycle Bin", &error))?;
    // SAFETY: `bin` is live; BHID_EnumItems yields an item enumerator.
    let children: IEnumShellItems = unsafe { bin.BindToHandler(None, &BHID_EnumItems) }
        .map_err(|error| io_error("cannot list the Recycle Bin", &error))?;
    loop {
        let mut batch = [None];
        let mut fetched = 0u32;
        // SAFETY: `batch` and `fetched` outlive the call.
        let status = unsafe { children.Next(&mut batch, Some(&mut fetched)) };
        if status.is_err() || fetched == 0 {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "the item is no longer in the Recycle Bin",
            ));
        }
        let Some(child) = batch[0].take() else {
            continue;
        };
        if item_path(&child).is_ok_and(|path| path == recycled) {
            return Ok(child);
        }
    }
}

fn find_in_bin_by_original_path(original: &Path) -> io::Result<IShellItem> {
    // SAFETY: standard shell known-folder lookup with no token.
    let bin: IShellItem =
        unsafe { SHGetKnownFolderItem(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, None) }
            .map_err(|error| io_error("cannot open the Recycle Bin", &error))?;
    // SAFETY: `bin` is live; BHID_EnumItems yields an item enumerator.
    let children: IEnumShellItems = unsafe { bin.BindToHandler(None, &BHID_EnumItems) }
        .map_err(|error| io_error("cannot list the Recycle Bin", &error))?;

    let mut found = None;
    loop {
        let mut batch = [None];
        let mut fetched = 0u32;
        // SAFETY: `batch` and `fetched` outlive the call.
        let status = unsafe { children.Next(&mut batch, Some(&mut fetched)) };
        if status.is_err() || fetched == 0 {
            break;
        }
        let Some(child) = batch[0].take() else {
            continue;
        };

        if recycle_bin::original(&child).is_ok_and(|path| path == original) {
            if found.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "multiple deleted items share the original path",
                ));
            }
            found = Some(child);
        }
    }

    found.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "the item is no longer in the Recycle Bin",
        )
    })
}

/// Restore `item` to its original path through the shell, so Recycle Bin
/// bookkeeping stays consistent. An existing destination is never replaced and
/// a missing parent folder is never created.
pub fn restore(item: &RecycledItem) -> io::Result<()> {
    let item = item.clone();
    let (Some(parent), Some(name)) = (item.original.parent(), item.original.file_name()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the original path has no parent or name",
        ));
    };
    if !parent.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the original folder no longer exists",
        ));
    }
    if item.original.symlink_metadata().is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "an item already exists at the original path",
        ));
    }
    let parent = parent.to_path_buf();
    let name = HSTRING::from(name);
    with_sta(move || {
        let outcomes = Rc::new(RefCell::new(Outcomes::default()));
        let sink: IFileOperationProgressSink = Sink {
            outcomes: outcomes.clone(),
            refuse_existing_destination: true,
            expect_recycled_item: false,
        }
        .into();
        let operation = new_operation(FOF_NOERRORUI | FOF_NOCONFIRMATION | FOF_NOCONFIRMMKDIR)?;
        let source = find_in_bin(&item.recycled)?;
        if recycle_bin::original(&source)? != item.original {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "restore destination disagrees with the deleted item",
            ));
        }
        let destination = shell_item(&parent)?;
        // SAFETY: all COM objects and `name` are live for the call.
        unsafe { operation.MoveItem(&source, &destination, &name, &sink) }
            .map_err(|error| io_error("cannot queue restore", &error))?;
        perform(&operation, &outcomes)
    })
}

/// Restore an item by its original filesystem path when the temporary
/// recycled name ($R...) is not retained.
pub fn restore_by_original_path(original: &Path) -> io::Result<PathBuf> {
    let original = original.to_path_buf();
    let (Some(parent), Some(name)) = (original.parent(), original.file_name()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the original path has no parent or name",
        ));
    };
    if !parent.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the original folder no longer exists",
        ));
    }
    if original.symlink_metadata().is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "an item already exists at the original path",
        ));
    }
    let parent = parent.to_path_buf();
    let name = HSTRING::from(name);
    with_sta(move || {
        let outcomes = Rc::new(RefCell::new(Outcomes::default()));
        let sink: IFileOperationProgressSink = Sink {
            outcomes: outcomes.clone(),
            refuse_existing_destination: true,
            expect_recycled_item: false,
        }
        .into();
        let operation = new_operation(FOF_NOERRORUI | FOF_NOCONFIRMATION | FOF_NOCONFIRMMKDIR)?;
        let source = find_in_bin_by_original_path(&original)?;
        let destination = shell_item(&parent)?;
        // SAFETY: all COM objects and `name` are live for the call.
        unsafe { operation.MoveItem(&source, &destination, &name, &sink) }
            .map_err(|error| io_error("cannot queue restore", &error))?;
        perform(&operation, &outcomes)?;
        Ok(original)
    })
}
