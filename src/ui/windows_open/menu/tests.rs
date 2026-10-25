use super::*;

fn leaf(label: &str, command: u32, enabled: bool, checked: bool) -> Node {
    Node {
        label: label.into(),
        command: Some(command),
        enabled,
        checked,
        separator: false,
        submenu: false,
        children: Vec::new(),
    }
}

#[test]
fn exported_commands_preserve_nesting_state_and_dispatch_exact_ids() {
    let mut nested = leaf("Tools", 1, true, false);
    nested.command = None;
    nested.submenu = true;
    nested.children = vec![
        leaf("&Editor_工具\tCtrl+E", 17, true, false),
        leaf("Unavailable", 18, false, false),
    ];
    let separator = Node {
        label: String::new(),
        command: None,
        enabled: false,
        checked: false,
        separator: true,
        submenu: false,
        children: Vec::new(),
    };
    let disabled = Node {
        label: "Disabled parent".into(),
        command: None,
        enabled: false,
        checked: false,
        separator: false,
        submenu: true,
        children: vec![leaf("Child", 20, true, false)],
    };
    let nodes = vec![nested, separator, leaf("Checked", 19, true, true), disabled];
    validate(&nodes).expect("Shell menu fixture");
    let model = gio::Menu::new();
    let group = gio::SimpleActionGroup::new();
    let invoked = Rc::new(RefCell::new(Vec::new()));
    let seen = invoked.clone();
    let invoke: Rc<dyn Fn(u32)> = Rc::new(move |id| seen.borrow_mut().push(id));
    append_nodes(&model, &group, &nodes, 9, &mut 0, &invoke, true);
    let first = model.item_link(0, "section").expect("Shell menu fixture");
    let submenu = first.item_link(0, "submenu").expect("Shell menu fixture");
    let child = submenu.item_link(0, "section").expect("Shell menu fixture");
    assert_eq!(
        child
            .item_attribute_value(0, "label", None)
            .expect("Shell menu fixture")
            .get::<String>()
            .expect("Shell menu fixture"),
        "Editor__工具"
    );
    assert_eq!(
        child
            .item_attribute_value(0, "x-strata-shortcut", None)
            .expect("Shell menu fixture")
            .get::<String>()
            .expect("Shell menu fixture"),
        "Ctrl+E"
    );
    assert!(model.item_link(1, "section").is_some());
    for (id, enabled) in [
        ("command-9-1", true),
        ("command-9-2", false),
        ("command-9-4", false),
        ("command-9-5", false),
    ] {
        assert_eq!(group.is_action_enabled(id), enabled);
        group.activate_action(id, None);
    }
    group.activate_action("command-9-3", None);
    assert_eq!(
        group
            .action_state("command-9-3")
            .expect("Shell menu fixture")
            .get::<bool>(),
        Some(true)
    );
    assert_eq!(*invoked.borrow(), [17, 19]);
}

#[test]
fn shell_mnemonics_are_removed_without_translating_external_labels() {
    for (raw, expected) in [
        ("&Open", "Open"),
        ("R&&D", "R&D"),
        ("A_&&B", "A__&B"),
        ("&工具(&T)", "工具(T)"),
    ] {
        assert_eq!(label_parts(raw).0, expected);
    }
}

#[test]
fn retargeting_discards_stale_shell_snapshots_and_actions() {
    crate::test_support::gtk_test(
        "ui::windows_open::menu::tests::retargeting_discards_stale_shell_snapshots_and_actions",
        || {
            let model = gio::Menu::new();
            let popover = gtk::PopoverMenu::from_model(Some(&model));
            let owner = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let controller = Controller::new(model.clone(), &popover, owner.upcast_ref(), || {});
            controller.prepare(vec![PathBuf::from(r"C:\first.txt")]);
            let old = controller.generation.get();
            controller.accept(old, Ok(vec![leaf("First", 17, true, false)]));
            let old_action = format!("command-{old}-0");
            assert!(controller.group.has_action(&old_action));
            controller.prepare(vec![PathBuf::from(r"C:\second.txt")]);
            let current = controller.generation.get();
            controller.accept(old, Ok(vec![leaf("Stale", 18, true, false)]));
            assert!(!controller.group.has_action(&old_action));
            assert!(model.item_link(0, "section").is_none());
            controller.accept(current, Ok(vec![leaf("Second", 19, true, false)]));
            let section = model.item_link(0, "section").expect("Shell menu fixture");
            assert_eq!(
                section
                    .item_attribute_value(0, "label", None)
                    .expect("Shell menu fixture")
                    .get::<String>()
                    .as_deref(),
                Some("Second")
            );
            controller.reset();
            assert!(!controller.group.has_action(&format!("command-{current}-0")));
        },
    );
}
