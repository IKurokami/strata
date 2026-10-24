// SPDX-License-Identifier: MIT

use super::*;
use gtk::glib;

#[expect(
    unsafe_code,
    reason = "Exercise the serialized payload through the Windows Shell consumer API"
)]
fn shell_paths(data: &[u8]) -> Vec<Vec<u16>> {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
        fn GlobalFree(memory: *mut c_void) -> *mut c_void;
    }
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn DragQueryFileW(drop: *mut c_void, index: u32, name: *mut u16, capacity: u32) -> u32;
    }
    // SAFETY: GMEM_FIXED yields a directly addressable HGLOBAL of the requested size.
    let memory = unsafe { GlobalAlloc(0, data.len()) };
    assert!(!memory.is_null());
    // SAFETY: The non-overlapping allocation holds the complete payload.
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), memory.cast::<u8>(), data.len()) };
    // SAFETY: The initialized HGLOBAL remains live for every synchronous Shell call.
    let count = unsafe { DragQueryFileW(memory, u32::MAX, std::ptr::null_mut(), 0) };
    let paths = (0..count)
        .map(|index| {
            // SAFETY: The index belongs to the live payload; null output queries its length.
            let length = unsafe { DragQueryFileW(memory, index, std::ptr::null_mut(), 0) };
            let mut name = vec![0; length as usize + 1];
            // SAFETY: The output buffer includes space for the reported path and NUL.
            let copied = unsafe { DragQueryFileW(memory, index, name.as_mut_ptr(), length + 1) };
            name.truncate(copied as usize);
            name
        })
        .collect();
    // SAFETY: All Shell calls have returned; this allocation is owned only here.
    assert!(unsafe { GlobalFree(memory) }.is_null());
    paths
}

#[test]
fn shell_reads_all_unicode_paths_in_selection_order() {
    let names = [
        r"C:\Strata fixture\ảnh tiếng Việt # 100%.png".to_string(),
        r"\\server\shared folder\日本語.txt".to_string(),
        format!(
            r"C:\Strata fixture\{}\file.txt",
            "long directory ".repeat(30)
        ),
    ];
    let files: Vec<_> = names.iter().map(gio::File::for_path).collect();
    let expected: Vec<Vec<u16>> = files
        .iter()
        .map(|file| file.path().unwrap().as_os_str().encode_wide().collect())
        .collect();
    let data = file_drop_data(&files).expect("native file payload");
    assert_eq!(shell_paths(&data), expected);
}

#[test]
fn native_payload_rejects_empty_and_mixed_remote_selections() {
    assert!(file_drop_data(&[]).is_none());
    assert!(
        file_drop_data(&[
            gio::File::for_path(r"C:\local.txt"),
            gio::File::for_uri("sftp://example.invalid/remote.txt"),
        ])
        .is_none()
    );
}

#[test]
fn drag_provider_preserves_gtk_files_and_supplies_shell_payload() {
    crate::test_support::gtk_test(
        "ui::browser::clipboard::windows::tests::drag_provider_preserves_gtk_files_and_supplies_shell_payload",
        || {
            let directory = tempfile::tempdir().unwrap();
            let paths = [
                directory.path().join("ảnh #%.png"),
                directory.path().join("folder"),
            ];
            let entries: Vec<_> = paths
                .iter()
                .map(|path| {
                    crate::test_support::operations::entry(crate::model::Location::local(path))
                })
                .collect();
            let provider = super::super::file_drag_content(&entries).expect("drag provider");
            let value = provider
                .value(gtk::gdk::FileList::static_type())
                .expect("GTK drag retained");
            let files = value.get::<gtk::gdk::FileList>().unwrap().files();
            assert_eq!(
                files
                    .iter()
                    .map(|file| file.path().unwrap())
                    .collect::<Vec<_>>(),
                paths
            );
            let stream = gio::MemoryOutputStream::new_resizable();
            glib::MainContext::default()
                .block_on(provider.write_mime_type_future(
                    FILE_DROP_MIME,
                    &stream,
                    glib::Priority::DEFAULT,
                ))
                .expect("native MIME transfer");
            stream.close(None::<&gio::Cancellable>).unwrap();
            let data = stream.steal_as_bytes();
            assert_eq!(
                shell_paths(data.as_ref()),
                paths
                    .iter()
                    .map(|path| path.as_os_str().encode_wide().collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            );
            super::super::clear_active_drag();
        },
    );
}
