// SPDX-License-Identifier: MIT
use super::super::{Actual, RouteStatus, tests::Fixture};
use super::*;
use std::{
    cell::Cell,
    panic::{AssertUnwindSafe, catch_unwind},
};
use windows::Win32::System::Registry::{REG_DWORD, REG_EXPAND_SZ};

thread_local! {static FAILURE:Cell<Option<(usize,bool)>>=const {Cell::new(None)};}
pub(in crate::file_manager) fn after_write() -> io::Result<()> {
    FAILURE.with(|failure| {
        if let Some((remaining, crash)) = failure.get() {
            if remaining <= 1 {
                failure.set(None);
                if crash {
                    panic!("simulated process interruption");
                }
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            failure.set(Some((remaining - 1, crash)));
        }
        Ok(())
    })
}

#[test]
fn frozen_identity_recovers_missing_metadata_and_extended_target_paths() {
    let fixture = Fixture::new();
    let reg = &fixture.registration;
    reg.enable(&fixture.executable(), &fixture.proxy(), "Strata")
        .unwrap();
    for name in ["Executable", "Proxy", "Label"] {
        registry::set(&reg.state, name, None).unwrap();
    }
    assert_eq!(reg.probe(None).unwrap().actual, Actual::Broken);
    reg.enable_policy(&fixture.executable(), &fixture.proxy(), "Strata", false)
        .unwrap();
    assert_eq!(reg.probe(None).unwrap().actual, Actual::Active);
    let mut extended = std::ffi::OsString::from(r"\\?\");
    extended.push(fixture.executable());
    reg.disable(std::path::Path::new(&extended)).unwrap();
    assert_removed(reg);
    assert!(Journal::load_if_present(reg).unwrap().is_none());
}

#[test]
fn v2_routes_and_typed_values_restore_exactly_without_creating_autostart() {
    for original in [
        None,
        Some(Value::text(OsStr::new("")).unwrap()),
        Some(Value {
            kind: REG_EXPAND_SZ,
            data: Value::text(OsStr::new("%SystemRoot%\\original.exe"))
                .unwrap()
                .data,
        }),
        Some(Value {
            kind: REG_BINARY,
            data: vec![0, 255, 10, 0],
        }),
        Some(Value {
            kind: REG_DWORD,
            data: 42u32.to_le_bytes().to_vec(),
        }),
    ] {
        let fixture = Fixture::new();
        let reg = &fixture.registration;
        let folder = format!(r"{}\Folder\shell\open\command", reg.classes);
        registry::set(&folder, "", original.as_ref()).unwrap();
        registry::set(&folder, "DelegateExecute", original.as_ref()).unwrap();
        registry::set(&reg.run, super::super::RUN_VALUE, original.as_ref()).unwrap();
        reg.enable(&fixture.executable(), &fixture.proxy(), "Strata")
            .unwrap();
        let status = reg.probe(Some(&fixture.executable())).unwrap();
        assert_eq!(status.actual, Actual::Active);
        assert_eq!(status.win_e_clsid, RouteStatus::Owned);
        assert_eq!(status.folder_explore, RouteStatus::Owned);
        assert_eq!(
            registry::get(&reg.run, super::super::RUN_VALUE).unwrap(),
            original
        );
        reg.enable(&fixture.executable(), &fixture.proxy(), "Strata")
            .unwrap();
        let updated_proxy = fixture.directory.join("other.exe");
        reg.enable_policy(
            &fixture.executable(),
            &updated_proxy,
            "Strata localized",
            false,
        )
        .unwrap();
        let command = super::super::routing::command(&updated_proxy, false).unwrap();
        assert_eq!(registry::get(&folder, "").unwrap(), Some(command));
        assert_eq!(
            registry::get(&reg.verb("Directory"), "").unwrap(),
            text("Strata localized").unwrap()
        );
        reg.disable(&fixture.executable()).unwrap();
        reg.disable(&fixture.executable()).unwrap();
        assert_eq!(registry::get(&folder, "").unwrap(), original);
        assert_eq!(registry::get(&folder, "DelegateExecute").unwrap(), original);
        assert!(Journal::load_if_present(reg).unwrap().is_none());
    }
}

#[test]
fn absent_keys_are_removed_but_foreign_delegate_and_subkeys_survive() {
    let fixture = Fixture::new();
    let reg = &fixture.registration;
    reg.enable(&fixture.executable(), &fixture.proxy(), "Strata")
        .unwrap();
    let folder = format!(r"{}\Folder\shell\open\command", reg.classes);
    let foreign = Value::text(OsStr::new("{foreign-delegate}")).unwrap();
    registry::set(&folder, "DelegateExecute", Some(&foreign)).unwrap();
    let child = format!(r"{folder}\Foreign");
    registry::set(&child, "keep", Some(&foreign)).unwrap();
    let before = registry::get(&reg.state, NAME).unwrap();
    assert_eq!(reg.probe(None).unwrap().actual, Actual::Conflict);
    assert_eq!(
        registry::get(&reg.state, NAME).unwrap(),
        before,
        "probe is read-only"
    );
    reg.disable(&fixture.executable()).unwrap();
    assert_eq!(
        registry::get(&folder, "DelegateExecute").unwrap(),
        Some(foreign.clone())
    );
    assert_eq!(registry::get(&child, "keep").unwrap(), Some(foreign));
    assert!(registry::get(&folder, "").unwrap().is_none());
    assert_eq!(
        Journal::load_if_present(reg).unwrap().unwrap().phase,
        Phase::Conflict
    );
    let empty = Fixture::new();
    empty
        .registration
        .enable(&empty.executable(), &empty.proxy(), "Strata")
        .unwrap();
    empty.registration.disable(&empty.executable()).unwrap();
    assert_removed(&empty.registration);
}

#[test]
fn legacy_migration_keeps_pre_v1_backups_and_survives_old_journal_deletion() {
    for takeover in [false, true] {
        let mut fixture = Fixture::new();
        let old_state = format!(r"{}\Legacy", fixture.key);
        fixture.registration.legacy_state = Some(old_state.clone());
        let legacy = Registration {
            state: old_state.clone(),
            legacy_state: None,
            ..fixture.registration.clone()
        };
        let original = Value {
            kind: REG_EXPAND_SZ,
            data: Value::text(OsStr::new("Prior.Manager")).unwrap().data,
        };
        for class in super::super::CLASSES {
            registry::set(&legacy.shell(class), "", Some(&original)).unwrap();
        }
        registry::set(&legacy.run, super::super::RUN_VALUE, Some(&original)).unwrap();
        legacy
            .enable_v1(&fixture.executable(), &fixture.proxy(), "Strata v1")
            .unwrap();
        assert!(legacy.enabled_v1(&fixture.executable()).unwrap());
        let proxy = fixture.directory.join("proxy-v2.exe");
        std::fs::write(&proxy, b"v2 fixture").unwrap();
        let reg = &fixture.registration;
        reg.enable_policy(&fixture.executable(), &proxy, "Strata", false)
            .unwrap();
        registry::remove_tree(&old_state).unwrap();
        let folder = format!(r"{}\Folder\shell\open\command", reg.classes);
        let foreign = Value::text(OsStr::new("A later file manager")).unwrap();
        if takeover {
            registry::set(&folder, "", Some(&foreign)).unwrap();
            reg.enable_policy(&fixture.executable(), &proxy, "Strata", true)
                .unwrap();
        }
        reg.disable(&fixture.executable()).unwrap();
        for class in super::super::CLASSES {
            assert_eq!(
                registry::get(&reg.shell(class), "").unwrap(),
                Some(original.clone())
            );
        }
        assert_eq!(
            registry::get(&reg.run, super::super::RUN_VALUE).unwrap(),
            Some(original)
        );
        assert!(reg.owner().unwrap().is_none());
        assert_eq!(
            registry::get(&folder, "").unwrap(),
            takeover.then_some(foreign)
        );
    }
}

#[test]
fn every_interrupted_route_write_recovers_from_the_durable_journal() {
    for step in 1..=12 {
        let fixture = Fixture::new();
        let reg = &fixture.registration;
        FAILURE.with(|failure| failure.set(Some((step, true))));
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            reg.enable(&fixture.executable(), &fixture.proxy(), "Strata")
        }));
        assert!(outcome.is_err(), "failpoint {step} was exercised");
        assert!(Journal::load_if_present(reg).unwrap().is_some());
        reg.disable(&fixture.executable()).unwrap();
        assert!(Journal::load_if_present(reg).unwrap().is_none());
        assert_removed(reg);
    }
}

