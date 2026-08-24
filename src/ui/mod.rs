mod bindings;
mod file;
mod menu;
mod preset;
mod settings;
mod tuner;
mod window;

pub use bindings::{bind_adjustment, bind_toggle, setup_eq_position, setup_reset_button};
pub use file::{setup_file_picker_row, FilePickerSpec, CAB_FILTER_SUFFIX, NAM_FILTER_SUFFIX};
pub use menu::setup_primary_menu;
pub use preset::{setup_preset_actions, setup_preset_row};
pub use settings::{setup_buffer_size_dropdown, setup_settings_window};
pub use tuner::setup_tuner_window;
pub use window::{restore_window_state, save_window_state};

use gio::prelude::*;
use libadwaita as adw;

pub fn show_persistent_toast(toast_overlay: &adw::ToastOverlay, msg: &str) {
    let toast = adw::Toast::new(msg);
    toast.set_timeout(0);
    toast_overlay.add_toast(toast);
}

pub fn path_from_settings(settings: &gio::Settings, key: &str) -> Option<String> {
    let p = settings.string(key);
    if p.is_empty() {
        None
    } else {
        Some(p.to_string())
    }
}
