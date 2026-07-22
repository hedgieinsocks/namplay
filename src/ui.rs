//! Widget setup helpers: window state, file picker rows, slider and toggle
//! bindings, EQ position dropdown, preset save/load actions.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use gio::prelude::*;
use gtk4::prelude::*;
use libadwaita::{self as adw, prelude::*};
use log::{debug, error};

use crate::audio::{AudioEngine, EqPosition};
use crate::preset::Preset;

const BUFFER_SIZES: &[u32] = &[32, 64, 128, 256, 512, 1024];

pub fn show_persistent_toast(toast_overlay: &adw::ToastOverlay, msg: &str) {
    let toast = adw::Toast::new(msg);
    toast.set_timeout(0);
    toast_overlay.add_toast(toast);
}

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

pub fn setup_buffer_size_dropdown(builder: &gtk4::Builder, settings: &gio::Settings) {
    let dropdown: gtk4::DropDown = builder
        .object("buffer_size_dropdown")
        .expect("buffer_size_dropdown");

    let index_for = |frames: i32| {
        BUFFER_SIZES
            .iter()
            .position(|&n| n as i32 == frames)
            .unwrap_or_else(|| BUFFER_SIZES.iter().position(|&n| n == 256).unwrap_or(0))
            as u32
    };

    dropdown.set_selected(index_for(settings.int("buffer-size")));

    let settings_c = settings.clone();
    dropdown.connect_selected_notify(move |dd| {
        if let Some(&frames) = BUFFER_SIZES.get(dd.selected() as usize) {
            let _ = settings_c.set_int("buffer-size", frames as i32);
        }
    });

    settings.connect_changed(Some("buffer-size"), move |s, key| {
        dropdown.set_selected(index_for(s.int(key)));
    });
}

pub fn setup_device_rows(
    builder: &gtk4::Builder,
    settings: &gio::Settings,
    engine: Rc<AudioEngine>,
) {
    setup_device_dropdown(
        builder,
        settings,
        "input_device_dropdown",
        "input_device_refresh_button",
        "input-device",
        Rc::clone(&engine),
        AudioEngine::input_devices,
    );
    setup_device_dropdown(
        builder,
        settings,
        "output_device_dropdown",
        "output_device_refresh_button",
        "output-device",
        engine,
        AudioEngine::output_devices,
    );
}

