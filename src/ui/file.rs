use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gio::prelude::*;
use glib::markup_escape_text;
use gtk4::prelude::*;
use libadwaita::{self as adw, prelude::*};
use log::debug;

use super::path_from_settings;

pub const NAM_FILTER_SUFFIX: &str = "nam";
pub const CAB_FILTER_SUFFIX: &str = "wav";
pub(crate) const PRESET_FILTER_SUFFIX: &str = "yaml";

pub struct FilePickerSpec {
    pub prefix: &'static str,
    pub key: &'static str,
    pub title: &'static str,
    pub filter_name: &'static str,
    pub filter_suffix: &'static str,
}

pub(crate) fn list_sibling_files(path: &str, suffix: &str) -> Vec<PathBuf> {
    let dir = match Path::new(path).parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
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

pub(crate) fn sibling_path(current: &str, suffix: &str, offset: isize) -> Option<String> {
    let files = list_sibling_files(current, suffix);
    if files.is_empty() {
        return None;
    }
    let index = files.iter().position(|p| p == Path::new(current))?;
    let len = files.len() as isize;
    let new_index = (index as isize + offset).rem_euclid(len);
    files[new_index as usize].to_str().map(String::from)
}

pub(crate) fn update_nav_buttons(
    prev: &gtk4::Button,
    next: &gtk4::Button,
    path: &str,
    suffix: &str,
) {
    if path.is_empty() {
        prev.set_sensitive(false);
        next.set_sensitive(false);
        return;
    }
    let files = list_sibling_files(path, suffix);
    if files.iter().any(|p| p == Path::new(path)) {
        let has_siblings = files.len() > 1;
        prev.set_sensitive(has_siblings);
        next.set_sensitive(has_siblings);
    } else {
        debug!(target: "nav", "state=not_found path={path} siblings={}", files.len());
        prev.set_sensitive(false);
        next.set_sensitive(false);
    }
}

pub fn setup_file_picker_row(
    builder: &gtk4::Builder,
    win: &adw::ApplicationWindow,
    settings: &gio::Settings,
    app: &adw::Application,
    spec: &FilePickerSpec,
    skip_normalize: Option<&Rc<Cell<bool>>>,
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
        let skip_normalize = skip_normalize.cloned();
        move |_| {
            let dialog = gtk4::FileDialog::new();
            dialog.set_title(title);
            dialog.set_filters(Some(&filters));
            dialog.set_default_filter(Some(&filter));

            let settings = settings.clone();
            let skip_normalize = skip_normalize.clone();
            dialog.open(Some(&win), None::<&gio::Cancellable>, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        if let Some(flag) = &skip_normalize {
                            flag.set(false);
                        }
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
    let connect_nav = |button: &gtk4::Button, offset: isize| {
        button.connect_clicked({
            let settings = settings.clone();
            let skip_normalize = skip_normalize.cloned();
            move |_| {
                if let Some(current) = path_from_settings(&settings, key) {
                    if let Some(new_path) = sibling_path(&current, suffix, offset) {
                        if let Some(flag) = &skip_normalize {
                            flag.set(false);
                        }
                        let _ = settings.set_string(key, &new_path);
                    }
                }
            }
        });
    };
    connect_nav(&prev_button, -1);
    connect_nav(&next_button, 1);

    let prev_action = gio::ActionEntry::builder(&format!("{}-prev", spec.prefix))
        .activate({
            let prev_button = prev_button.clone();
            move |_: &adw::Application, _, _| prev_button.emit_clicked()
        })
        .build();
    let next_action = gio::ActionEntry::builder(&format!("{}-next", spec.prefix))
        .activate({
            let next_button = next_button.clone();
            move |_: &adw::Application, _, _| next_button.emit_clicked()
        })
        .build();
    app.add_action_entries([prev_action, next_action]);

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
