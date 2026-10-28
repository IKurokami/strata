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


def test_disk_usage_uses_shared_inline_rename(strata):
    source = strata.fixture.path("readme.md")
    contents = source.read_text()
    strata.switch_view("Disk Usage")
    item = strata.wait(
        lambda: next(
            (node for node in strata.pane().find_all(role="list item")
             if node.name.startswith("readme.md · ")),
            None,
        ),
        "the source file in the usage snapshot",
    )
    strata.pointer.click(item)
    strata.keyboard.press("F2")
    strata.wait(
        lambda: strata.window.find(role="text", name="Rename", states={"editable", "focused"}),
        "the shared inline rename editor",
    )
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("renamed.md")
    strata.keyboard.press("Return")
    destination = strata.fixture.path("renamed.md")
    strata.wait(lambda: destination.exists() and not source.exists(), "the shared rename operation")
    assert destination.read_text() == contents
    strata.wait(
        lambda: any(node.name.startswith("renamed.md · ") for node in strata.pane().find_all(role="list item")),
        "the usage snapshot to reconcile the rename",
    )
