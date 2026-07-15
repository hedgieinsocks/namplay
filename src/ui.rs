use std::path::Path;

use gio::prelude::*;
use gtk4::prelude::*;
use libadwaita::{self as adw, prelude::*};
use log::{debug, error};

use crate::profile::{apply_preset_to_settings, build_preset_from_settings, Profile};

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

pub fn setup_file_picker_row(
    builder: &gtk4::Builder,
    win: &adw::ApplicationWindow,
    settings: &gio::Settings,
    row_id: &str,
    button_id: &str,
    clear_id: &str,
    path_key: &str,
    title: &str,
    filter_name: &str,
    filter_suffix: &str,
) {
    let row: adw::ExpanderRow = builder.object(row_id).expect(row_id);
    let button: gtk4::Button = builder.object(button_id).expect(button_id);
    let clear_button: gtk4::Button = builder.object(clear_id).expect(clear_id);

    let stored = settings.string(path_key);
    if !stored.is_empty() {
        let name = Path::new(stored.as_str())
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(stored.as_str());
        row.set_subtitle(name);
        row.set_enable_expansion(true);
        row.set_expanded(true);
    }

    let filter = gtk4::FileFilter::new();
    filter.set_name(Some(filter_name));
    filter.add_suffix(filter_suffix);

    let filters = gio::ListStore::new::<gtk4::FileFilter>();
    filters.append(&filter);

    let settings_c = settings.clone();
    let win_c = win.clone();
    let path_key_c = path_key.to_owned();
    let title_c = title.to_owned();
    let filter_c = filter.clone();
    let filters_c = filters.clone();

    button.connect_clicked(move |_| {
        let dialog = gtk4::FileDialog::new();
        dialog.set_title(&title_c);
        dialog.set_filters(Some(&filters_c));
        dialog.set_default_filter(Some(&filter_c));

        let settings = settings_c.clone();
        let path_key = path_key_c.clone();

        dialog.open(Some(&win_c), None::<&gio::Cancellable>, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    let _ = settings.set_string(&path_key, path.to_str().unwrap_or(""));
                }
            }
        });
    });

    let path_key_c = path_key.to_owned();
    let settings_c = settings.clone();

    clear_button.connect_clicked(move |_| {
        settings_c.reset(&path_key_c);
    });

    let row_c = row.clone();
    settings.connect_changed(Some(path_key), move |s, key| {
        update_file_row(&row_c, s.string(key).as_str());
    });
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
    let pre_pedal_btn: gtk4::ToggleButton = builder
        .object("eq_pre_pedal_button")
        .expect("eq_pre_pedal_button");
    let pre_amp_btn: gtk4::ToggleButton = builder
        .object("eq_pre_amp_button")
        .expect("eq_pre_amp_button");
    let post_ir_btn: gtk4::ToggleButton = builder
        .object("eq_post_ir_button")
        .expect("eq_post_ir_button");

    match settings.string("eq-position").as_str() {
        "pre-pedal" => pre_pedal_btn.set_active(true),
        "post-ir" => post_ir_btn.set_active(true),
        _ => pre_amp_btn.set_active(true),
    }

    let settings_c = settings.clone();
    pre_pedal_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            let _ = settings_c.set_string("eq-position", "pre-pedal");
        }
    });

    let settings_c = settings.clone();
    pre_amp_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            let _ = settings_c.set_string("eq-position", "pre-amp");
        }
    });

    let settings_c = settings.clone();
    post_ir_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            let _ = settings_c.set_string("eq-position", "post-ir");
        }
    });

    let pre_pedal_btn_c = pre_pedal_btn.clone();
    let pre_amp_btn_c = pre_amp_btn.clone();
    let post_ir_btn_c = post_ir_btn.clone();
    settings.connect_changed(Some("eq-position"), move |s, key| {
        match s.string(key).as_str() {
            "pre-pedal" => pre_pedal_btn_c.set_active(true),
            "post-ir" => post_ir_btn_c.set_active(true),
            _ => pre_amp_btn_c.set_active(true),
        }
    });
}

pub fn update_file_row(row: &adw::ExpanderRow, path: &str) {
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
            let profile = build_preset_from_settings(&settings_save);
            let yaml = match serde_yaml::to_string(&profile) {
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
                        let profile = match serde_yaml::from_str::<Profile>(&content) {
                            Ok(p) => p,
                            Err(e) => {
                                error!("invalid preset: {e}");
                                toast_overlay
                                    .add_toast(adw::Toast::new(&format!("Failed to load preset")));
                                return;
                            }
                        };
                        debug!("preset loaded: {}", path.display());
                        apply_preset_to_settings(&profile, &settings);
                    }
                }
            });
        })
        .build();

    app.add_action_entries([save_action, load_action]);
}
