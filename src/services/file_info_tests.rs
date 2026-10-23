// SPDX-License-Identifier: MIT

use super::gio_file_type;
use gtk::gio;

#[test]
fn missing_standard_type_is_reported_without_querying_file_type() {
    let info = gio::FileInfo::new();
    assert_eq!(gio_file_type(&info), None);

    info.set_file_type(gio::FileType::Regular);
    assert_eq!(gio_file_type(&info), Some(gio::FileType::Regular));
}
