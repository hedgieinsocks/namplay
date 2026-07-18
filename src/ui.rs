//! Widget setup helpers: window state, file picker rows, slider and toggle
//! bindings, EQ position dropdown, preset save/load actions.

use std::path::Path;

use gio::prelude::*;
use gtk4::prelude::*;
use libadwaita::{self as adw, prelude::*};
use log::{debug, error};

use crate::audio::EqPosition;
use crate::preset::Preset;

pub fn restore_window_state(win: &adw::ApplicationWindow, settings: &gio::Settings) {
    win.set_default_size(settings.int("window-width"), settings.int("window-height"));
    if settings.boolean("window-maximized") {
        win.maximize();
    }
}

pub fn save_window_state(win: &adw::ApplicationWindow, settings: &gio::Settings) {
    let _ = settings.set_boolean("window-maximized", win.is_maximized());
    if !win.is_maximized() {
        let (width, height) = win.default_size();
        let _ = settings.set_int("window-width", width);
        let _ = settings.set_int("window-height", height);
    }
}

pub fn path_from_settings(settings: &gio::Settings, key: &str) -> Option<String> {
    let p = settings.string(key);
    if p.is_empty() {
        None
    } else {
        Some(p.to_string())
    }
}

/// A file-backed ExpanderRow whose widget ids follow the
/// `{prefix}_row` / `{prefix}_button` / `{prefix}_clear_button` convention.
pub struct FilePickerSpec {
    pub prefix: &'static str,
    pub key: &'static str,
    pub title: &'static str,
    pub filter_name: &'static str,
    pub filter_suffix: &'static str,
}

pub fn setup_file_picker_row(
    builder: &gtk4::Builder,
    win: &adw::ApplicationWindow,
    settings: &gio::Settings,
    spec: &FilePickerSpec,
) {
    let row: adw::ExpanderRow = builder
        .object(format!("{}_row", spec.prefix))
        .expect(spec.prefix);
    let button: gtk4::Button = builder
        .object(format!("{}_button", spec.prefix))
        .expect(spec.prefix);
    let clear_button: gtk4::Button = builder
        .object(format!("{}_clear_button", spec.prefix))
        .expect(spec.prefix);

    update_file_row(&row, settings.string(spec.key).as_str());

    let filter = gtk4::FileFilter::new();
    filter.set_name(Some(spec.filter_name));
    filter.add_suffix(spec.filter_suffix);

    let filters = gio::ListStore::new::<gtk4::FileFilter>();
    filters.append(&filter);

    let settings_c = settings.clone();
    let win_c = win.clone();
    let key = spec.key;
    let title = spec.title;

    button.connect_clicked(move |_| {
        let dialog = gtk4::FileDialog::new();
        dialog.set_title(title);
        dialog.set_filters(Some(&filters));
        dialog.set_default_filter(Some(&filter));

        let settings = settings_c.clone();
        dialog.open(Some(&win_c), None::<&gio::Cancellable>, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    let _ = settings.set_string(key, path.to_str().unwrap_or(""));
                }
            }
        });
    });

    let settings_c = settings.clone();
    clear_button.connect_clicked(move |_| {
        settings_c.reset(key);
    });

    settings.connect_changed(Some(spec.key), move |s, key| {
        update_file_row(&row, s.string(key).as_str());
    });
}

fn update_file_row(row: &adw::ExpanderRow, path: &str) {
    if path.is_empty() {
        row.set_subtitle("No file selected");
        row.set_enable_expansion(false);
    } else {
        let name = Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path);
        row.set_subtitle(name);
        row.set_enable_expansion(true);
        row.set_expanded(true);
    }
}

pub fn bind_adjustment(builder: &gtk4::Builder, settings: &gio::Settings, id: &str, key: &str) {
    let adj: gtk4::Adjustment = builder.object(id).expect(id);
    settings.bind(key, &adj, "value").build();
}

pub fn bind_toggle(builder: &gtk4::Builder, settings: &gio::Settings, id: &str, key: &str) {
    let row: adw::ExpanderRow = builder.object(id).expect(id);
    settings.bind(key, &row, "enable-expansion").build();
}

