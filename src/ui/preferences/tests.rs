// SPDX-License-Identifier: MIT

mod preferences;
mod text_size;

pub(super) fn read_saved_preferences() -> std::io::Result<super::Preferences> {
    super::flush_pending_preferences();
    super::read_preferences()
}
