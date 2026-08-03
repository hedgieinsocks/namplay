use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gio::prelude::*;
use gtk4::prelude::*;

use crate::audio::AudioEngine;
use crate::keys::*;

const BUFFER_SIZES: &[u32] = &[16, 32, 64, 128, 192, 256, 320, 384, 448, 512];

pub fn setup_buffer_size_dropdown(builder: &gtk4::Builder, settings: &gio::Settings) {
    let dropdown: gtk4::DropDown = builder
        .object("buffer_size_dropdown")
        .expect("buffer_size_dropdown");

    let labels: Vec<String> = BUFFER_SIZES.iter().map(|n| n.to_string()).collect();
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    dropdown.set_model(Some(&gtk4::StringList::new(&labels)));

    let index_for = |frames: i32| {
        BUFFER_SIZES
            .iter()
            .position(|&n| n as i32 == frames)
            .unwrap_or_else(|| BUFFER_SIZES.iter().position(|&n| n == 256).unwrap_or(0))
            as u32
    };

    dropdown.set_selected(index_for(settings.int(BUFFER_SIZE)));

    dropdown.connect_selected_notify({
        let settings = settings.clone();
        move |dd| {
            if let Some(&frames) = BUFFER_SIZES.get(dd.selected() as usize) {
                let _ = settings.set_int(BUFFER_SIZE, frames as i32);
            }
        }
    });

    settings.connect_changed(Some(BUFFER_SIZE), move |s, key| {
        dropdown.set_selected(index_for(s.int(key)));
    });
}

fn format_latency(buffer_size: u32, sample_rate: u32) -> String {
    format!("{:.1}", buffer_size as f64 / sample_rate as f64 * 1000.0)
}

pub fn setup_audio_window(
    builder: &gtk4::Builder,
    settings: &gio::Settings,
    engine: &Rc<AudioEngine>,
) {
    let sample_rate_label: gtk4::Label = builder
        .object("sample_rate_label")
        .expect("sample_rate_label");
    sample_rate_label.set_text(&engine.sample_rate().to_string());

    let latency_label: gtk4::Label = builder.object("latency_label").expect("latency_label");
    latency_label.set_text(&format_latency(engine.buffer_size(), engine.sample_rate()));

    setup_device_rows(builder, settings, Rc::clone(engine));

    settings.connect_changed(Some(BUFFER_SIZE), {
        let engine = Rc::clone(engine);
        move |_, _| {
            // PipeWire applies the new buffer size to the graph asynchronously,
            // so jack_get_buffer_size() briefly still reports the old value.
            let latency_label = latency_label.clone();
            let engine = Rc::clone(&engine);
            glib::timeout_add_local_once(Duration::from_millis(100), move || {
                latency_label.set_text(&format_latency(engine.buffer_size(), engine.sample_rate()));
            });
        }
    });
}

fn setup_device_rows(builder: &gtk4::Builder, settings: &gio::Settings, engine: Rc<AudioEngine>) {
    setup_device_dropdown(
        builder,
        settings,
        "input_device_dropdown",
        "input_device_refresh_button",
        INPUT_DEVICE,
        Rc::clone(&engine),
        AudioEngine::input_devices,
    );
    setup_device_dropdown(
        builder,
        settings,
        "output_device_dropdown",
        "output_device_refresh_button",
        OUTPUT_DEVICE,
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

    const NONE_LABEL: &str = "None";
    let known: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

    let rebuild = {
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
            dropdown.set_selected(selected_index(&current, &devices));
        }
    };

    rebuild(list_devices(&engine));

    dropdown.connect_selected_notify({
        let settings = settings.clone();
        move |dd| {
            let name = if dd.selected() == 0 {
                String::new()
            } else {
                dd.selected_item()
                    .and_downcast::<gtk4::StringObject>()
                    .map(|s| s.string().to_string())
                    .unwrap_or_default()
            };
            let _ = settings.set_string(key, &name);
        }
    });

    settings.connect_changed(Some(key), {
        let dropdown = dropdown.clone();
        let known = Rc::clone(&known);
        move |s, k| {
            let current = s.string(k).to_string();
            dropdown.set_selected(selected_index(&current, &known.borrow()));
        }
    });

    refresh_button.connect_clicked(move |_| {
        rebuild(list_devices(&engine));
    });
}

fn selected_index(current: &str, devices: &[String]) -> u32 {
    if current.is_empty() {
        0
    } else {
        devices
            .iter()
            .position(|d| d == current)
            .map(|i| i as u32 + 1)
            .unwrap_or(0)
    }
}
