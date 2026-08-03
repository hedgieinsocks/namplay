mod menu;
mod settings;
mod tuner;

pub use menu::setup_primary_menu;
pub use settings::{setup_audio_window, setup_buffer_size_dropdown};
pub use tuner::create_tuner_window;

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gio::prelude::*;
use glib::markup_escape_text;
use gtk4::prelude::*;
use libadwaita::{self as adw, prelude::*};
use log::{debug, error};

use crate::audio::EqPosition;
use crate::keys::*;
use crate::preset::Preset;

pub fn show_persistent_toast(toast_overlay: &adw::ToastOverlay, msg: &str) {
    let toast = adw::Toast::new(msg);
    toast.set_timeout(0);
    toast_overlay.add_toast(toast);
}

pub fn restore_window_state(win: &adw::ApplicationWindow, settings: &gio::Settings) {
    win.set_default_size(settings.int(WINDOW_WIDTH), settings.int(WINDOW_HEIGHT));
    if settings.boolean(WINDOW_MAXIMIZED) {
        win.maximize();
    }
}

pub fn save_window_state(win: &adw::ApplicationWindow, settings: &gio::Settings) {
    let _ = settings.set_boolean(WINDOW_MAXIMIZED, win.is_maximized());
    if !win.is_maximized() {
        let (width, height) = win.default_size();
        let _ = settings.set_int(WINDOW_WIDTH, width);
        let _ = settings.set_int(WINDOW_HEIGHT, height);
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

pub struct FilePickerSpec {
    pub prefix: &'static str,
    pub key: &'static str,
    pub title: &'static str,
    pub filter_name: &'static str,
    pub filter_suffix: &'static str,
}

fn list_sibling_files(path: &str, suffix: &str) -> Vec<PathBuf> {
    let dir = match Path::new(path).parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case(suffix))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

fn sibling_path(current: &str, suffix: &str, offset: isize) -> Option<String> {
    let files = list_sibling_files(current, suffix);
    let index = files.iter().position(|p| p == Path::new(current))?;
    let new_index = index as isize + offset;
    if new_index < 0 || new_index as usize >= files.len() {
        return None;
    }
    files[new_index as usize].to_str().map(String::from)
}

fn update_nav_buttons(prev: &gtk4::Button, next: &gtk4::Button, path: &str, suffix: &str) {
    if path.is_empty() {
        prev.set_sensitive(false);
        next.set_sensitive(false);
        return;
    }
    let files = list_sibling_files(path, suffix);
    match files.iter().position(|p| p == Path::new(path)) {
        Some(index) => {
            prev.set_sensitive(index > 0);
            next.set_sensitive(index + 1 < files.len());
        }
        None => {
            debug!(target: "nav", "state=not_found path={path} siblings={}", files.len());
            prev.set_sensitive(false);
            next.set_sensitive(false);
        }
    }
}

pub fn setup_file_picker_row(
    builder: &gtk4::Builder,
    win: &adw::ApplicationWindow,
    settings: &gio::Settings,
    spec: &FilePickerSpec,
) {
    let row_id = format!("{}_row", spec.prefix);
    let row: adw::ExpanderRow = builder.object(&row_id).expect(&row_id);
    let button_id = format!("{}_button", spec.prefix);
    let button: gtk4::Button = builder.object(&button_id).expect(&button_id);
    let clear_button_id = format!("{}_clear_button", spec.prefix);
    let clear_button: gtk4::Button = builder.object(&clear_button_id).expect(&clear_button_id);
    let prev_button_id = format!("{}_prev_button", spec.prefix);
    let prev_button: gtk4::Button = builder.object(&prev_button_id).expect(&prev_button_id);
    let next_button_id = format!("{}_next_button", spec.prefix);
    let next_button: gtk4::Button = builder.object(&next_button_id).expect(&next_button_id);

    let current_path = settings.string(spec.key);
    update_file_row(&row, current_path.as_str());
    update_nav_buttons(
        &prev_button,
        &next_button,
        current_path.as_str(),
        spec.filter_suffix,
    );

    let filter = gtk4::FileFilter::new();
    filter.set_name(Some(spec.filter_name));
    filter.add_suffix(spec.filter_suffix);

    let filters = gio::ListStore::new::<gtk4::FileFilter>();
    filters.append(&filter);

    let key = spec.key;
    let title = spec.title;

    button.connect_clicked({
        let settings = settings.clone();
        let win = win.clone();
        move |_| {
            let dialog = gtk4::FileDialog::new();
            dialog.set_title(title);
            dialog.set_filters(Some(&filters));
            dialog.set_default_filter(Some(&filter));

            let settings = settings.clone();
            dialog.open(Some(&win), None::<&gio::Cancellable>, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        let _ = settings.set_string(key, path.to_str().unwrap_or(""));
                    }
                }
            });
        }
    });

    clear_button.connect_clicked({
        let settings = settings.clone();
        move |_| {
            settings.reset(key);
        }
    });

    let suffix = spec.filter_suffix;
    prev_button.connect_clicked({
        let settings = settings.clone();
        move |_| {
            if let Some(current) = path_from_settings(&settings, key) {
                if let Some(new_path) = sibling_path(&current, suffix, -1) {
                    let _ = settings.set_string(key, &new_path);
                }
            }
        }
    });

    next_button.connect_clicked({
        let settings = settings.clone();
        move |_| {
            if let Some(current) = path_from_settings(&settings, key) {
                if let Some(new_path) = sibling_path(&current, suffix, 1) {
                    let _ = settings.set_string(key, &new_path);
                }
            }
        }
    });

    settings.connect_changed(Some(spec.key), move |s, key| {
        let current_path = s.string(key);
        update_file_row(&row, current_path.as_str());
        update_nav_buttons(&prev_button, &next_button, current_path.as_str(), suffix);
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
        row.set_subtitle(&markup_escape_text(name));
        row.set_enable_expansion(true);
        row.set_expanded(true);
    }
}

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

