mod audio;
mod preset;
mod tray;
mod ui;
mod ui_menu;
mod ui_settings;
mod ui_tuner;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use gio::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use log::{debug, error};

use audio::{AudioEngine, EqPosition, InitialParams};
use ui::{
    bind_adjustment, bind_toggle, path_from_settings, restore_window_state, save_window_state,
    setup_eq_position, setup_file_picker_row, setup_preset_actions, setup_reset_button,
    show_persistent_toast, FilePickerSpec,
};
use ui_menu::setup_primary_menu;
use ui_settings::{setup_audio_window, setup_buffer_size_dropdown};
use ui_tuner::create_tuner_window;

pub(crate) const APP_ID: &str = "io.github.hedgieinsocks.Namplay";
const UI: &str = include_str!(concat!(env!("OUT_DIR"), "/window.ui"));
const TARGET_LOUDNESS_DBFS: f64 = -18.0;

const FILE_PICKERS: &[FilePickerSpec] = &[
    FilePickerSpec {
        prefix: "pedal",
        key: "pedal-path",
        title: "Choose Pedal Profile",
        filter_name: "NAM Profiles",
        filter_suffix: "nam",
    },
    FilePickerSpec {
        prefix: "amp",
        key: "amp-path",
        title: "Choose Amp Profile",
        filter_name: "NAM Profiles",
        filter_suffix: "nam",
    },
    FilePickerSpec {
        prefix: "cab",
        key: "cab-path",
        title: "Choose Cabinet IR",
        filter_name: "WAV Files",
        filter_suffix: "wav",
    },
];

macro_rules! slider_settings {
    ($($key:literal => $setter:ident),+ $(,)?) => {
        const SLIDER_KEYS: &[&str] = &[$($key),+];

        fn dispatch_slider_change(engine: &AudioEngine, s: &gio::Settings, key: &str) -> bool {
            match key {
                $($key => {
                    engine.$setter(s.double(key) as f32);
                    true
                })+
                _ => false,
            }
        }
    };
}

slider_settings! {
    "gate-threshold" => set_gate_threshold_db,
    "eq-hp" => set_eq_hp_freq,
    "eq-low" => set_eq_low_db,
    "eq-mid" => set_eq_mid_db,
    "eq-high" => set_eq_high_db,
    "eq-lp" => set_eq_lp_freq,
    "pedal-input" => set_pedal_in_gain_db,
    "pedal-output" => set_pedal_out_gain_db,
    "amp-input" => set_amp_in_gain_db,
    "amp-output" => set_amp_out_gain_db,
    "cab-level" => set_cab_level_db,
}

fn main() {
    env_logger::init();
    let app = adw::Application::builder().application_id(APP_ID).build();

    app.add_main_option(
        "background",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Start without showing the main window",
        None,
    );

    let start_hidden = Rc::new(Cell::new(false));

    app.connect_handle_local_options({
        let start_hidden = Rc::clone(&start_hidden);
        move |_app, options| {
            start_hidden.set(options.contains("background"));
            std::ops::ControlFlow::Continue(())
        }
    });

    app.connect_activate(move |app| build_ui(app, start_hidden.get()));

    std::process::exit(app.run().into());
}

