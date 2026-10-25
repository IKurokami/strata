use gtk::{gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    io,
    path::PathBuf,
    rc::Rc,
};

mod transport;

#[derive(Debug, serde::Deserialize)]
struct Node {
    label: String,
    command: Option<u32>,
    enabled: bool,
    checked: bool,
    separator: bool,
    submenu: bool,
    children: Vec<Node>,
}

fn validate(nodes: &[Node]) -> io::Result<()> {
    fn visit(nodes: &[Node], depth: usize, remaining: &mut usize) -> io::Result<()> {
        if depth > 12 {
            return Err(io::Error::other("Shell menu nesting limit exceeded"));
        }
        for node in nodes {
            *remaining = remaining
                .checked_sub(1)
                .ok_or_else(|| io::Error::other("Shell menu item limit exceeded"))?;
            if node.label.len() > 32768
                || node.command.is_some_and(|id| !(1..=0x7fff).contains(&id))
                || (node.separator && (node.command.is_some() || node.submenu))
                || (node.submenu && node.command.is_some())
                || (!node.submenu && !node.children.is_empty())
            {
                return Err(io::Error::other("Invalid Shell menu item"));
            }
            visit(&node.children, depth + 1, remaining)?;
        }
        Ok(())
    }
    visit(nodes, 0, &mut 4096)
}

fn label_parts(raw: &str) -> (String, &str) {
    let (label, shortcut) = raw.split_once('\t').unwrap_or((raw, ""));
    let mut label_chars = label.chars().peekable();
    let mut text = String::new();
    while let Some(character) = label_chars.next() {
        if character != '&' {
            text.push(character);
        } else if label_chars.peek() == Some(&'&') {
            text.push('&');
            label_chars.next();
        }
    }
    (text.replace('_', "__"), shortcut)
}

fn append_nodes(
    model: &gio::Menu,
    group: &gio::SimpleActionGroup,
    nodes: &[Node],
    generation: u64,
    next: &mut usize,
    invoke: &Rc<dyn Fn(u32)>,
    enabled: bool,
) {
    let mut section = gio::Menu::new();
    for node in nodes {
        if node.separator {
            if section.n_items() > 0 {
                model.append_section(None, &section);
                section = gio::Menu::new();
            }
            continue;
        }
        let name = format!("command-{generation}-{}", *next);
        *next += 1;
        let action = if node.checked && !node.submenu {
            gio::SimpleAction::new_stateful(&name, None, &true.to_variant())
        } else {
            gio::SimpleAction::new(&name, None)
        };
        action.set_enabled(enabled && node.enabled && (node.submenu || node.command.is_some()));
        if let Some(command) = node.command {
            let invoke = invoke.clone();
            action.connect_activate(move |_, _| invoke(command));
        }
        group.add_action(&action);
        let (label, shortcut) = label_parts(&node.label);
        let label = if label.is_empty() {
            crate::i18n::tr("Windows commands")
        } else {
            label
        };
        let item = gio::MenuItem::new(Some(&label), Some(&format!("shell.{name}")));
        item.set_attribute_value("x-strata-shortcut", Some(&shortcut.to_variant()));
        if node.submenu {
            let children = gio::Menu::new();
            append_nodes(
                &children,
                group,
                &node.children,
                generation,
                next,
                invoke,
                enabled && node.enabled,
            );
            if children.n_items() == 0 {
                children.append(
                    Some(&crate::i18n::tr("No Windows commands available")),
                    Some("shell.status"),
                );
            }
            item.set_submenu(Some(&children));
        }
        section.append_item(&item);
    }
    if section.n_items() > 0 {
        model.append_section(None, &section);
    }
}

pub(in crate::ui) struct Controller {
    model: gio::Menu,
    group: gio::SimpleActionGroup,
    open: gio::SimpleAction,
    generation: Cell<u64>,
    paths: RefCell<Vec<PathBuf>>,
    session: RefCell<Option<transport::Session>>,
    started: Cell<bool>,
    popover: glib::WeakRef<gtk::PopoverMenu>,
    owner: glib::WeakRef<gtk::Widget>,
    changed: Rc<dyn Fn()>,
}

