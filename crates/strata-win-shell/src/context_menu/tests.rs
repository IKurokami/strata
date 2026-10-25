use super::*;

#[test]
fn invalid_selections_never_reach_shell_handlers() {
    let empty: [&Path; 0] = [];
    assert_eq!(show(&empty).unwrap_err().raw_os_error(), Some(87));
    for path in ["relative.txt", "C:\\bad\0.txt", ""] {
        assert_eq!(
            show(&[Path::new(path)]).unwrap_err().raw_os_error(),
            Some(87)
        );
    }
}

#[test]
fn shell_binds_complete_file_and_folder_selection_without_showing_a_menu() {
    // SAFETY: This test owns the apartment on its test thread until scope exit.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .unwrap();
    let _apartment = Apartment;
    let root = std::env::temp_dir().join(format!("strata-shell-context-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let file = root.join("ảnh & notes.txt");
    std::fs::write(&file, b"context menu fixture").unwrap();
    let folder = root.join("folder");
    std::fs::create_dir(&folder).unwrap();
    let result = (|| {
        for paths in [vec![&file], vec![&folder], vec![&file, &folder]] {
            let context = selection_context(&paths)?;
            // SAFETY: The menu is private to this STA and not displayed.
            let menu = Menu(unsafe { CreatePopupMenu() }.map_err(io::Error::other)?);
            // SAFETY: Context and menu stay live for the entire Shell query.
            unsafe { context.QueryContextMenu(menu.0, 0, FIRST_COMMAND, LAST_COMMAND, CMF_NORMAL) }
                .ok()
                .map_err(io::Error::other)?;
            // SAFETY: This test owns the live menu handle.
            assert!(unsafe { GetMenuItemCount(Some(menu.0)) } > 0);
        }
        assert!(selection_context(&[root.join("missing.txt")]).is_err());
        Ok::<_, io::Error>(())
    })();
    std::fs::remove_file(file).unwrap();
    std::fs::remove_dir(folder).unwrap();
    std::fs::remove_dir(root).unwrap();
    result.unwrap();
}
