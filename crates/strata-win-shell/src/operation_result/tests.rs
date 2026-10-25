// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn item_failures_survive_aggregate_abort_and_keep_their_hresult() {
    for (code, kind) in [
        (
            HRESULT(0x8007_0005u32 as i32),
            io::ErrorKind::PermissionDenied,
        ),
        (
            COPYENGINE_E_ACCESS_DENIED_SRC,
            io::ErrorKind::PermissionDenied,
        ),
        (
            COPYENGINE_E_SHARING_VIOLATION_DEST,
            io::ErrorKind::ResourceBusy,
        ),
        (HRESULT(0x8007_0020u32 as i32), io::ErrorKind::ResourceBusy),
        (COPYENGINE_E_DISK_FULL, io::ErrorKind::StorageFull),
    ] {
        let result = resolve(Err(Error::from_hresult(E_ABORT)), Ok(true), Some(code));
        let failure = result.expect_err("the item failure must be retained");
        assert_eq!(failure.kind(), kind);
        assert_eq!(
            failure
                .get_ref()
                .and_then(|source| source.downcast_ref::<Error>())
                .expect("HRESULT source")
                .code(),
            code
        );
    }
    assert_eq!(
        resolve(
            Err(Error::from_hresult(COPYENGINE_E_DISK_FULL)),
            Ok(true),
            Some(E_ABORT)
        )
        .expect_err("aggregate failure survives a generic callback abort")
        .kind(),
        io::ErrorKind::StorageFull
    );
}

#[test]
fn cancellation_is_not_success_and_status_query_failures_are_not_ignored() {
    for code in [
        E_ABORT,
        COPYENGINE_E_USER_CANCELLED,
        HRESULT(0x8007_04c7u32 as i32),
    ] {
        assert_eq!(
            resolve(Err(Error::from_hresult(code)), Ok(false), None)
                .expect_err("cancelled operation")
                .kind(),
            io::ErrorKind::Interrupted
        );
    }
    assert_eq!(
        resolve(Ok(()), Ok(true), None)
            .expect_err("silent abort")
            .kind(),
        io::ErrorKind::Interrupted
    );
    assert!(
        resolve(
            Ok(()),
            Err(Error::from_hresult(windows::Win32::Foundation::E_FAIL)),
            None
        )
        .is_err()
    );
    assert!(resolve(Ok(()), Ok(false), None).is_ok());
}