impl Controller {
    pub(in crate::ui) fn new(
        model: gio::Menu,
        popover: &gtk::PopoverMenu,
        owner: &gtk::Widget,
        changed: impl Fn() + 'static,
    ) -> Rc<Self> {
        let group = gio::SimpleActionGroup::new();
        let open = gio::SimpleAction::new_stateful("open", None, &false.to_variant());
        let status = gio::SimpleAction::new("status", None);
        status.set_enabled(false);
        group.add_action(&status);
        group.add_action(&open);
        popover.insert_action_group("shell", Some(&group));
        let controller = Rc::new(Self {
            model,
            group,
            open,
            generation: Cell::new(0),
            paths: RefCell::new(Vec::new()),
            session: RefCell::new(None),
            started: Cell::new(false),
            popover: popover.downgrade(),
            owner: owner.downgrade(),
            changed: Rc::new(changed),
        });
        let weak = Rc::downgrade(&controller);
        controller.open.connect_change_state(move |action, value| {
            if let Some(value) = value.and_then(|value| value.get::<bool>()) {
                action.set_state(&value.to_variant());
                if value && let Some(controller) = weak.upgrade() {
                    controller.start();
                }
            }
        });
        let weak = Rc::downgrade(&controller);
        popover.connect_closed(move |popover| {
            let Some(controller) = weak.upgrade() else {
                return;
            };
            let generation = controller.generation.get();
            let weak = Rc::downgrade(&controller);
            let popover = popover.downgrade();
            // GtkModelButton closes before activating the leaf. Let that action
            // submit its command before cancelling an unselected session.
            glib::idle_add_local_once(move || {
                if let Some(controller) = weak.upgrade()
                    && controller.generation.get() == generation
                    && popover
                        .upgrade()
                        .is_none_or(|popover| !popover.is_visible())
                {
                    controller.reset();
                }
            });
        });
        controller
    }

    fn reset(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.session.take();
        self.started.set(false);
        self.model.remove_all();
        for name in self.group.list_actions() {
            if name.starts_with("command-") {
                self.group.remove_action(&name);
            }
        }
    }

    pub(in crate::ui) fn prepare(&self, paths: Vec<PathBuf>) {
        self.reset();
        self.open.set_state(&false.to_variant());
        self.paths.replace(paths);
        if !self.paths.borrow().is_empty() {
            self.model
                .append(Some(&crate::i18n::tr("Loading…")), Some("shell.status"));
        }
    }

    fn start(self: &Rc<Self>) {
        if self.paths.borrow().is_empty() || self.started.replace(true) {
            return;
        }
        self.model.remove_all();
        self.model
            .append(Some(&crate::i18n::tr("Loading…")), Some("shell.status"));
        let generation = self.generation.get();
        let Ok((session, snapshot)) = transport::Session::start(self.paths.borrow().clone()) else {
            self.failure();
            return;
        };
        self.session.replace(Some(session));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = futures_lite::future::race(
                async {
                    snapshot
                        .await
                        .unwrap_or_else(|_| Err(io::Error::other("Shell helper stopped")))
                },
                async {
                    glib::timeout_future_seconds(12).await;
                    Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "Shell menu loading timed out",
                    ))
                },
            )
            .await;
            let Some(controller) = weak.upgrade() else {
                return;
            };
            if controller.generation.get() != generation {
                return;
            }
            controller.accept(generation, result);
        });
        (self.changed)();
    }

    fn accept(self: &Rc<Self>, generation: u64, result: io::Result<Vec<Node>>) {
        if self.generation.get() != generation {
            return;
        }
        match result {
            Ok(nodes) => {
                self.model.remove_all();
                let weak = Rc::downgrade(self);
                let invoke: Rc<dyn Fn(u32)> = Rc::new(move |command| {
                    if let Some(controller) = weak.upgrade()
                        && controller.generation.get() == generation
                    {
                        controller.invoke(command);
                    }
                });
                append_nodes(
                    &self.model,
                    &self.group,
                    &nodes,
                    generation,
                    &mut 0,
                    &invoke,
                    true,
                );
                if self.model.n_items() == 0 {
                    self.model.append(
                        Some(&crate::i18n::tr("No Windows commands available")),
                        Some("shell.status"),
                    );
                }
                (self.changed)();
            }
            Err(_) => self.failure(),
        }
    }

    fn failure(&self) {
        self.session.take();
        self.model.remove_all();
        self.model.append(
            Some(&crate::i18n::tr("Unable to load Windows commands")),
            Some("shell.status"),
        );
        (self.changed)();
    }

    fn invoke(&self, command: u32) {
        let completion = self
            .session
            .borrow_mut()
            .as_mut()
            .and_then(|session| session.invoke(command));
        if let Some(popover) = self.popover.upgrade() {
            popover.popdown();
        }
        let owner = self.owner.clone();
        glib::spawn_future_local(async move {
            let result = match completion {
                Some(completion) => completion
                    .await
                    .unwrap_or_else(|_| Err(io::Error::other("Shell helper stopped"))),
                None => Err(io::Error::other("Shell menu session expired")),
            };
            if let Some(owner) = owner.upgrade()
                && let Err(error) = result
            {
                tracing::warn!(kind = ?error.kind(), "Windows Shell command failed");
                crate::ui::modal::show_error_dialog(
                    &owner,
                    &crate::i18n::tr("Unable to complete operation"),
                    &crate::i18n::tr("Unable to run Windows command"),
                );
            }
        });
    }
}

#[cfg(test)]
mod tests;
