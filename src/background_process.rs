// SPDX-License-Identifier: MIT

use std::process::Command;

/// Decoder/probe utilities must not allocate a console when Strata has none.
pub(crate) fn configure(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}
