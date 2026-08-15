use gio::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;

use crate::keys::{WINDOW_HEIGHT, WINDOW_MAXIMIZED, WINDOW_WIDTH};

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
