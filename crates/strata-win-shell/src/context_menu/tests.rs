use super::*;

#[windows::core::implement(IContextMenu)]
struct UnicodeHandler(std::rc::Rc<std::cell::Cell<bool>>);

#[allow(non_snake_case)]
impl windows::Win32::UI::Shell::IContextMenu_Impl for UnicodeHandler_Impl {
    fn QueryContextMenu(&self, _: HMENU, _: u32, _: u32, _: u32, _: u32) -> windows::core::HRESULT {
        windows::core::HRESULT(0)
    }
    fn GetCommandString(
        &self,
        _: usize,
        _: u32,
        _: *const u32,
        _: PSTR,
        _: u32,
    ) -> windows::core::Result<()> {
        Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_NOTIMPL,
        ))
    }
    fn InvokeCommand(&self, info: *const CMINVOKECOMMANDINFO) -> windows::core::Result<()> {
        // SAFETY: The test's invoking code supplies the documented live structure.
        let base = unsafe { &*info };
        if base.cbSize as usize != std::mem::size_of::<CMINVOKECOMMANDINFOEX>()
            || base.fMask & SEE_MASK_UNICODE == 0
            || base.fMask & SEE_MASK_NOASYNC == 0
        {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_INVALIDARG,
            ));
        }
        // SAFETY: The checked size identifies the extended structure.
        let extended = unsafe { &*info.cast::<CMINVOKECOMMANDINFOEX>() };
        if extended.lpVerbW.0 as usize != 16 || extended.lpVerb.0 as usize != 16 {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_INVALIDARG,
            ));
        }
        self.0.set(true);
        Ok(())
    }
}

#[test]
fn unicode_handlers_receive_the_selected_command_before_helper_exit() {
    let completed = std::rc::Rc::new(std::cell::Cell::new(false));
    let handler: IContextMenu = UnicodeHandler(completed.clone()).into();
    invoke_command(&handler, HWND::default(), 17).expect("invoke selected Unicode command");
    assert!(completed.get());
    assert!(invoke_command(&handler, HWND::default(), 18).is_err());
}

#[test]
fn invalid_selections_never_reach_shell_handlers() {
    let empty: [&Path; 0] = [];
    assert_eq!(Session::new(&empty).err().unwrap().raw_os_error(), Some(87));
    for path in ["relative.txt", "C:\\bad\0.txt", ""] {
        assert_eq!(
            Session::new(&[Path::new(path)])
                .err()
                .unwrap()
                .raw_os_error(),
            Some(87)
        );
    }
}

#[test]
fn shell_binds_complete_file_and_folder_selection_without_showing_a_menu() {
    let root = std::env::temp_dir().join(format!("strata-shell-context-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let file = root.join("ảnh & notes.txt");
    std::fs::write(&file, b"context menu fixture").unwrap();
    let folder = root.join("folder");
    std::fs::create_dir(&folder).unwrap();
    let result = (|| {
        for paths in [vec![&file], vec![&folder], vec![&file, &folder]] {
            let mut session = Session::new(&paths)?;
            assert_eq!(
                session.invoke(FIRST_COMMAND).unwrap_err().raw_os_error(),
                Some(87)
            );
            let snapshot = session.snapshot()?;
            assert!(!snapshot.is_empty());
            fn contains_verb(entries: &[Entry], verb: &str) -> bool {
                entries.iter().any(|entry| {
                    entry.verb.eq_ignore_ascii_case(verb) || contains_verb(&entry.children, verb)
                })
            }
            assert!(
                contains_verb(&snapshot, "copy"),
                "native Copy canonical verb"
            );
            assert!(contains_verb(&snapshot, "cut"), "native Cut canonical verb");
            assert!(!session.allowed.is_empty());
            assert_eq!(session.invoke(0).unwrap_err().raw_os_error(), Some(87));
            assert_eq!(
                session.invoke(LAST_COMMAND + 1).unwrap_err().raw_os_error(),
                Some(87)
            );
            session.pump_messages();
        }
        assert!(Session::new(&[root.join("missing.txt")]).is_err());
        Ok::<_, io::Error>(())
    })();
    std::fs::remove_file(file).unwrap();
    std::fs::remove_dir(folder).unwrap();
    std::fs::remove_dir(root).unwrap();
    result.unwrap();
}
