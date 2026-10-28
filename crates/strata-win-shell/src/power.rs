// SPDX-License-Identifier: MIT

/// Query power policy without waking storage or requiring elevated access.
pub fn on_battery() -> bool {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: status is a live, correctly sized output structure.
    unsafe { GetSystemPowerStatus(&mut status) }.is_ok()
        && (status.ACLineStatus == 0 || status.SystemStatusFlag != 0)
}
