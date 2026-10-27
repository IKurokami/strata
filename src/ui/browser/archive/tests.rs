// SPDX-License-Identifier: MIT

use super::*;
use crate::model::Location;
use crate::services::{ArchiveFormat, validate_basename};

#[test]
fn archive_folder_password_retries_keep_the_requested_destination_mode() {
    crate::test_support::gtk_test(
        "ui::browser::archive::tests::archive_folder_password_retries_keep_the_requested_destination_mode",
        || {
            use std::io::Write;
            let fixture = tempfile::tempdir().expect("archive test directory");
            let archive = fixture.path().join("secret.zip");
            let mut writer = zip::ZipWriter::new(
                std::fs::File::create(&archive).expect("create archive fixture"),
            );
            writer
                .start_file(
                    "notes.txt",
                    zip::write::SimpleFileOptions::default()
                        .with_aes_encryption(zip::AesMode::Aes256, "correct"),
                )
                .expect("archive fixture or password dialog control");
            writer
                .write_all(b"secret notes")
                .expect("write encrypted contents");
            writer.finish().expect("finish encrypted archive");
            let view = archive_test_view();
            let overlay = view.overlay();
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            view.state
                .extract_entry_into_folder(crate::test_support::operations::entry(
                    Location::local(&archive),
                ));
            for (password, expected_prompt) in
                [("wrong", "Password"), ("correct", "Invalid password")]
            {
                wait_for_archive(|| {
                    visible_texts(&overlay)
                        .iter()
                        .any(|text| text == expected_prompt)
                });
                let widgets = archive_widgets(&overlay);
                let field = widgets
                    .iter()
                    .find_map(|widget| widget.clone().downcast::<gtk::PasswordEntry>().ok())
                    .expect("archive fixture or password dialog control");
                field.set_text(password);
                let confirm = widgets
                    .iter()
                    .find_map(|widget| {
                        widget
                            .clone()
                            .downcast::<gtk::Button>()
                            .ok()
                            .filter(|button| button.label().as_deref() == Some("Extract"))
                    })
                    .expect("archive fixture or password dialog control");
                confirm.emit_clicked();
            }
            let extracted = fixture.path().join("secret/notes.txt");
            wait_for_archive(|| {
                extracted.is_file() && view.state.pending_extract_retry.borrow().is_none()
            });
            assert_eq!(
                std::fs::read(extracted).expect("read decrypted notes"),
                b"secret notes"
            );
            assert!(!fixture.path().join("secret (1)").exists());
            assert!(!fixture.path().join("notes.txt").exists());
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

fn wait_for_archive(condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !condition() && std::time::Instant::now() < deadline {
        let context = glib::MainContext::default();
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        condition(),
        "archive operation did not reach its expected state"
    );
}

fn archive_widgets(overlay: &gtk::Overlay) -> Vec<gtk::Widget> {
    let mut stack = vec![overlay.clone().upcast::<gtk::Widget>()];
    let mut widgets = Vec::new();
    while let Some(widget) = stack.pop() {
        if !widget.is_visible() {
            continue;
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            stack.push(next.clone());
            child = next.next_sibling();
        }
        widgets.push(widget);
    }
    widgets
}

#[test]
fn archive_folder_action_wraps_a_single_file_and_keeps_existing_folders() {
    crate::test_support::gtk_test(
        "ui::browser::archive::tests::archive_folder_action_wraps_a_single_file_and_keeps_existing_folders",
        || {
            let fixture = tempfile::tempdir().expect("archive test directory");
            let source = fixture.path().join("notes.txt");
            std::fs::write(&source, b"notes").expect("write source notes");
            let archive = fixture.path().join("bundle.tar.gz");
            let gzip = flate2::write::GzEncoder::new(
                std::fs::File::create(&archive).expect("create archive fixture"),
                flate2::Compression::default(),
            );
            let mut builder = tar::Builder::new(gzip);
            builder
                .append_path_with_name(source, "notes.txt")
                .expect("archive fixture or password dialog control");
            builder
                .into_inner()
                .expect("archive fixture or password dialog control")
                .finish()
                .expect("archive fixture or password dialog control");
            let view = archive_test_view();
            let overlay = view.overlay();
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            std::fs::create_dir(fixture.path().join("bundle")).expect("create existing folder");
            std::fs::write(fixture.path().join("bundle/keep"), b"keep")
                .expect("write preserved sibling");
            view.state
                .extract_entry_into_folder(crate::test_support::operations::entry(
                    Location::local(&archive),
                ));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let expected = fixture.path().join("bundle (1)/notes.txt");
            while !expected.is_file() && std::time::Instant::now() < deadline {
                let context = glib::MainContext::default();
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert_eq!(
                std::fs::read(expected).expect("read extracted notes"),
                b"notes"
            );
            assert_eq!(
                std::fs::read(fixture.path().join("bundle/keep")).expect("read preserved sibling"),
                b"keep"
            );
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn archive_names_strip_only_the_selected_dotted_extension() {
    assert_eq!(
        normalized_archive_name("backup.zip", ArchiveFormat::Zip),
        "backup"
    );
    assert_eq!(
        normalized_archive_name("backupzip", ArchiveFormat::Zip),
        "backupzip"
    );
    assert_eq!(
        normalized_archive_name("backup.tar.gz", ArchiveFormat::TarGz),
        "backup"
    );
    assert_eq!(
        normalized_archive_name("backup.rar", ArchiveFormat::Rar),
        "backup"
    );
    assert!(
        validate_basename(&normalized_archive_name(
            "../outside.zip",
            ArchiveFormat::Zip
        ))
        .is_err()
    );
}

#[test]
fn archive_collisions_use_the_final_name() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let destination = Location::local(root.path());
    assert!(!archive_has_collision(&destination, "archive.zip"));
    std::fs::write(root.path().join("archive.zip"), b"existing")?;
    assert!(archive_has_collision(&destination, "archive.zip"));
    Ok(())
}

fn visible_texts(overlay: &gtk::Overlay) -> Vec<String> {
    let mut texts = Vec::new();
    let mut stack = Vec::new();
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        stack.push(widget.clone());
        child = widget.next_sibling();
    }
    while let Some(widget) = stack.pop() {
        if !widget.is_visible() {
            continue;
        }
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            texts.push(label.label().to_string());
        }
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && let Some(label) = button.label()
        {
            texts.push(label.to_string());
        }
        let mut descendant = widget.first_child();
        while let Some(next) = descendant {
            stack.push(next.clone());
            descendant = next.next_sibling();
        }
    }
    texts
}

#[test]
fn only_a_password_failure_kind_opens_the_extract_password_dialog() {
    crate::test_support::gtk_test(
        "ui::browser::archive::tests::only_a_password_failure_kind_opens_the_extract_password_dialog",
        || {
            let message = "The password may be incorrect. Could not open `passwords.zip`.";
            for (password_failure, password_dialog, invalid_password) in [
                (None, false, false),
                (Some(PasswordFailure::Required), true, false),
                (Some(PasswordFailure::Incorrect), true, true),
            ] {
                let fixture = tempfile::tempdir().expect("archive fixture");
                let view = crate::ui::browser::BrowserView::new(
                    std::rc::Rc::new(crate::adapters::LocalFileSource),
                    crate::ui::browser::PeekBehavior::default(),
                );
                let overlay = view.overlay();
                let window = gtk::Window::builder().child(&overlay).build();
                window.present();
                let archive = Location::local(fixture.path().join("passwords.zip"));
                view.state.pending_extract_retry.replace(Some((
                    crate::test_support::operations::entry(archive),
                    Location::local(fixture.path()),
                    ExtractionMode::Automatic,
                )));

                view.state
                    .handle(&crate::app::BrowserEvent::OperationFailed {
                        message: message.to_owned(),
                        password_failure,
                    });
                let context = glib::MainContext::default();
                while context.pending() {
                    context.iteration(false);
                }

                let texts = visible_texts(&overlay);
                let label = format!("{password_failure:?}: {texts:?}");
                assert_eq!(
                    texts.iter().any(|text| text == "Password"),
                    password_dialog,
                    "{label}"
                );
                assert_eq!(
                    texts.iter().any(|text| text == "Invalid password"),
                    invalid_password,
                    "{label}"
                );
                assert_eq!(
                    texts
                        .iter()
                        .any(|text| text == "Unable to complete operation"),
                    !password_dialog,
                    "{label}"
                );
                assert_eq!(
                    texts.iter().any(|text| text == message),
                    !password_dialog,
                    "{label}"
                );
                window.destroy();
            }
        },
    );
}

fn archive_test_view() -> crate::ui::browser::BrowserView {
    let view = crate::ui::browser::BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        crate::ui::browser::PeekBehavior::default(),
    );
    #[cfg(windows)]
    let operations = crate::adapters::LocalOperationProvider::default();
    #[cfg(not(windows))]
    let operations = crate::adapters::LocalOperationProvider;
    view.set_operation_provider(Rc::new(operations));
    view
}
