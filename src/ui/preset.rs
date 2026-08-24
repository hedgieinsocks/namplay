use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gio::prelude::*;
use glib::markup_escape_text;
use gtk4::prelude::*;
use libadwaita::{self as adw, prelude::*};
use log::{debug, error};

use super::file::{sibling_path, update_nav_buttons, PRESET_FILTER_SUFFIX};
use super::{path_from_settings, show_persistent_toast};
use crate::keys::PRESET_PATH;
use crate::preset::Preset;

fn spawn_background_task<T: Send + 'static>(
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

fn load_preset_from_path(
    path: PathBuf,
    settings: gio::Settings,
    toast_overlay: adw::ToastOverlay,
    pedal_skip_normalize: Rc<Cell<bool>>,
    amp_skip_normalize: Rc<Cell<bool>>,
) {
    debug!(target: "preset", "state=loading file={}", path.display());
    let path_for_log = path.clone();
    let path_for_setting = path.clone();
    spawn_background_task(
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
                    show_persistent_toast(&toast_overlay, "Preset: failed to load file");
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
                    show_persistent_toast(&toast_overlay, "Preset: invalid format");
                    return;
                }
            };
            debug!(target: "preset", "state=loaded file={}", path_for_log.display());
            pedal_skip_normalize.set(true);
            amp_skip_normalize.set(true);
            preset.apply(&settings);
            let _ = settings.set_string(PRESET_PATH, path_for_setting.to_str().unwrap_or(""));
        },
    );
}

fn update_preset_row(row: &adw::ActionRow, path: &str) {
    if path.is_empty() {
        row.set_subtitle("No preset loaded");
    } else {
        let name = Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path);
        row.set_subtitle(&markup_escape_text(name));
    }
}

pub fn setup_preset_row(
    builder: &gtk4::Builder,
    settings: &gio::Settings,
    app: &adw::Application,
    toast_overlay: &adw::ToastOverlay,
    pedal_skip_normalize: &Rc<Cell<bool>>,
    amp_skip_normalize: &Rc<Cell<bool>>,
) {
    let row: adw::ActionRow = builder.object("preset_row").expect("preset_row");
    let prev_button: gtk4::Button = builder
        .object("preset_prev_button")
        .expect("preset_prev_button");
    let next_button: gtk4::Button = builder
        .object("preset_next_button")
        .expect("preset_next_button");

    let current_path = settings.string(PRESET_PATH);
    update_preset_row(&row, current_path.as_str());
    update_nav_buttons(
        &prev_button,
        &next_button,
        current_path.as_str(),
        PRESET_FILTER_SUFFIX,
    );

    let connect_nav = |button: &gtk4::Button, offset: isize| {
        button.connect_clicked({
            let settings = settings.clone();
            let toast_overlay = toast_overlay.clone();
            let pedal_skip_normalize = Rc::clone(pedal_skip_normalize);
            let amp_skip_normalize = Rc::clone(amp_skip_normalize);
            move |_| {
                if let Some(current) = path_from_settings(&settings, PRESET_PATH) {
                    if let Some(new_path) = sibling_path(&current, PRESET_FILTER_SUFFIX, offset) {
                        load_preset_from_path(
                            PathBuf::from(new_path),
                            settings.clone(),
                            toast_overlay.clone(),
                            Rc::clone(&pedal_skip_normalize),
                            Rc::clone(&amp_skip_normalize),
                        );
                    }
                }
            }
        });
    };
    connect_nav(&prev_button, -1);
    connect_nav(&next_button, 1);

    let prev_action = gio::ActionEntry::builder("preset-prev")
        .activate({
            let prev_button = prev_button.clone();
            move |_: &adw::Application, _, _| prev_button.emit_clicked()
        })
        .build();
    let next_action = gio::ActionEntry::builder("preset-next")
        .activate({
            let next_button = next_button.clone();
            move |_: &adw::Application, _, _| next_button.emit_clicked()
        })
        .build();
    app.add_action_entries([prev_action, next_action]);

    settings.connect_changed(Some(PRESET_PATH), move |s, key| {
        let current_path = s.string(key);
        update_preset_row(&row, current_path.as_str());
        update_nav_buttons(
            &prev_button,
            &next_button,
            current_path.as_str(),
            PRESET_FILTER_SUFFIX,
        );
    });
}

#[allow(clippy::too_many_lines)]
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
            let settings = settings_save.clone();
            dialog.save(Some(&win), None::<&gio::Cancellable>, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        debug!(target: "preset", "state=saving file={}", path.display());
                        let yaml = yaml.clone();
                        let toast_overlay = toast_overlay.clone();
                        let settings = settings.clone();
                        let path_for_log = path.clone();
                        let path_for_setting = path.clone();
                        spawn_background_task(
                            move || std::fs::write(&path, yaml.as_bytes()),
                            move |result| match result {
                                Ok(()) => {
                                    debug!(target: "preset", "state=saved file={}", path_for_log.display());
                                    let _ = settings.set_string(
                                        PRESET_PATH,
                                        path_for_setting.to_str().unwrap_or(""),
                                    );
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
                        load_preset_from_path(
                            path,
                            settings.clone(),
                            toast_overlay.clone(),
                            Rc::clone(&pedal_skip_normalize),
                            Rc::clone(&amp_skip_normalize),
                        );
                    }
                }
            });
        })
        .build();

    app.add_action_entries([save_action, load_action]);
}
