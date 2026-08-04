mod cab;
mod device;
mod eq;
mod gate;
mod nam;
mod processor;
mod tuner;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};

use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender};
use jack::{AudioIn, AudioOut, Client, ClientOptions, PortFlags};
use log::{debug, error, warn};
use nam_rs::Model;

const MAX_BLOCK_SIZE: usize = 8192;

use cab::CabConvolver;
pub use eq::EqPosition;
use eq::{EqChannel, EqCoeffs};
use gate::Gate;
pub use nam::ProfileKind;
use processor::NamProcessor;
pub(crate) use processor::Params;
pub(crate) use tuner::hz_to_note;

pub(super) fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

pub(super) fn spawn_background_load<T: Send + 'static>(
    target: &'static str,
    tx: mpsc::Sender<Option<T>>,
    path: Option<String>,
    load: impl FnOnce(&str) -> Option<T> + Send + 'static,
    on_clear: impl FnOnce() + Send + 'static,
    fail_message: impl FnOnce(&str) -> String + Send + 'static,
    event_tx: UnboundedSender<EngineEvent>,
) {
    std::thread::Builder::new()
        .name(target.into())
        .spawn(move || {
            let result = match path {
                None => {
                    debug!(target: target, "state=cleared");
                    on_clear();
                    None
                }
                Some(p) => {
                    debug!(target: target, "state=loading file={p}");
                    let result = load(&p);
                    if result.is_some() {
                        debug!(target: target, "state=loaded file={p}");
                    } else {
                        error!(target: target, "state=error file={p}");
                        on_clear();
                        let _ = event_tx.unbounded_send(EngineEvent::Warning(fail_message(&p)));
                    }
                    result
                }
            };
            let _ = tx.send(result);
        })
        .expect("background load thread spawn failed");
}

struct Notifications;

impl jack::NotificationHandler for Notifications {
    fn xrun(&mut self, _: &Client) -> jack::Control {
        warn!(target: "jack", "xrun (buffer under/overrun)");
        jack::Control::Continue
    }
}

pub enum EngineEvent {
    Warning(String),
    ProfileLoaded(ProfileKind),
}

pub struct InitialParams {
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub buffer_size: u32,
    pub mute: bool,
    pub gate_enabled: bool,
    pub gate_threshold_db: f32,
    pub pedal_profile_path: Option<String>,
    pub pedal_in_gain_db: f32,
    pub pedal_out_gain_db: f32,
    pub pedal_bypass: bool,
    pub amp_profile_path: Option<String>,
    pub amp_in_gain_db: f32,
    pub amp_out_gain_db: f32,
    pub amp_bypass: bool,
    pub cab_path: Option<String>,
    pub cab_level_db: f32,
    pub cab_bypass: bool,
    pub eq_enabled: bool,
    pub eq_pos: EqPosition,
    pub eq_low_db: f32,
    pub eq_mid_db: f32,
    pub eq_high_db: f32,
    pub eq_hp_freq: f32,
    pub eq_lp_freq: f32,
}

pub struct AudioEngine {
    pub mute: Arc<AtomicBool>,
    pedal_profile_tx: mpsc::Sender<Option<Model>>,
    pub pedal_loudness: Arc<Mutex<Option<f32>>>,
    pub pedal_bypass: Arc<AtomicBool>,
    amp_profile_tx: mpsc::Sender<Option<Model>>,
    pub amp_loudness: Arc<Mutex<Option<f32>>>,
    pub amp_bypass: Arc<AtomicBool>,
    cab_tx: mpsc::Sender<Option<CabConvolver>>,
    pub cab_bypass: Arc<AtomicBool>,
    params: Arc<Mutex<Params>>,
    client: jack::AsyncClient<Notifications, NamProcessor>,
    sample_rate: u32,
    pub tuner_hz_rx: RefCell<Option<futures_channel::mpsc::UnboundedReceiver<f32>>>,
    pub tuner_enabled: Arc<AtomicBool>,
    tuner_shutdown: Arc<AtomicBool>,
    event_tx: UnboundedSender<EngineEvent>,
    pub event_rx: RefCell<Option<UnboundedReceiver<EngineEvent>>>,
}

