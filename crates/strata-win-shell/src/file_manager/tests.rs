// SPDX-License-Identifier: MIT

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    registration: Registration,
    directory: PathBuf,
    key: String,
}

impl Fixture {
    fn new() -> Self {
        let token = format!(
            "{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let key = format!(r"Software\Strata\Tests\FileManager\{token}");
        let directory = std::env::temp_dir().join(format!("strata-shell-{token}"));
        std::fs::create_dir(&directory).expect("private fixture");
        for name in ["Strata 中文.exe", "proxy.exe", "other.exe"] {
            std::fs::write(directory.join(name), b"fixture").expect("executable fixture");
        }
        Self {
            registration: Registration {
                classes: format!(r"{key}\Classes"),
                state: format!(r"{key}\State"),
                run: format!(r"{key}\Run"),
            },
            directory,
            key,
        }
    }
    fn executable(&self) -> PathBuf {
        self.directory.join("Strata 中文.exe")
    }
    fn proxy(&self) -> PathBuf {
        self.directory.join("proxy.exe")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = registry::remove_tree(&self.key);
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn shortcut_reuses_the_running_endpoint_and_falls_back_after_shutdown() {
    let fixture = Fixture::new();
    let registration = &fixture.registration;
    registration
        .enable(&fixture.executable(), &fixture.proxy(), "Strata")
        .unwrap();
    assert!(!registration.activate_running());
    let scope = format!("{}-activation-中文", fixture.key);
    let server = crate::activation::Server::bind(&scope).unwrap();
    let request = b"opaque browser activation";
    registration
        .set_activation(Some((&scope, request)))
        .unwrap();
    assert!(registration.activate_running());
    assert_eq!(server.receive().as_deref(), Some(request.as_slice()));
    assert!(registration.set_activation(Some(("", request))).is_err());
    assert!(
        registration.activate_running(),
        "invalid updates retain the prior route"
    );
    assert_eq!(server.receive().as_deref(), Some(request.as_slice()));
    drop(server);
    assert!(
        !registration.activate_running(),
        "missing primary uses normal launch"
    );
    let server = crate::activation::Server::bind(&scope).unwrap();
    registration.disable(&fixture.executable()).unwrap();
    assert!(!registration.activate_running());
    assert!(
        server.receive().is_none(),
        "unregister clears the direct route"
    );
}

#[test]
fn malformed_shortcut_metadata_uses_normal_launch() {
    use windows::Win32::System::Registry::REG_BINARY;

    let fixture = Fixture::new();
    let registration = &fixture.registration;
    for data in [
        vec![],
        vec![1, 0, 0],
        vec![255; 4],
        vec![1, 0, 0, 0, 255, 42],
    ] {
        registry::set(
            &registration.state,
            "Activation",
            Some(&Value {
                kind: REG_BINARY,
                data,
            }),
        )
        .unwrap();
        assert!(!registration.activate_running());
    }
    registration.set_activation(None).unwrap();
    assert!(
        registry::get(&registration.state, "Activation")
            .unwrap()
            .is_none()
    );
}

#[test]
fn toggling_restores_missing_empty_and_custom_folder_defaults_exactly() {
    for original in [
        None,
        Some(Value::text(OsStr::new("")).unwrap()),
        Some(Value::text(OsStr::new("Other.Open")).unwrap()),
    ] {
        let fixture = Fixture::new();
        let registration = &fixture.registration;
        for class in CLASSES {
            registry::set(&registration.shell(class), "", original.as_ref()).unwrap();
        }
        registry::set(&registration.run, RUN_VALUE, original.as_ref()).unwrap();
        registration
            .enable(&fixture.executable(), &fixture.proxy(), "Strata")
            .unwrap();
        assert!(registration.enabled(&fixture.executable()).unwrap());
        registration
            .enable(&fixture.executable(), &fixture.proxy(), "Strata")
            .unwrap();
        registration.disable(&fixture.executable()).unwrap();
        assert_eq!(
            registry::get(&registration.run, RUN_VALUE).unwrap(),
            original
        );
        for class in CLASSES {
            assert_eq!(
                registry::get(&registration.shell(class), "").unwrap(),
                original
            );
            assert!(
                registry::get(&format!(r"{}\command", registration.verb(class)), "")
                    .unwrap()
                    .is_none()
            );
        }
        assert!(!registration.enabled(&fixture.executable()).unwrap());
    }
}

#[test]
fn unregister_preserves_external_changes_and_another_installation() {
    let fixture = Fixture::new();
    let registration = &fixture.registration;
    registration
        .enable(&fixture.executable(), &fixture.proxy(), "Strata")
        .unwrap();
    let external = Value::text(OsStr::new("Other.Open")).unwrap();
    registry::set(&registration.shell("Directory"), "", Some(&external)).unwrap();
    registry::set(&registration.run, RUN_VALUE, Some(&external)).unwrap();
    registration
        .disable(&fixture.directory.join("other.exe"))
        .unwrap();
    assert!(registration.owner().unwrap().is_some());
    registration.disable(&fixture.executable()).unwrap();
    assert_eq!(
        registry::get(&registration.shell("Directory"), "").unwrap(),
        Some(external)
    );
    assert_eq!(
        registry::get(&registration.run, RUN_VALUE).unwrap(),
        Some(Value::text(OsStr::new("Other.Open")).unwrap())
    );
    assert!(
        registry::get(&registration.shell("Drive"), "")
            .unwrap()
            .is_none()
    );
}

#[test]
fn interrupted_registration_resumes_without_losing_the_backup() {
    let fixture = Fixture::new();
    let registration = &fixture.registration;
    let original = Value::text(OsStr::new("Previous.Open")).unwrap();
    registry::set(&registration.shell("Drive"), "", Some(&original)).unwrap();
    registration
        .enable(&fixture.executable(), &fixture.proxy(), "Strata")
        .unwrap();
    registry::set(&registration.shell("Drive"), "", Some(&original)).unwrap();
    registration
        .enable(&fixture.executable(), &fixture.proxy(), "Strata")
        .unwrap();
    registration.disable(&fixture.executable()).unwrap();
    assert_eq!(
        registry::get(&registration.shell("Drive"), "").unwrap(),
        Some(original)
    );
}

#[test]
fn unowned_verbs_and_missing_executables_are_never_overwritten() {
    let fixture = Fixture::new();
    let registration = &fixture.registration;
    assert!(
        registration
            .enable(
                &fixture.directory.join("missing.exe"),
                &fixture.proxy(),
                "Strata"
            )
            .is_err()
    );
    let foreign = Value::text(OsStr::new("foreign command")).unwrap();
    let command = format!(r"{}\command", registration.verb("Directory"));
    registry::set(&command, "", Some(&foreign)).unwrap();
    assert!(
        registration
            .enable(&fixture.executable(), &fixture.proxy(), "Strata")
            .is_err()
    );
    assert_eq!(registry::get(&command, "").unwrap(), Some(foreign));
    assert!(registration.owner().unwrap().is_none());
}

#[test]
fn deleted_portable_target_repairs_defaults_before_falling_back_to_explorer() {
    let fixture = Fixture::new();
    let registration = &fixture.registration;
    let previous = Value::text(OsStr::new("Previous.Open")).unwrap();
    registry::set(&registration.shell("Directory"), "", Some(&previous)).unwrap();
    registration
        .enable(&fixture.executable(), &fixture.proxy(), "Strata")
        .unwrap();
    std::fs::remove_file(fixture.executable()).unwrap();
    assert!(registration.available_target().unwrap().is_none());
    assert_eq!(
        registry::get(&registration.shell("Directory"), "").unwrap(),
        Some(previous)
    );
    assert!(
        registry::get(&registration.shell("Drive"), "")
            .unwrap()
            .is_none()
    );
    assert!(registration.owner().unwrap().is_none());
}