fn build_ui(app: &adw::Application, start_hidden: bool) {
    if let Some(win) = app.active_window() {
        win.present();
        return;
    }

    let builder = gtk4::Builder::from_string(UI);
    let win: adw::ApplicationWindow = builder.object("window").expect("window not found");
    win.set_application(Some(app));

    let settings = gio::Settings::new(APP_ID);

    restore_window_state(&win, &settings);

    for spec in FILE_PICKERS {
        setup_file_picker_row(&builder, &win, &settings, spec);
    }

    bind_toggle(&builder, &settings, "gate_row", "gate-enabled");
    bind_toggle(&builder, &settings, "eq_row", "eq-enabled");

    for key in SLIDER_KEYS {
        let id_base = key.replace('-', "_");
        bind_adjustment(&builder, &settings, &format!("{id_base}_adjustment"), key);
        setup_reset_button(&builder, &settings, &format!("{id_base}_reset_button"), key);
    }

    setup_eq_position(&builder, &settings);
    setup_buffer_size_dropdown(&builder, &settings);

    let toast_overlay: adw::ToastOverlay = builder.object("toast_overlay").expect("toast_overlay");

    let pedal_profile_path = path_from_settings(&settings, "pedal-path");
    let amp_profile_path = path_from_settings(&settings, "amp-path");

    // The engine reports a load for the profile restored from settings at startup
    // too, and a preset carries its own explicit output gain; both are cases where
    // auto-normalize must not clobber an output gain that was just set on purpose.
    let pedal_skip_normalize = Rc::new(Cell::new(pedal_profile_path.is_some()));
    let amp_skip_normalize = Rc::new(Cell::new(amp_profile_path.is_some()));

    match AudioEngine::new(InitialParams {
        input_device: path_from_settings(&settings, "input-device"),
        output_device: path_from_settings(&settings, "output-device"),
        buffer_size: settings.int("buffer-size") as u32,
        gate_enabled: settings.boolean("gate-enabled"),
        gate_threshold_db: settings.double("gate-threshold") as f32,
        pedal_profile_path: pedal_profile_path.clone(),
        pedal_in_gain_db: settings.double("pedal-input") as f32,
        pedal_out_gain_db: settings.double("pedal-output") as f32,
        amp_profile_path: amp_profile_path.clone(),
        amp_in_gain_db: settings.double("amp-input") as f32,
        amp_out_gain_db: settings.double("amp-output") as f32,
        cab_path: path_from_settings(&settings, "cab-path"),
        cab_level_db: settings.double("cab-level") as f32,
        eq_enabled: settings.boolean("eq-enabled"),
        eq_pos: EqPosition::from_setting(settings.string("eq-position").as_str()),
        eq_low_db: settings.double("eq-low") as f32,
        eq_mid_db: settings.double("eq-mid") as f32,
        eq_high_db: settings.double("eq-high") as f32,
        eq_hp_freq: settings.double("eq-hp") as f32,
        eq_lp_freq: settings.double("eq-lp") as f32,
    }) {
        Ok(engine) => {
            let warning_rx = engine
                .warning_rx
                .borrow_mut()
                .take()
                .expect("warning receiver already taken");
            glib::MainContext::default().spawn_local({
                let toast_overlay = toast_overlay.clone();
                async move {
                    use futures_util::StreamExt;
                    let mut warning_rx = warning_rx;
                    while let Some(msg) = warning_rx.next().await {
                        show_persistent_toast(&toast_overlay, &msg);
                    }
                }
            });

            let engine = Rc::new(engine);
            setup_audio_window(&builder, &settings, &engine);

            wire_toggle_button(&builder, "mute_button", "mute", Arc::clone(&engine.mute));
            wire_toggle_button(
                &builder,
                "pedal_bypass_button",
                "pedal",
                Arc::clone(&engine.pedal_bypass),
            );
            wire_toggle_button(
                &builder,
                "amp_bypass_button",
                "amp",
                Arc::clone(&engine.amp_bypass),
            );
            wire_toggle_button(
                &builder,
                "cab_bypass_button",
                "cab",
                Arc::clone(&engine.cab_bypass),
            );

            setup_tray(app, &win, &builder, &settings, &engine, start_hidden);

            let tuner_hz_rx = engine
                .tuner_hz_rx
                .borrow_mut()
                .take()
                .expect("tuner receiver already taken");
            let tuner_window = create_tuner_window(&builder, tuner_hz_rx);

            let tuner_action = gio::ActionEntry::builder("tuner")
                .activate({
                    let tuner_window = tuner_window.clone();
                    move |_: &adw::Application, _, _| {
                        tuner_window.present();
                    }
                })
                .build();
            app.add_action_entries([tuner_action]);

            let tuner_enabled = Arc::clone(&engine.tuner_enabled);
            tuner_window.connect_show(move |_| {
                debug!(target: "tuner", "state=on");
                tuner_enabled.store(true, Ordering::Relaxed);
            });
            let tuner_enabled = Arc::clone(&engine.tuner_enabled);
            tuner_window.connect_hide(move |_| {
                debug!(target: "tuner", "state=off");
                tuner_enabled.store(false, Ordering::Relaxed);
            });

            wire_normalize_button(
                &builder,
                "pedal_output_normalize_button",
                Arc::clone(&engine.pedal_loudness),
                settings.clone(),
                "pedal-output",
            );
            wire_normalize_button(
                &builder,
                "amp_output_normalize_button",
                Arc::clone(&engine.amp_loudness),
                settings.clone(),
                "amp-output",
            );

            let profile_loaded_rx = engine
                .profile_loaded_rx
                .borrow_mut()
                .take()
                .expect("profile-loaded receiver already taken");
            glib::MainContext::default().spawn_local({
                let settings = settings.clone();
                let pedal_loudness = Arc::clone(&engine.pedal_loudness);
                let amp_loudness = Arc::clone(&engine.amp_loudness);
                let pedal_skip_normalize = Rc::clone(&pedal_skip_normalize);
                let amp_skip_normalize = Rc::clone(&amp_skip_normalize);
                async move {
                    use futures_util::StreamExt;
                    let mut profile_loaded_rx = profile_loaded_rx;
                    while let Some(label) = profile_loaded_rx.next().await {
                        let (loudness, key, skip_normalize) = match label {
                            "Pedal" => (&pedal_loudness, "pedal-output", &pedal_skip_normalize),
                            "Amp" => (&amp_loudness, "amp-output", &amp_skip_normalize),
                            _ => continue,
                        };
                        if skip_normalize.replace(false) {
                            continue;
                        }
                        if settings.boolean("normalize-output") {
                            apply_normalize(&settings, loudness, key);
                        }
                    }
                }
            });

            settings.connect_changed(None, move |s, key| {
                if dispatch_slider_change(&engine, s, key) {
                    return;
                }
                match key {
                    "input-device" => engine.set_input_device(path_from_settings(s, key)),
                    "output-device" => engine.set_output_device(path_from_settings(s, key)),
                    "gate-enabled" => engine.set_gate_enabled(s.boolean(key)),
                    "pedal-path" => engine.load_pedal_profile(path_from_settings(s, key)),
                    "amp-path" => engine.load_amp_profile(path_from_settings(s, key)),
                    "cab-path" => engine.load_cab(path_from_settings(s, key)),
                    "eq-enabled" => engine.set_eq_enabled(s.boolean(key)),
                    "eq-position" => {
                        engine.set_eq_pos(EqPosition::from_setting(s.string(key).as_str()))
                    }
                    _ => {}
                }
            });
        }
        Err(e) => {
            error!(target: "audio", "Audio unavailable: {e}");
            show_persistent_toast(&toast_overlay, "Audio unavailable");
        }
    }

    let settings_clone = settings.clone();
    win.connect_close_request(move |w| {
        save_window_state(w, &settings_clone);
        if settings_clone.boolean("run-in-background") {
            w.set_visible(false);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });

    setup_primary_menu(app, &builder, &settings, &toast_overlay);

    setup_preset_actions(
        &builder,
        &win,
        &settings,
        app,
        pedal_skip_normalize,
        amp_skip_normalize,
    );

    if start_hidden {
        win.set_visible(false);
    } else {
        win.present();
    }
}

fn wire_toggle_button(
    builder: &gtk4::Builder,
    id: &str,
    label: &'static str,
    flag: Arc<AtomicBool>,
) {
    let btn: gtk4::ToggleButton = builder.object(id).expect(id);
    btn.connect_toggled(move |btn| {
        let active = btn.is_active();
        debug!(target: label, "{}", if active { "on" } else { "off" });
        btn.set_icon_name(if active {
            "audio-volume-muted-symbolic"
        } else {
            "audio-volume-high-symbolic"
        });
        flag.store(active, Ordering::Relaxed);
    });
}

fn setup_tray(
    app: &adw::Application,
    win: &adw::ApplicationWindow,
    builder: &gtk4::Builder,
    settings: &gio::Settings,
    engine: &Rc<AudioEngine>,
    start_hidden: bool,
) {
    let (cmd_tx, mut cmd_rx) = futures_channel::mpsc::unbounded();
    let window_visible = Arc::new(AtomicBool::new(!start_hidden));

    let tray_handle: Rc<RefCell<Option<ksni::blocking::Handle<tray::TrayState>>>> =
        Rc::new(RefCell::new(None));

    let refresh_tray = {
        let tray_handle = Rc::clone(&tray_handle);
        move || {
            if let Some(h) = tray_handle.borrow().as_ref() {
                let _ = h.update(|_| {});
            }
        }
    };

    win.connect_show({
        let window_visible = Arc::clone(&window_visible);
        let refresh_tray = refresh_tray.clone();
        move |_| {
            window_visible.store(true, Ordering::Relaxed);
            refresh_tray();
        }
    });
    win.connect_hide({
        let window_visible = Arc::clone(&window_visible);
        let refresh_tray = refresh_tray.clone();
        move |_| {
            window_visible.store(false, Ordering::Relaxed);
            refresh_tray();
        }
    });

    let set_enabled: Rc<dyn Fn(bool)> = Rc::new({
        let tray_handle = Rc::clone(&tray_handle);
        let mute = Arc::clone(&engine.mute);
        let window_visible = Arc::clone(&window_visible);
        let cmd_tx = cmd_tx.clone();
        move |enabled: bool| {
            let mut handle = tray_handle.borrow_mut();
            if enabled {
                if handle.is_none() {
                    let state = tray::TrayState {
                        window_visible: Arc::clone(&window_visible),
                        mute: Arc::clone(&mute),
                        cmd_tx: cmd_tx.clone(),
                    };
                    *handle = tray::spawn(state);
                }
            } else if let Some(h) = handle.take() {
                h.shutdown().wait();
            }
        }
    });

    set_enabled(settings.boolean("tray-icon"));

    app.add_action(&settings.create_action("tray-icon"));
    settings.connect_changed(Some("tray-icon"), {
        let set_enabled = Rc::clone(&set_enabled);
        move |s, key| set_enabled(s.boolean(key))
    });

    let mute_button: gtk4::ToggleButton = builder.object("mute_button").expect("mute_button");
    mute_button.connect_toggled(move |_| refresh_tray());

    let win = win.clone();
    let app = app.clone();
    glib::MainContext::default().spawn_local(async move {
        use futures_util::StreamExt;
        while let Some(cmd) = cmd_rx.next().await {
            match cmd {
                tray::TrayCommand::ToggleWindow => {
                    if win.is_visible() {
                        win.set_visible(false);
                    } else {
                        win.present();
                    }
                }
                tray::TrayCommand::ToggleMute => {
                    mute_button.set_active(!mute_button.is_active());
                }
                tray::TrayCommand::Quit => app.quit(),
            }
        }
    });
}

fn normalize_gain_db(loudness: f32) -> f64 {
    (((TARGET_LOUDNESS_DBFS - loudness as f64) * 10.0).round() / 10.0).clamp(-20.0, 20.0)
}

fn apply_normalize(settings: &gio::Settings, loudness: &Mutex<Option<f32>>, key: &str) {
    if let Some(loudness) = *loudness.lock().unwrap() {
        let _ = settings.set_double(key, normalize_gain_db(loudness));
    }
}

fn wire_normalize_button(
    builder: &gtk4::Builder,
    id: &str,
    loudness: Arc<Mutex<Option<f32>>>,
    settings: gio::Settings,
    key: &'static str,
) {
    let btn: gtk4::Button = builder.object(id).expect(id);
    btn.connect_clicked(move |_| apply_normalize(&settings, &loudness, key));
}
