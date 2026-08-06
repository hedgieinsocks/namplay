mod audio;
mod keys;
mod preset;
mod tray;
mod ui;

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

use audio::{AudioEngine, EngineEvent, EqPosition, InitialParams, ProfileKind};
use keys::*;
use ui::{
    bind_adjustment, bind_toggle, create_tuner_window, path_from_settings, restore_window_state,
    save_window_state, setup_audio_window, setup_buffer_size_dropdown, setup_eq_position,
    setup_file_picker_row, setup_preset_actions, setup_primary_menu, setup_reset_button,
    show_persistent_toast, FilePickerSpec,
};

pub(crate) const APP_ID: &str = "io.github.hedgieinsocks.Namplay";
const UI: &str = include_str!(concat!(env!("OUT_DIR"), "/window.ui"));
const TARGET_LOUDNESS_DBFS: f64 = -18.0;

const FILE_PICKERS: &[FilePickerSpec] = &[
    FilePickerSpec {
        prefix: "pedal",
        key: PEDAL_PATH,
        title: "Choose Pedal Profile",
        filter_name: "NAM Profiles",
        filter_suffix: "nam",
    },
    FilePickerSpec {
        prefix: "amp",
        key: AMP_PATH,
        title: "Choose Amp Profile",
        filter_name: "NAM Profiles",
        filter_suffix: "nam",
    },
    FilePickerSpec {
        prefix: "cab",
        key: CAB_PATH,
        title: "Choose Cabinet IR",
        filter_name: "WAV Files",
        filter_suffix: "wav",
    },
];

type SliderSetter = fn(&AudioEngine, f32);

const SLIDERS: &[(&str, SliderSetter)] = &[
    (GATE_THRESHOLD, AudioEngine::set_gate_threshold_db),
    (EQ_HP, AudioEngine::set_eq_hp_freq),
    (EQ_LOW, AudioEngine::set_eq_low_db),
    (EQ_MID, AudioEngine::set_eq_mid_db),
    (EQ_HIGH, AudioEngine::set_eq_high_db),
    (EQ_LP, AudioEngine::set_eq_lp_freq),
    (PEDAL_INPUT, AudioEngine::set_pedal_in_gain_db),
    (PEDAL_OUTPUT, AudioEngine::set_pedal_out_gain_db),
    (AMP_INPUT, AudioEngine::set_amp_in_gain_db),
    (AMP_OUTPUT, AudioEngine::set_amp_out_gain_db),
    (CAB_LEVEL, AudioEngine::set_cab_level_db),
];