fn setup_device_dropdown(
    builder: &gtk4::Builder,
    settings: &gio::Settings,
    dropdown_id: &str,
    refresh_button_id: &str,
    key: &'static str,
    engine: Rc<AudioEngine>,
    list_devices: fn(&AudioEngine) -> Vec<String>,
) {
    let dropdown: gtk4::DropDown = builder.object(dropdown_id).expect(dropdown_id);
    let refresh_button: gtk4::Button = builder.object(refresh_button_id).expect(refresh_button_id);

    // Index 0 is always the NONE_LABEL sentinel, so device list positions are offset by 1.
    const NONE_LABEL: &str = "None";
    let known: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

    let rebuild: Rc<dyn Fn(Vec<String>)> = Rc::new({
        let dropdown = dropdown.clone();
        let known = Rc::clone(&known);
        let settings = settings.clone();
        move |devices: Vec<String>| {
            *known.borrow_mut() = devices.clone();

            let current = settings.string(key).to_string();
            let model = gtk4::StringList::new(&[NONE_LABEL]);
            for device in &devices {
                model.append(device);
            }
            dropdown.set_model(Some(&model));

            let selected = if current.is_empty() {
                0
            } else {
                devices
                    .iter()
                    .position(|d| d == &current)
                    .map(|i| i as u32 + 1)
                    .unwrap_or(0)
            };
            dropdown.set_selected(selected);
        }
    });

    rebuild(list_devices(&engine));

    let settings_c = settings.clone();
    dropdown.connect_selected_notify(move |dd| {
        let name = if dd.selected() == 0 {
            String::new()
        } else {
            dd.selected_item()
                .and_downcast::<gtk4::StringObject>()
                .map(|s| s.string().to_string())
                .unwrap_or_default()
        };
        let _ = settings_c.set_string(key, &name);
    });

    let dropdown_c = dropdown.clone();
    let known_c = Rc::clone(&known);
    settings.connect_changed(Some(key), move |s, k| {
        let current = s.string(k).to_string();
        let devices = known_c.borrow();
        let selected = if current.is_empty() {
            0
        } else {
            devices
                .iter()
                .position(|d| d == &current)
                .map(|i| i as u32 + 1)
                .unwrap_or(0)
        };
        dropdown_c.set_selected(selected);
    });

    refresh_button.connect_clicked(move |_| {
        rebuild(list_devices(&engine));
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
                    let detail = format!("failed to serialize data: {e}");
                    error!(target: "preset", "{detail}");
                    show_persistent_toast(&toast_overlay_save, &format!("Preset: {detail}"));
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
                        debug!(target: "preset", "saving file: {}", path.display());
                        if let Err(e) = std::fs::write(&path, yaml.as_bytes()) {
                            let detail = format!("failed to save file: {e}");
                            error!(target: "preset", "{detail}");
                            show_persistent_toast(&toast_overlay, &format!("Preset: {detail}"));
                        } else {
                            debug!(target: "preset", "file saved: {}", path.display());
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
                        debug!(target: "preset", "loading file: {}", path.display());
                        let content = match std::fs::read_to_string(&path) {
                            Ok(c) => c,
                            Err(e) => {
                                let detail = format!("failed to load file: {e}");
                                error!(target: "preset", "{detail}");
                                show_persistent_toast(&toast_overlay, &format!("Preset: {detail}"));
                                return;
                            }
                        };
                        let preset = match serde_yaml::from_str::<Preset>(&content) {
                            Ok(p) => p,
                            Err(e) => {
                                let detail = format!("invalid format: {e}");
                                error!(target: "preset", "{detail}");
                                show_persistent_toast(&toast_overlay, &format!("Preset: {detail}"));
                                return;
                            }
                        };
                        debug!(target: "preset", "file loaded: {}", path.display());
                        preset.apply(&settings);
                    }
                }
            });
        })
        .build();

    app.add_action_entries([save_action, load_action]);
}

pub fn create_tuner_window(
    builder: &gtk4::Builder,
    mut tuner_hz_rx: futures_channel::mpsc::UnboundedReceiver<f32>,
) -> adw::Window {
    let window: adw::Window = builder.object("tuner_window").expect("tuner_window");
    let note_label: gtk4::Label = builder
        .object("tuner_note_label")
        .expect("tuner_note_label");
    let cents_label: gtk4::Label = builder
        .object("tuner_cents_label")
        .expect("tuner_cents_label");
    let hz_label: gtk4::Label = builder.object("tuner_hz_label").expect("tuner_hz_label");

    window.connect_hide({
        let note_label = note_label.clone();
        let cents_label = cents_label.clone();
        let hz_label = hz_label.clone();
        move |_| {
            note_label.set_text("--");
            cents_label.set_text("");
            hz_label.set_text("");
            note_label.remove_css_class("success");
        }
    });

    glib::MainContext::default().spawn_local(async move {
        use futures_util::StreamExt;
        while let Some(hz) = tuner_hz_rx.next().await {
            if let Some((name, cents)) = hz_to_note(hz) {
                note_label.set_text(&name);
                cents_label.set_text(&format!("{:+.0} cents", cents));
                hz_label.set_text(&format!("{hz:.1} Hz"));
                if cents.abs() <= 5.0 {
                    note_label.add_css_class("success");
                } else {
                    note_label.remove_css_class("success");
                }
            } else {
                note_label.set_text("--");
                cents_label.set_text("");
                hz_label.set_text("");
                note_label.remove_css_class("success");
            }
        }
    });

    window
}

fn hz_to_note(hz: f32) -> Option<(String, f32)> {
    if !(20.0..=8000.0).contains(&hz) {
        return None;
    }
    let midi_float = 69.0 + 12.0 * (hz / 440.0).log2();
    let midi_round = midi_float.round();
    let cents = (midi_float - midi_round) * 100.0;
    let midi_int = midi_round as i32;
    if !(21..=108).contains(&midi_int) {
        return None;
    }
    const NAMES: &[&str] = &[
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = (midi_int / 12) - 1;
    let name = format!("{}{}", NAMES[(midi_int % 12) as usize], octave);
    Some((name, cents))
}
