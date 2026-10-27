// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn material_associations_prioritize_names_and_longest_suffixes_without_losing_case_variants() {
    assert_eq!(
        icon_for_name("package.json", false),
        icon_for_name("PACKAGE.JSON", false)
    );
    assert_ne!(
        icon_for_name("package.json", false),
        icon_for_name("ordinary.json", false)
    );
    assert_ne!(
        icon_for_name("widget.d.ts", false),
        icon_for_name("widget.ts", false)
    );
    assert_eq!(
        icon_for_name("widget.d.ts", false),
        icon_for_name("WIDGET.D.TS", false)
    );
    assert_ne!(
        icon_for_name(".github", true),
        icon_for_name("unrecognized-folder", true)
    );
    assert_ne!(
        icon_for_name(".gitignore", false),
        icon_for_name("unknown.ignore", false)
    );
}

#[test]
fn material_icons_use_parent_context_for_nested_associations() {
    assert_ne!(
        icon_for_name("/home/me/.github/workflows", true),
        icon_for_name("/home/me/other/workflows", true)
    );
    assert_ne!(
        icon_for_name("/home/me/.config/graphqlrc", false),
        icon_for_name("/home/me/other/graphqlrc", false)
    );
}