#[test]
fn write_denied_rolls_back_and_corrupt_journals_are_never_executed() {
    let fixture = Fixture::new();
    let reg = &fixture.registration;
    FAILURE.with(|failure| failure.set(Some((5, false))));
    assert_eq!(
        reg.enable(&fixture.executable(), &fixture.proxy(), "Strata")
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_removed(reg);
    reg.enable(&fixture.executable(), &fixture.proxy(), "Strata")
        .unwrap();
    let journal = registry::get(&reg.state, NAME).unwrap().unwrap();
    registry::set(&reg.state, NAME, None).unwrap();
    assert!(
        reg.disable(&fixture.executable()).is_err(),
        "lost backups must block uninstall"
    );
    assert!(
        reg.enable_policy(&fixture.executable(), &fixture.proxy(), "Strata", false)
            .is_err(),
        "repair must not back up its own dangling routes"
    );
    registry::set(&reg.state, "Executable", None).unwrap();
    assert!(
        reg.owner().is_err(),
        "orphan metadata must not be treated as disabled"
    );
    registry::set(
        &reg.state,
        "Executable",
        Some(&Value::text(fixture.executable().as_os_str()).unwrap()),
    )
    .unwrap();
    for data in [
        vec![],
        b"invalid".to_vec(),
        journal.data[..journal.data.len() - 1].to_vec(),
    ] {
        registry::set(
            &reg.state,
            NAME,
            Some(&Value {
                kind: REG_BINARY,
                data,
            }),
        )
        .unwrap();
        assert!(Journal::load_if_present(reg).is_err());
        assert!(reg.disable(&fixture.executable()).is_err());
    }
    registry::set(&reg.state, NAME, Some(&journal)).unwrap();
    reg.disable(&fixture.executable()).unwrap();
}

#[test]
fn automatic_reconcile_preserves_foreign_routes_and_requires_explicit_takeover() {
    let fixture = Fixture::new();
    let reg = &fixture.registration;
    let path = format!(r"{}\Folder\shell\open\command", reg.classes);
    let foreign = Value::text(OsStr::new("Other.Manager")).unwrap();
    registry::set(&path, "", Some(&foreign)).unwrap();
    assert_eq!(
        reg.enable_policy(&fixture.executable(), &fixture.proxy(), "Strata", false)
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(registry::get(&path, "").unwrap(), Some(foreign.clone()));
    reg.enable_policy(&fixture.executable(), &fixture.proxy(), "Strata", true)
        .unwrap();
    let changed = Value::text(OsStr::new("Another.Manager")).unwrap();
    registry::set(&path, "", Some(&changed)).unwrap();
    assert!(
        reg.enable_policy(&fixture.executable(), &fixture.proxy(), "Strata", false)
            .is_err()
    );
    assert_eq!(registry::get(&path, "").unwrap(), Some(changed.clone()));
    reg.enable_policy(&fixture.executable(), &fixture.proxy(), "Strata", true)
        .unwrap();
    reg.disable(&fixture.executable()).unwrap();
    assert_eq!(registry::get(&path, "").unwrap(), Some(changed));
}

fn assert_removed(reg: &Registration) {
    for class in ["Folder", "Directory", "Drive", "CLSID"] {
        assert!(
            !registry::key_exists(&format!(r"{}\{class}", reg.classes)).unwrap(),
            "owned handler key survived rollback"
        );
    }
}