fn dispatch_slider_change(engine: &AudioEngine, s: &gio::Settings, key: &str) -> bool {
    match SLIDERS.iter().find(|(k, _)| *k == key) {
        Some((_, setter)) => {
            setter(engine, s.double(key) as f32);
            true
        }
        None => false,
    }
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

    bind_toggle(&builder, &settings, "gate_row", GATE_ENABLED);
    bind_toggle(&builder, &settings, "eq_row", EQ_ENABLED);

    for (key, _) in SLIDERS {
        let id_base = key.replace('-', "_");
        bind_adjustment(&builder, &settings, &format!("{id_base}_adjustment"), key);
        setup_reset_button(&builder, &settings, &format!("{id_base}_reset_button"), key);
    }

    setup_eq_position(&builder, &settings);
    setup_buffer_size_dropdown(&builder, &settings);

    let toast_overlay: adw::ToastOverlay = builder.object("toast_overlay").expect("toast_overlay");

    let pedal_profile_path = path_from_settings(&settings, PEDAL_PATH);
    let amp_profile_path = path_from_settings(&settings, AMP_PATH);

    let pedal_skip_normalize = Rc::new(Cell::new(pedal_profile_path.is_some()));
    let amp_skip_normalize = Rc::new(Cell::new(amp_profile_path.is_some()));

    match AudioEngine::new(InitialParams {
        input_device: path_from_settings(&settings, INPUT_DEVICE),
        output_device: path_from_settings(&settings, OUTPUT_DEVICE),
        buffer_size: settings.int(BUFFER_SIZE) as u32,
        mute: settings.boolean(MUTE),
        gate_enabled: settings.boolean(GATE_ENABLED),
        gate_threshold_db: settings.double(GATE_THRESHOLD) as f32,
        pedal_profile_path: pedal_profile_path.clone(),
        pedal_in_gain_db: settings.double(PEDAL_INPUT) as f32,
        pedal_out_gain_db: settings.double(PEDAL_OUTPUT) as f32,
        pedal_bypass: settings.boolean(PEDAL_BYPASS),
        amp_profile_path: amp_profile_path.clone(),
        amp_in_gain_db: settings.double(AMP_INPUT) as f32,
        amp_out_gain_db: settings.double(AMP_OUTPUT) as f32,
        amp_bypass: settings.boolean(AMP_BYPASS),
        cab_path: path_from_settings(&settings, CAB_PATH),
        cab_level_db: settings.double(CAB_LEVEL) as f32,
        cab_bypass: settings.boolean(CAB_BYPASS),
        eq_enabled: settings.boolean(EQ_ENABLED),
        eq_pos: EqPosition::from_setting(settings.string(EQ_POSITION).as_str()),
        eq_low_db: settings.double(EQ_LOW) as f32,
        eq_mid_db: settings.double(EQ_MID) as f32,
        eq_high_db: settings.double(EQ_HIGH) as f32,
        eq_hp_freq: settings.double(EQ_HP) as f32,
        eq_lp_freq: settings.double(EQ_LP) as f32,
    }) {
        Ok(engine) => {
            let engine = Rc::new(engine);
            setup_audio_window(&builder, &settings, &engine);

            setup_toggle_button(&builder, &settings, "mute_button", MUTE);
            setup_toggle_button(&builder, &settings, "pedal_bypass_button", PEDAL_BYPASS);
            setup_toggle_button(&builder, &settings, "amp_bypass_button", AMP_BYPASS);
            setup_toggle_button(&builder, &settings, "cab_bypass_button", CAB_BYPASS);

            setup_tray(
                app,
                &win,
                &builder,
                &settings,
                &engine,
                start_hidden,
                &toast_overlay,
            );

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

            setup_normalize_button(
                &builder,
                "pedal_output_normalize_button",
                Arc::clone(&engine.pedal_loudness),
                &settings,
                PEDAL_OUTPUT,
            );
            setup_normalize_button(
                &builder,
                "amp_output_normalize_button",
                Arc::clone(&engine.amp_loudness),
                &settings,
                AMP_OUTPUT,
            );

            let event_rx = engine
                .event_rx
                .borrow_mut()
                .take()
                .expect("event receiver already taken");
            glib::MainContext::default().spawn_local({
                let toast_overlay = toast_overlay.clone();
                let settings = settings.clone();
                let pedal_loudness = Arc::clone(&engine.pedal_loudness);
                let amp_loudness = Arc::clone(&engine.amp_loudness);
                let pedal_skip_normalize = Rc::clone(&pedal_skip_normalize);
                let amp_skip_normalize = Rc::clone(&amp_skip_normalize);
                async move {
                    use futures_util::StreamExt;
                    let mut event_rx = event_rx;
                    while let Some(event) = event_rx.next().await {
                        match event {
                            EngineEvent::Warning(msg) => {
                                show_persistent_toast(&toast_overlay, &msg)
                            }
                            EngineEvent::ProfileLoaded(kind) => {
                                let (loudness, key, skip_normalize) = match kind {
                                    ProfileKind::Pedal => {
                                        (&pedal_loudness, PEDAL_OUTPUT, &pedal_skip_normalize)
                                    }
                                    ProfileKind::Amp => {
                                        (&amp_loudness, AMP_OUTPUT, &amp_skip_normalize)
                                    }
                                };
                                if skip_normalize.replace(false) {
                                    continue;
                                }
                                if settings.boolean(NORMALIZE_OUTPUT) {
                                    apply_normalize(&settings, loudness, key);
                                }
                            }
                        }
                    }
                }
            });

            settings.connect_changed(None, move |s, key| {
                if dispatch_slider_change(&engine, s, key) {
                    return;
                }
                match key {
                    INPUT_DEVICE => engine.set_input_device(path_from_settings(s, key)),
                    OUTPUT_DEVICE => engine.set_output_device(path_from_settings(s, key)),
                    BUFFER_SIZE => engine.set_buffer_size(s.int(key) as u32),
                    MUTE => engine.set_mute(s.boolean(key)),
                    GATE_ENABLED => engine.set_gate_enabled(s.boolean(key)),
                    PEDAL_PATH => engine.load_pedal_profile(path_from_settings(s, key)),
                    PEDAL_BYPASS => engine.set_pedal_bypass(s.boolean(key)),
                    AMP_PATH => engine.load_amp_profile(path_from_settings(s, key)),
                    AMP_BYPASS => engine.set_amp_bypass(s.boolean(key)),
                    CAB_PATH => engine.load_cab(path_from_settings(s, key)),
                    CAB_BYPASS => engine.set_cab_bypass(s.boolean(key)),
                    EQ_ENABLED => engine.set_eq_enabled(s.boolean(key)),
                    EQ_POSITION => {
                        engine.set_eq_pos(EqPosition::from_setting(s.string(key).as_str()))
                    }
                    _ => {}
                }
            });
        }
        Err(e) => {
            error!(target: "audio", "state=unavailable reason={e}");
            show_persistent_toast(&toast_overlay, "Audio unavailable");
        }
    }

    win.connect_close_request({
        let settings = settings.clone();
        move |w| {
            save_window_state(w, &settings);
            if settings.boolean("run-in-background") {
                w.set_visible(false);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
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

fn setup_toggle_button(
    builder: &gtk4::Builder,
    settings: &gio::Settings,
    id: &str,
    key: &'static str,
) {
    let btn: gtk4::ToggleButton = builder.object(id).expect(id);
    btn.connect_toggled(|btn| {
        btn.set_icon_name(if btn.is_active() {
            "audio-volume-muted-symbolic"
        } else {
            "audio-volume-high-symbolic"
        });
    });
    settings.bind(key, &btn, "active").build();
}

fn setup_tray(
    app: &adw::Application,
    win: &adw::ApplicationWindow,
    builder: &gtk4::Builder,
    settings: &gio::Settings,
    engine: &Rc<AudioEngine>,
    start_hidden: bool,
    toast_overlay: &adw::ToastOverlay,
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

    let set_enabled = {
        let tray_handle = Rc::clone(&tray_handle);
        let mute = Arc::clone(&engine.mute);
        let window_visible = Arc::clone(&window_visible);
        let cmd_tx = cmd_tx.clone();
        let toast_overlay = toast_overlay.clone();
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
                    if handle.is_none() {
                        show_persistent_toast(&toast_overlay, "Tray: failed to start icon");
                    }
                }
            } else if let Some(h) = handle.take() {
                h.shutdown().wait();
            }
        }
    };

    set_enabled(settings.boolean(TRAY_ICON));

    app.add_action(&settings.create_action(TRAY_ICON));
    settings.connect_changed(Some(TRAY_ICON), move |s, key| set_enabled(s.boolean(key)));

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
    preset::round1(TARGET_LOUDNESS_DBFS - loudness as f64).clamp(-20.0, 20.0)
}

fn apply_normalize(settings: &gio::Settings, loudness: &Mutex<Option<f32>>, key: &str) {
    if let Some(loudness) = *loudness.lock().unwrap() {
        let _ = settings.set_double(key, normalize_gain_db(loudness));
    }
}

fn setup_normalize_button(
    builder: &gtk4::Builder,
    id: &str,
    loudness: Arc<Mutex<Option<f32>>>,
    settings: &gio::Settings,
    key: &'static str,
) {
    let btn: gtk4::Button = builder.object(id).expect(id);
    let settings = settings.clone();
    btn.connect_clicked(move |_| apply_normalize(&settings, &loudness, key));
}
