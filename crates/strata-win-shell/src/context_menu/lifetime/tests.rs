use super::*;
use windows::Win32::UI::Shell::{SHGetInstanceExplorer, SHGetThreadRef};

#[test]
fn shell_references_hold_the_host_until_released_and_registrations_are_cleared() {
    let lifetime = ShellLifetime::new().expect("Shell host lifetime");
    // SAFETY: Read the registrations owned by this test's live lifetime guard.
    let process = unsafe { SHGetInstanceExplorer() }.expect("registered host reference");
    // SAFETY: Read this thread's registered Shell reference.
    let thread = unsafe { SHGetThreadRef() }.expect("registered thread reference");
    assert!(lifetime.busy());
    drop(process);
    assert!(lifetime.busy());
    drop(thread);
    assert!(!lifetime.busy());
    drop(lifetime);
    // SAFETY: The guard has cleared this test's registration.
    assert!(unsafe { SHGetThreadRef() }.is_err());
}
