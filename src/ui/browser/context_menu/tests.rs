// SPDX-License-Identifier: MIT

use super::*;
use std::time::{Duration, Instant};

#[cfg(not(windows))]
mod menus;
#[cfg(not(windows))]
mod open_with;

#[track_caller]
pub(in crate::ui::browser::context_menu) fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "menu fixture did not settle");
        let context = glib::MainContext::default();
        for _ in 0..100 {
            if !context.pending() {
                break;
            }
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
