use gio::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;

use crate::audio::EqPosition;
use crate::keys::EQ_POSITION;

pub fn bind_adjustment(
    builder: &gtk4::Builder,
    settings: &gio::Settings,
    id: &str,
    key: &'static str,
) {
    let adj: gtk4::Adjustment = builder.object(id).expect(id);
    settings.bind(key, &adj, "value").build();
}

pub fn bind_toggle(builder: &gtk4::Builder, settings: &gio::Settings, id: &str, key: &'static str) {
    let row: adw::ExpanderRow = builder.object(id).expect(id);
    settings.bind(key, &row, "enable-expansion").build();
}

pub fn setup_reset_button(
    builder: &gtk4::Builder,
    settings: &gio::Settings,
    id: &str,
    key: &'static str,
) {
    let btn: gtk4::Button = builder.object(id).expect(id);
    let settings = settings.clone();
    btn.connect_clicked(move |_| {
        settings.reset(key);
    });
}

pub fn setup_eq_position(builder: &gtk4::Builder, settings: &gio::Settings) {
    let dropdown: gtk4::DropDown = builder
        .object("eq_position_dropdown")
        .expect("eq_position_dropdown");

    dropdown.set_selected(EqPosition::from_setting(settings.string(EQ_POSITION).as_str()).index());

    dropdown.connect_selected_notify({
        let settings = settings.clone();
        move |dd| {
            let _ =
                settings.set_string(EQ_POSITION, EqPosition::from_index(dd.selected()).setting());
        }
    });

    settings.connect_changed(Some(EQ_POSITION), move |s, key| {
        dropdown.set_selected(EqPosition::from_setting(s.string(key).as_str()).index());
    });
}