impl AudioEngine {
    pub fn new(params: InitialParams) -> Result<Self, String> {
        let (client, _status) = Client::new("namplay", ClientOptions::NO_START_SERVER)
            .map_err(|e| format!("JACK connection failed: {e}"))?;

        let (event_tx, event_rx) = futures_channel::mpsc::unbounded();

        debug!(target: "jack", "buffer_size={}", params.buffer_size);
        if let Err(e) = client.set_buffer_size(params.buffer_size) {
            warn!(target: "jack", "state=error buffer_size={} reason={e}", params.buffer_size);
            let _ = event_tx.unbounded_send(EngineEvent::Warning(format!(
                "JACK: failed to set buffer size to {}",
                params.buffer_size
            )));
        }

        let sample_rate = client.sample_rate();
        debug!(target: "jack", "state=connected sample_rate={sample_rate}Hz");

        let in_port = client
            .register_port("input", AudioIn::default())
            .map_err(|e| format!("register input port: {e}"))?;
        let out_port_1 = client
            .register_port("out_1", AudioOut::default())
            .map_err(|e| format!("register out_1 port: {e}"))?;
        let out_port_2 = client
            .register_port("out_2", AudioOut::default())
            .map_err(|e| format!("register out_2 port: {e}"))?;

        let (pedal_profile_tx, pedal_profile_rx) = mpsc::channel();
        let pedal_loudness = Arc::new(Mutex::new(None::<f32>));
        let pedal_bypass = Arc::new(AtomicBool::new(params.pedal_bypass));
        let (amp_profile_tx, amp_profile_rx) = mpsc::channel();
        let amp_loudness = Arc::new(Mutex::new(None::<f32>));
        let amp_bypass = Arc::new(AtomicBool::new(params.amp_bypass));
        let cab_bypass = Arc::new(AtomicBool::new(params.cab_bypass));
        let (cab_tx, cab_rx) = mpsc::channel::<Option<CabConvolver>>();

        let mute = Arc::new(AtomicBool::new(params.mute));

        debug!(target: "mute", "state={}", if params.mute { "on" } else { "off" });
        debug!(
            target: "gate",
            "state={} threshold={}dB",
            if params.gate_enabled { "on" } else { "off" },
            params.gate_threshold_db
        );
        debug!(
            target: "pedal",
            "in={}dB out={}dB bypass={}",
            params.pedal_in_gain_db,
            params.pedal_out_gain_db,
            if params.pedal_bypass { "on" } else { "off" }
        );
        debug!(
            target: "amp",
            "in={}dB out={}dB bypass={}",
            params.amp_in_gain_db,
            params.amp_out_gain_db,
            if params.amp_bypass { "on" } else { "off" }
        );
        debug!(
            target: "cab",
            "level={}dB bypass={}",
            params.cab_level_db,
            if params.cab_bypass { "on" } else { "off" }
        );
        debug!(
            target: "eq",
            "state={} position={} low={}dB mid={}dB high={}dB hp={}Hz lp={}Hz",
            if params.eq_enabled { "on" } else { "off" },
            params.eq_pos.setting(),
            params.eq_low_db,
            params.eq_mid_db,
            params.eq_high_db,
            params.eq_hp_freq,
            params.eq_lp_freq,
        );

        let initial_params = Params {
            gate_enabled: params.gate_enabled,
            gate_threshold_db: params.gate_threshold_db,
            pedal_in_gain: db_to_gain(params.pedal_in_gain_db),
            pedal_out_gain: db_to_gain(params.pedal_out_gain_db),
            amp_in_gain: db_to_gain(params.amp_in_gain_db),
            amp_out_gain: db_to_gain(params.amp_out_gain_db),
            cab_level: db_to_gain(params.cab_level_db),
            eq_enabled: params.eq_enabled,
            eq_pos: params.eq_pos,
            eq_low_db: params.eq_low_db,
            eq_mid_db: params.eq_mid_db,
            eq_high_db: params.eq_high_db,
            eq_hp_freq: params.eq_hp_freq,
            eq_lp_freq: params.eq_lp_freq,
        };
        let shared_params = Arc::new(Mutex::new(initial_params));

        let tuner_enabled = Arc::new(AtomicBool::new(false));
        let tuner_shutdown = Arc::new(AtomicBool::new(false));
        let tuner_samples: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
        let (tuner_hz_tx, tuner_hz_rx) = futures_channel::mpsc::unbounded();
        tuner::spawn(
            Arc::clone(&tuner_samples),
            Arc::clone(&tuner_enabled),
            Arc::clone(&tuner_shutdown),
            sample_rate,
            tuner_hz_tx,
        );

        let eq_coeffs = EqCoeffs::new(
            params.eq_low_db,
            params.eq_mid_db,
            params.eq_high_db,
            params.eq_hp_freq,
            params.eq_lp_freq,
            sample_rate as f32,
        );

        let processor = NamProcessor {
            mute: Arc::clone(&mute),
            gate: Gate::new(params.gate_threshold_db, sample_rate),
            pedal_profile_rx,
            current_pedal_profile: None,
            pedal_bypass: Arc::clone(&pedal_bypass),
            amp_profile_rx,
            current_amp_profile: None,
            amp_bypass: Arc::clone(&amp_bypass),
            cab_rx,
            current_cab: None,
            cab_bypass: Arc::clone(&cab_bypass),
            params: Arc::clone(&shared_params),
            last_params: initial_params,
            eq_coeffs,
            eq: EqChannel::new(),
            conv_buf: vec![0.0f32; MAX_BLOCK_SIZE],
            in_port,
            out_port_1,
            out_port_2,
            tuner_samples: Arc::clone(&tuner_samples),
            tuner_enabled: Arc::clone(&tuner_enabled),
        };

        let active_client = client
            .activate_async(Notifications, processor)
            .map_err(|e| format!("JACK: activation failed: {e}"))?;

        let engine = AudioEngine {
            mute,
            pedal_profile_tx,
            pedal_loudness,
            pedal_bypass,
            amp_profile_tx,
            amp_loudness,
            amp_bypass,
            cab_tx,
            cab_bypass,
            params: shared_params,
            client: active_client,
            sample_rate,
            tuner_hz_rx: RefCell::new(Some(tuner_hz_rx)),
            tuner_enabled,
            tuner_shutdown,
            event_tx,
            event_rx: RefCell::new(Some(event_rx)),
        };

        engine.load_pedal_profile(params.pedal_profile_path);
        engine.load_amp_profile(params.amp_profile_path);
        engine.load_cab(params.cab_path);
        engine.set_input_device(params.input_device);
        engine.set_output_device(params.output_device);

        Ok(engine)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn buffer_size(&self) -> u32 {
        self.client.as_client().buffer_size()
    }

    pub fn set_buffer_size(&self, frames: u32) {
        debug!(target: "jack", "buffer_size={frames}");
        if let Err(e) = self.client.as_client().set_buffer_size(frames) {
            error!(target: "jack", "state=error buffer_size={frames} reason={e}");
            let _ = self.event_tx.unbounded_send(EngineEvent::Warning(format!(
                "JACK: failed to set buffer size to {frames}"
            )));
        }
    }

    pub fn input_devices(&self) -> Vec<String> {
        device::audio_devices(self.client.as_client(), PortFlags::IS_OUTPUT)
    }

    pub fn output_devices(&self) -> Vec<String> {
        device::audio_devices(self.client.as_client(), PortFlags::IS_INPUT)
    }

    pub fn set_input_device(&self, device: Option<String>) {
        let client = self.client.as_client();

        if let Some(port) = client.port_by_name("namplay:input") {
            let _ = client.disconnect(&port);
        }

        let Some(device) = device else {
            debug!(target: "input", "state=cleared");
            return;
        };

        let sources = device::matching_ports(client, &device, PortFlags::IS_OUTPUT);

        match sources.first() {
            Some(source) => {
                if let Err(e) = client.connect_ports_by_name(source, "namplay:input") {
                    error!(
                        target: "input",
                        "state=error device={device} port={source} reason={e}"
                    );
                    let _ = self.event_tx.unbounded_send(EngineEvent::Warning(format!(
                        "Input: failed to connect {source}"
                    )));
                } else {
                    debug!(target: "input", "state=connected device={device} port={source}");
                }
            }
            None => {
                warn!(target: "input", "state=not_found device={device}");
                let _ = self.event_tx.unbounded_send(EngineEvent::Warning(format!(
                    "Input: device not found: {device}"
                )));
            }
        }
    }

    pub fn set_output_device(&self, device: Option<String>) {
        let client = self.client.as_client();

        for own_port in ["namplay:out_1", "namplay:out_2"] {
            if let Some(port) = client.port_by_name(own_port) {
                let _ = client.disconnect(&port);
            }
        }

        let Some(device) = device else {
            debug!(target: "output", "state=cleared");
            return;
        };

        let destinations = device::matching_ports(client, &device, PortFlags::IS_INPUT);

        if destinations.is_empty() {
            warn!(target: "output", "state=not_found device={device}");
            let _ = self.event_tx.unbounded_send(EngineEvent::Warning(format!(
                "Output: device not found: {device}"
            )));
            return;
        }

        for (own_port, dest) in ["namplay:out_1", "namplay:out_2"]
            .into_iter()
            .zip(destinations.iter())
        {
            if let Err(e) = client.connect_ports_by_name(own_port, dest) {
                error!(
                    target: "output",
                    "state=error device={device} port={own_port} dest={dest} reason={e}"
                );
                let _ = self.event_tx.unbounded_send(EngineEvent::Warning(format!(
                    "Output: failed to connect {own_port} to {dest}"
                )));
            } else {
                debug!(
                    target: "output",
                    "state=connected device={device} port={own_port} dest={dest}"
                );
            }
        }
    }

    pub fn set_mute(&self, muted: bool) {
        debug!(target: "mute", "state={}", if muted { "on" } else { "off" });
        self.mute.store(muted, Ordering::Relaxed);
    }

    pub fn set_gate_enabled(&self, enabled: bool) {
        debug!(target: "gate", "state={}", if enabled { "on" } else { "off" });
        self.params.lock().unwrap().gate_enabled = enabled;
    }

    pub fn set_gate_threshold_db(&self, db: f32) {
        debug!(target: "gate", "threshold={db}dB");
        self.params.lock().unwrap().gate_threshold_db = db;
    }

    pub fn load_pedal_profile(&self, path: Option<String>) {
        nam::load(
            ProfileKind::Pedal,
            self.pedal_profile_tx.clone(),
            path,
            self.sample_rate,
            Arc::clone(&self.pedal_loudness),
            self.event_tx.clone(),
        );
    }

    pub fn set_pedal_in_gain_db(&self, db: f32) {
        debug!(target: "pedal", "in={db}dB");
        self.params.lock().unwrap().pedal_in_gain = db_to_gain(db);
    }

    pub fn set_pedal_out_gain_db(&self, db: f32) {
        debug!(target: "pedal", "out={db}dB");
        self.params.lock().unwrap().pedal_out_gain = db_to_gain(db);
    }

    pub fn set_pedal_bypass(&self, bypassed: bool) {
        debug!(target: "pedal", "bypass={}", if bypassed { "on" } else { "off" });
        self.pedal_bypass.store(bypassed, Ordering::Relaxed);
    }

    pub fn load_amp_profile(&self, path: Option<String>) {
        nam::load(
            ProfileKind::Amp,
            self.amp_profile_tx.clone(),
            path,
            self.sample_rate,
            Arc::clone(&self.amp_loudness),
            self.event_tx.clone(),
        );
    }

    pub fn set_amp_in_gain_db(&self, db: f32) {
        debug!(target: "amp", "in={db}dB");
        self.params.lock().unwrap().amp_in_gain = db_to_gain(db);
    }

    pub fn set_amp_out_gain_db(&self, db: f32) {
        debug!(target: "amp", "out={db}dB");
        self.params.lock().unwrap().amp_out_gain = db_to_gain(db);
    }

    pub fn set_amp_bypass(&self, bypassed: bool) {
        debug!(target: "amp", "bypass={}", if bypassed { "on" } else { "off" });
        self.amp_bypass.store(bypassed, Ordering::Relaxed);
    }

    pub fn load_cab(&self, path: Option<String>) {
        cab::spawn(
            self.cab_tx.clone(),
            path,
            self.sample_rate,
            self.buffer_size() as usize,
            self.event_tx.clone(),
        );
    }

    pub fn set_cab_level_db(&self, db: f32) {
        debug!(target: "cab", "level={db}dB");
        self.params.lock().unwrap().cab_level = db_to_gain(db);
    }

    pub fn set_cab_bypass(&self, bypassed: bool) {
        debug!(target: "cab", "bypass={}", if bypassed { "on" } else { "off" });
        self.cab_bypass.store(bypassed, Ordering::Relaxed);
    }

    pub fn set_eq_enabled(&self, enabled: bool) {
        debug!(target: "eq", "state={}", if enabled { "on" } else { "off" });
        self.params.lock().unwrap().eq_enabled = enabled;
    }

    pub fn set_eq_pos(&self, pos: EqPosition) {
        debug!(target: "eq", "position={}", pos.setting());
        self.params.lock().unwrap().eq_pos = pos;
    }

    pub fn set_eq_low_db(&self, db: f32) {
        debug!(target: "eq", "low={db}dB");
        self.params.lock().unwrap().eq_low_db = db;
    }

    pub fn set_eq_mid_db(&self, db: f32) {
        debug!(target: "eq", "mid={db}dB");
        self.params.lock().unwrap().eq_mid_db = db;
    }

    pub fn set_eq_high_db(&self, db: f32) {
        debug!(target: "eq", "high={db}dB");
        self.params.lock().unwrap().eq_high_db = db;
    }

    pub fn set_eq_hp_freq(&self, hz: f32) {
        debug!(target: "eq", "high-pass={hz}Hz");
        self.params.lock().unwrap().eq_hp_freq = hz;
    }

    pub fn set_eq_lp_freq(&self, hz: f32) {
        debug!(target: "eq", "low-pass={hz}Hz");
        self.params.lock().unwrap().eq_lp_freq = hz;
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.tuner_shutdown.store(true, Ordering::Relaxed);
    }
}
