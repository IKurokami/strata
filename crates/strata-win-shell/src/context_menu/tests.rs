use super::*;

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
