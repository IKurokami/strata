// SPDX-License-Identifier: MIT

use super::PreferenceManager;
use gtk::prelude::*;
use std::rc::Rc;

pub(super) fn button(preferences: &Rc<PreferenceManager>) -> gtk::MenuButton {
    let button = gtk::MenuButton::new();
    button.set_valign(gtk::Align::Center);
    button.add_css_class("form-control");
    button.add_css_class("settings-choice");
    button.add_css_class("choice-button");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let value = gtk::Label::new(None);
    value.set_hexpand(true);
    value.set_xalign(0.0);
    value.set_ellipsize(gtk::pango::EllipsizeMode::End);
    content.append(&value);
    content.append(&crate::assets::primary_icon(
        crate::assets::icons::ARROW_DOWN,
        14,
    ));
    button.set_child(Some(&content));
    crate::ui::accessibility::set_label(&button, &crate::i18n::tr("Interface font"));
    preferences.bind_preference(
        &value,
        PreferenceManager::interface_font_family,
        |widget, family| {
            if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                label.set_text(&family);
            }
        },
    );
    let manager = preferences.clone();
    button.set_create_popup_func(move |button| {
        if button.popover().is_some() {
            return;
        }
        let mut names: Vec<String> = button
            .pango_context()
            .font_map()
            .map(|map| {
                map.list_families()
                    .into_iter()
                    .map(|family| family.name().to_string())
                    .collect()
            })
            .unwrap_or_default();
        names.sort_by_cached_key(|name| name.to_lowercase());
        names.dedup();
        let refs: Vec<_> = names.iter().map(String::as_str).collect();
        let model = gtk::StringList::new(&refs);
        let filter = gtk::StringFilter::builder()
            .expression(gtk::PropertyExpression::new(
                gtk::StringObject::static_type(),
                None::<gtk::Expression>,
                "string",
            ))
            .ignore_case(true)
            .match_mode(gtk::StringFilterMatchMode::Substring)
            .build();
        let filtered = gtk::FilterListModel::new(Some(model), Some(filter.clone()));
        let selected = gtk::SingleSelection::new(Some(filtered.clone()));
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, object| {
            let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let label = gtk::Label::new(None);
            label.set_xalign(0.0);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.add_css_class("font-choice-row");
            item.set_child(Some(&label));
        });
        factory.connect_bind(|_, object| {
            let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let Some(font) = item.item().and_downcast::<gtk::StringObject>() else {
                return;
            };
            let Some(label) = item.child().and_downcast::<gtk::Label>() else {
                return;
            };
            label.set_text(&font.string());
            let mut description = gtk::pango::FontDescription::new();
            description.set_family(&font.string());
            let attrs = gtk::pango::AttrList::new();
            attrs.insert(gtk::pango::AttrFontDesc::new(&description));
            label.set_attributes(Some(&attrs));
        });
        let view = gtk::ListView::new(Some(selected.clone()), Some(factory));
        view.set_single_click_activate(true);
        view.add_css_class("font-choices");
        let popover = gtk::Popover::builder().has_arrow(false).build();
        popover.add_css_class("font-popover");
        popover.add_css_class("choice-popover");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some(&crate::i18n::tr("Search fonts")));
        search.set_search_delay(60);
        search.add_css_class("form-control");
        let scroll = gtk::ScrolledWindow::builder()
            .child(&view)
            .min_content_height(180)
            .max_content_height(320)
            .min_content_width(240)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        content.append(&search);
        content.append(&scroll);
        popover.set_child(Some(&content));
        let weak_popup = popover.downgrade();
        let manager_for_activation = manager.clone();
        view.connect_activate(move |_, index| {
            if let Some(font) = filtered.item(index).and_downcast::<gtk::StringObject>() {
                manager_for_activation.set_interface_font_family(font.string().to_string());
                if let Some(popover) = weak_popup.upgrade() {
                    popover.popdown();
                }
            }
        });
        search.connect_search_changed(move |search| {
            filter.set_search(Some(&search.text()));
        });
        let manager_for_map = manager.clone();
        popover.connect_map(move |_| {
            let family = manager_for_map.interface_font_family();
            if let Some(model) = selected.model() {
                for index in 0..model.n_items() {
                    if model
                        .item(index)
                        .and_downcast::<gtk::StringObject>()
                        .is_some_and(|font| font.string() == family)
                    {
                        selected.set_selected(index);
                        break;
                    }
                }
            }
            search.grab_focus();
        });
        crate::ui::scrolling::popover::dismiss_on_outside_scroll(&popover);
        button.set_popover(Some(&popover));
    });
    button
}
