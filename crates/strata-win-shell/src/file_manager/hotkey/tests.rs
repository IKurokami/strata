// SPDX-License-Identifier: MIT
use super::*;

#[test]
fn desktop_shortcut_dispatches_once_and_consumes_its_matching_release() {
    for win in [0x5b, 0x5c] {
        let mut state = Routing::default();
        assert_eq!(state.event(win, true, false), Decision::Pass);
        assert_eq!(state.event(0x45, true, false), Decision::Activate);
        assert_eq!(state.event(0x45, true, false), Decision::Consume);
        assert_eq!(state.event(win, false, false), Decision::Pass);
        assert_eq!(state.event(0x45, false, false), Decision::Consume);
        assert_eq!(state.event(0x45, true, false), Decision::Pass);
        state.event(win, true, false);
        assert_eq!(state.event(0x45, true, false), Decision::Activate);
    }
}

#[test]
fn other_shortcuts_typing_and_menu_mask_input_pass_through() {
    for modifier in [0x10, 0x11, 0x12, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5] {
        let mut state = Routing::default();
        state.event(0x5b, true, false);
        state.event(modifier, true, false);
        assert_eq!(state.event(0x45, true, false), Decision::Pass);
        assert_eq!(state.event(0x45, false, false), Decision::Pass);
        state.event(modifier, false, false);
        assert_eq!(state.event(0x45, true, true), Decision::Pass);
        assert_eq!(state.event(0x41, true, false), Decision::Pass);
        assert_eq!(state.event(0x45, true, false), Decision::Activate);
    }
}