fn run_blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    on_done: impl FnOnce(T) + 'static,
) {
    let (tx, rx) = futures_channel::oneshot::channel();
    std::thread::Builder::new()
        .name("preset-io".into())
        .spawn(move || {
            let _ = tx.send(work());
        })
        .expect("preset-io thread spawn failed");
    glib::MainContext::default().spawn_local(async move {
        if let Ok(result) = rx.await {
            on_done(result);
        }
    });
}

pub fn setup_preset_actions(
    builder: &gtk4::Builder,
    win: &adw::ApplicationWindow,
    settings: &gio::Settings,
    app: &adw::Application,
    pedal_skip_normalize: Rc<Cell<bool>>,
    amp_skip_normalize: Rc<Cell<bool>>,
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
                    error!(target: "preset", "state=error reason={e}");
                    show_persistent_toast(&toast_overlay_save, "Preset: failed to serialize data");
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
                        debug!(target: "preset", "state=saving file={}", path.display());
                        let yaml = yaml.clone();
                        let toast_overlay = toast_overlay.clone();
                        let path_for_log = path.clone();
                        run_blocking(
                            move || std::fs::write(&path, yaml.as_bytes()),
                            move |result| match result {
                                Ok(()) => {
                                    debug!(target: "preset", "state=saved file={}", path_for_log.display())
                                }
                                Err(e) => {
                                    error!(
                                        target: "preset",
                                        "state=error file={} reason={e}",
                                        path_for_log.display()
                                    );
                                    show_persistent_toast(
                                        &toast_overlay,
                                        "Preset: failed to save file",
                                    );
                                }
                            },
                        );
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
            let pedal_skip_normalize = Rc::clone(&pedal_skip_normalize);
            let amp_skip_normalize = Rc::clone(&amp_skip_normalize);

            dialog.open(Some(&win), None::<&gio::Cancellable>, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        debug!(target: "preset", "state=loading file={}", path.display());
                        let path_for_log = path.clone();
                        run_blocking(
                            move || std::fs::read_to_string(&path),
                            move |result| {
                                let content = match result {
                                    Ok(c) => c,
                                    Err(e) => {
                                        error!(
                                            target: "preset",
                                            "state=error file={} reason={e}",
                                            path_for_log.display()
                                        );
                                        show_persistent_toast(
                                            &toast_overlay,
                                            "Preset: failed to load file",
                                        );
                                        return;
                                    }
                                };
                                let preset = match serde_yaml::from_str::<Preset>(&content) {
                                    Ok(p) => p,
                                    Err(e) => {
                                        error!(
                                            target: "preset",
                                            "state=invalid file={} reason={e}",
                                            path_for_log.display()
                                        );
                                        show_persistent_toast(
                                            &toast_overlay,
                                            "Preset: invalid format",
                                        );
                                        return;
                                    }
                                };
                                debug!(target: "preset", "state=loaded file={}", path_for_log.display());
                                // A preset carries its own explicit output gain; don't let
                                // auto-normalize clobber it once the profile finishes loading.
                                pedal_skip_normalize.set(true);
                                amp_skip_normalize.set(true);
                                preset.apply(&settings);
                            },
                        );
                    }
                }
            });
        })
        .build();

    app.add_action_entries([save_action, load_action]);
}