pub fn setup_reset_button(builder: &gtk4::Builder, settings: &gio::Settings, id: &str, key: &str) {
    let btn: gtk4::Button = builder.object(id).expect(id);
    let settings = settings.clone();
    let key = key.to_owned();
    btn.connect_clicked(move |_| {
        settings.reset(&key);
    });
}

pub fn setup_eq_position(builder: &gtk4::Builder, settings: &gio::Settings) {
    let dropdown: gtk4::DropDown = builder
        .object("eq_position_dropdown")
        .expect("eq_position_dropdown");

    dropdown
        .set_selected(EqPosition::from_setting(settings.string("eq-position").as_str()).index());

    let settings_c = settings.clone();
    dropdown.connect_selected_notify(move |dd| {
        let _ = settings_c.set_string(
            "eq-position",
            EqPosition::from_index(dd.selected()).setting(),
        );
    });

    settings.connect_changed(Some("eq-position"), move |s, key| {
        dropdown.set_selected(EqPosition::from_setting(s.string(key).as_str()).index());
    });
}

pub fn setup_preset_actions(
    builder: &gtk4::Builder,
    win: &adw::ApplicationWindow,
    settings: &gio::Settings,
    app: &adw::Application,
) {
    let toast_overlay: adw::ToastOverlay = builder.object("toast_overlay").expect("toast_overlay");

    let settings_save = settings.clone();
    let win_save = win.clone();
    let toast_overlay_save = toast_overlay.clone();
    let save_action = gio::ActionEntry::builder("save-preset")
        .activate(move |_: &adw::Application, _, _| {
            let preset = Preset::from_settings(&settings_save);
            let yaml = match serde_yaml::to_string(&preset) {
                Ok(y) => y,
                Err(e) => {
                    error!("failed to serialize preset: {e}");
                    toast_overlay_save.add_toast(adw::Toast::new("Failed to serialize preset"));
                    return;
                }
            };

            let dialog = gtk4::FileDialog::new();
            dialog.set_title("Save Preset");
            dialog.set_initial_name(Some("new_preset.yaml"));

            let win = win_save.clone();
            let toast_overlay = toast_overlay_save.clone();
            dialog.save(Some(&win), None::<&gio::Cancellable>, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        if let Err(e) = std::fs::write(&path, yaml.as_bytes()) {
                            error!("failed to write preset: {e}");
                            toast_overlay.add_toast(adw::Toast::new("Failed to save preset"));
                        } else {
                            debug!("preset saved: {}", path.display());
                        }
                    }
                }
            });
        })
        .build();

    let settings_load = settings.clone();
    let win_load = win.clone();
    let toast_overlay_load = toast_overlay.clone();
    let load_action = gio::ActionEntry::builder("load-preset")
        .activate(move |_: &adw::Application, _, _| {
            let filter = gtk4::FileFilter::new();
            filter.set_name(Some("Namplay YAML Presets"));
            filter.add_suffix("yaml");

            let filters = gio::ListStore::new::<gtk4::FileFilter>();
            filters.append(&filter);

            let dialog = gtk4::FileDialog::new();
            dialog.set_title("Load Preset");
            dialog.set_filters(Some(&filters));
            dialog.set_default_filter(Some(&filter));

            let settings = settings_load.clone();
            let win = win_load.clone();
            let toast_overlay = toast_overlay_load.clone();

            dialog.open(Some(&win), None::<&gio::Cancellable>, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        let content = match std::fs::read_to_string(&path) {
                            Ok(c) => c,
                            Err(e) => {
                                error!("failed to read preset: {e}");
                                toast_overlay.add_toast(adw::Toast::new("Failed to read preset"));
                                return;
                            }
                        };
                        let preset = match serde_yaml::from_str::<Preset>(&content) {
                            Ok(p) => p,
                            Err(e) => {
                                error!("invalid preset: {e}");
                                toast_overlay.add_toast(adw::Toast::new("Failed to load preset"));
                                return;
                            }
                        };
                        debug!("preset loaded: {}", path.display());
                        preset.apply(&settings);
                    }
                }
            });
        })
        .build();

    app.add_action_entries([save_action, load_action]);
}
