# SPDX-License-Identifier: MIT
"""Disk Usage commands and exact-folder view persistence."""


def _navigate(strata, path):
    strata.keyboard.press("ctrl+l")
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(str(path))
    strata.keyboard.press("Return")
    strata.wait_for_directory(path.name)


def test_disk_usage_is_remembered_for_its_folder_without_child_inheritance(strata):
    root = strata.fixture.root
    strata.switch_view("Disk Usage")
    strata.wait(
        lambda: strata.environment.read_folder_mode(root) == "disk_usage",
        "the explicit Disk Usage choice to persist",
    )
    strata.wait(
        lambda: strata.pane().find(role="list item", rendered=False),
        "the size snapshot to expose accessible files",
    )
    _navigate(strata, root / "documents")
    strata.wait_for_view("Columns")
    assert strata.environment.read_folder_mode(root / "documents") == "columns"
    _navigate(strata, root)
    strata.wait_for_view("Disk Usage")
    strata.keyboard.press("ctrl+3")
    strata.wait_for_view("List")
    strata.wait(lambda: strata.environment.read_folder_mode(root) == "list", "the updated exact-folder mode")
    _navigate(strata, root / "documents")
    strata.wait_for_view("Columns")
    _navigate(strata, root)
    strata.wait_for_view("List")


def test_disk_usage_ctrl_shortcut_and_reset_to_columns(strata):
    root = strata.fixture.root
    strata.keyboard.press("ctrl+4")
    strata.wait_for_view("Disk Usage")
    strata.wait(lambda: strata.environment.read_folder_mode(root) == "disk_usage", "Ctrl+4 mode persistence")
    strata.keyboard.press("ctrl+1")
    strata.wait_for_view("Columns")
    strata.wait(lambda: strata.environment.read_folder_mode(root) == "columns", "Columns to remove the mode override")
    _navigate(strata, root / "documents")
    _navigate(strata, root)
    strata.wait_for_view("Columns")
