mod cab;
mod device;
mod eq;
mod gate;
mod nam;
mod processor;
mod tuner;
mod util;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc, Arc, Mutex,
};

use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender};
use jack::{AudioIn, AudioOut, Client, ClientOptions, PortFlags};
use log::{debug, error, warn};
use nam_rs::Model;

const MAX_BLOCK_SIZE: usize = 8192;

use cab::CabConvolvers;
pub use eq::EqPosition;
use eq::{EqChannel, EqCoeffs};
use gate::Gate;
use processor::NamProcessor;
use util::db_to_gain;
pub(crate) use util::AtomicF32;

struct Notifications;

impl jack::NotificationHandler for Notifications {
    fn xrun(&mut self, _: &Client) -> jack::Control {
        warn!(target: "jack", "xrun (buffer under/overrun)");
        jack::Control::Continue
    }
}

pub struct InitialParams {
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub buffer_size: u32,
    pub gate_enabled: bool,
    pub gate_threshold_db: f32,
    pub pedal_profile_path: Option<String>,
    pub pedal_in_gain_db: f32,
    pub pedal_out_gain_db: f32,
    pub amp_profile_path: Option<String>,
    pub amp_in_gain_db: f32,
    pub amp_out_gain_db: f32,
    pub cab_path: Option<String>,
    pub cab_level_db: f32,
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
    gate_enabled: Arc<AtomicBool>,
    gate_threshold_db: Arc<AtomicF32>,
    pedal_profile_tx: mpsc::Sender<Option<Model>>,
    pub pedal_loudness: Arc<Mutex<Option<f32>>>,
    pub pedal_bypass: Arc<AtomicBool>,
    pedal_in_gain: Arc<AtomicF32>,
    pedal_out_gain: Arc<AtomicF32>,
    amp_profile_tx: mpsc::Sender<Option<Model>>,
    pub amp_loudness: Arc<Mutex<Option<f32>>>,
    pub amp_bypass: Arc<AtomicBool>,
    amp_in_gain: Arc<AtomicF32>,
    amp_out_gain: Arc<AtomicF32>,
    cab_tx: mpsc::Sender<Option<CabConvolvers>>,
    pub cab_bypass: Arc<AtomicBool>,
    cab_level: Arc<AtomicF32>,
    eq_enabled: Arc<AtomicBool>,
    eq_pos: Arc<AtomicU32>,
    eq_low_db: Arc<AtomicF32>,
    eq_mid_db: Arc<AtomicF32>,
    eq_high_db: Arc<AtomicF32>,
    eq_hp_freq: Arc<AtomicF32>,
    eq_lp_freq: Arc<AtomicF32>,
    _client: jack::AsyncClient<Notifications, NamProcessor>,
    sample_rate: u32,
    block_size: usize,
    pub tuner_hz_rx: RefCell<Option<futures_channel::mpsc::UnboundedReceiver<f32>>>,
    pub tuner_enabled: Arc<AtomicBool>,
    tuner_shutdown: Arc<AtomicBool>,
    warning_tx: UnboundedSender<String>,
    pub warning_rx: RefCell<Option<UnboundedReceiver<String>>>,
}

impl AudioEngine {
    pub fn new(params: InitialParams) -> Result<Self, String> {
        let (client, _status) = Client::new("namplay", ClientOptions::NO_START_SERVER)
            .map_err(|e| format!("JACK connection failed: {e}"))?;

        let (warning_tx, warning_rx) = futures_channel::mpsc::unbounded();

        debug!(target: "jack", "buffer_size={}", params.buffer_size);
        if let Err(e) = client.set_buffer_size(params.buffer_size) {
            let detail = format!("failed to set buffer size to {}: {e}", params.buffer_size);
            warn!(target: "jack", "{detail}");
            let _ = warning_tx.unbounded_send(format!("JACK: {detail}"));
        }

        let sample_rate = client.sample_rate();
        let block_size = client.buffer_size() as usize;
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
        let pedal_bypass = Arc::new(AtomicBool::new(false));
        let (amp_profile_tx, amp_profile_rx) = mpsc::channel();
        let amp_loudness = Arc::new(Mutex::new(None::<f32>));
        let amp_bypass = Arc::new(AtomicBool::new(false));
        let cab_bypass = Arc::new(AtomicBool::new(false));
        let (cab_tx, cab_rx) = mpsc::channel::<Option<CabConvolvers>>();

        let mute = Arc::new(AtomicBool::new(false));
        let gate_enabled = Arc::new(AtomicBool::new(params.gate_enabled));
        let gate_threshold_db = Arc::new(AtomicF32::new(params.gate_threshold_db));
        let pedal_in_gain = Arc::new(AtomicF32::new(db_to_gain(params.pedal_in_gain_db)));
        let pedal_out_gain = Arc::new(AtomicF32::new(db_to_gain(params.pedal_out_gain_db)));
        let amp_in_gain = Arc::new(AtomicF32::new(db_to_gain(params.amp_in_gain_db)));
        let amp_out_gain = Arc::new(AtomicF32::new(db_to_gain(params.amp_out_gain_db)));
        let cab_level = Arc::new(AtomicF32::new(db_to_gain(params.cab_level_db)));
        let eq_enabled = Arc::new(AtomicBool::new(params.eq_enabled));
        let eq_pos = Arc::new(AtomicU32::new(params.eq_pos.index()));
        let eq_low_db = Arc::new(AtomicF32::new(params.eq_low_db));
        let eq_mid_db = Arc::new(AtomicF32::new(params.eq_mid_db));
        let eq_high_db = Arc::new(AtomicF32::new(params.eq_high_db));
        let eq_hp_freq = Arc::new(AtomicF32::new(params.eq_hp_freq));
        let eq_lp_freq = Arc::new(AtomicF32::new(params.eq_lp_freq));

        debug!(
            target: "gate",
            "state={} threshold={}dB",
            if params.gate_enabled { "on" } else { "off" },
            params.gate_threshold_db
        );
        debug!(
            target: "pedal",
            "in={}dB out={}dB",
            params.pedal_in_gain_db, params.pedal_out_gain_db
        );
        debug!(
            target: "amp",
            "in={}dB out={}dB",
            params.amp_in_gain_db, params.amp_out_gain_db
        );
        debug!(target: "cab", "level={}dB", params.cab_level_db);
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
            gate_enabled: Arc::clone(&gate_enabled),
            gate_threshold_db: Arc::clone(&gate_threshold_db),
            gate: Gate::new(params.gate_threshold_db, sample_rate),
            pedal_profile_rx,
            current_pedal_profile: None,
            pedal_bypass: Arc::clone(&pedal_bypass),
            pedal_in_gain: Arc::clone(&pedal_in_gain),
            pedal_out_gain: Arc::clone(&pedal_out_gain),
            amp_profile_rx,
            current_amp_profile: None,
            amp_bypass: Arc::clone(&amp_bypass),
            amp_in_gain: Arc::clone(&amp_in_gain),
            amp_out_gain: Arc::clone(&amp_out_gain),
            cab_rx,
            current_cab_l: None,
            current_cab_r: None,
            cab_bypass: Arc::clone(&cab_bypass),
            cab_level: Arc::clone(&cab_level),
            eq_enabled: Arc::clone(&eq_enabled),
            eq_pos: Arc::clone(&eq_pos),
            eq_low_db: Arc::clone(&eq_low_db),
            eq_mid_db: Arc::clone(&eq_mid_db),
            eq_high_db: Arc::clone(&eq_high_db),
            eq_hp_freq: Arc::clone(&eq_hp_freq),
            eq_lp_freq: Arc::clone(&eq_lp_freq),
            eq_coeffs,
            eq_l: EqChannel::new(),
            eq_r: EqChannel::new(),
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
            gate_enabled,
            gate_threshold_db,
            pedal_profile_tx,
            pedal_loudness,
            pedal_bypass,
            pedal_in_gain,
            pedal_out_gain,
            amp_profile_tx,
            amp_loudness,
            amp_bypass,
            amp_in_gain,
            amp_out_gain,
            cab_tx,
            cab_bypass,
            cab_level,
            eq_enabled,
            eq_pos,
            eq_low_db,
            eq_mid_db,
            eq_high_db,
            eq_hp_freq,
            eq_lp_freq,
            _client: active_client,
            sample_rate,
            block_size,
            tuner_hz_rx: RefCell::new(Some(tuner_hz_rx)),
            tuner_enabled,
            tuner_shutdown,
            warning_tx,
            warning_rx: RefCell::new(Some(warning_rx)),
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
        self._client.as_client().buffer_size()
    }

    pub fn set_buffer_size(&self, frames: u32) {
        debug!(target: "jack", "buffer_size={frames}");
        if let Err(e) = self._client.as_client().set_buffer_size(frames) {
            let detail = format!("failed to set buffer size to {frames}: {e}");
            error!(target: "jack", "{detail}");
            let _ = self.warning_tx.unbounded_send(format!("JACK: {detail}"));
        }
    }

    pub fn input_devices(&self) -> Vec<String> {
        device::audio_devices(self._client.as_client(), PortFlags::IS_OUTPUT)
    }

    pub fn output_devices(&self) -> Vec<String> {
        device::audio_devices(self._client.as_client(), PortFlags::IS_INPUT)
    }

    pub fn set_input_device(&self, device: Option<String>) {
        let client = self._client.as_client();

        if let Some(port) = client.port_by_name("namplay:input") {
            let _ = client.disconnect(&port);
        }

        let Some(device) = device else {
            debug!(target: "input", "device cleared");
            return;
        };

        let sources = device::matching_ports(client, &device, PortFlags::IS_OUTPUT);

        match sources.first() {
            Some(source) => {
                if let Err(e) = client.connect_ports_by_name(source, "namplay:input") {
                    let detail = format!("failed to connect {source}: {e}");
                    error!(target: "input", "{detail}");
                    let _ = self.warning_tx.unbounded_send(format!("Input: {detail}"));
                } else {
                    debug!(target: "input", "device={device} port={source}");
                }
            }
            None => {
                let detail = format!("device not found: {device}");
                warn!(target: "input", "{detail}");
                let _ = self.warning_tx.unbounded_send(format!("Input: {detail}"));
            }
        }
    }

    pub fn set_output_device(&self, device: Option<String>) {
        let client = self._client.as_client();

        for own_port in ["namplay:out_1", "namplay:out_2"] {
            if let Some(port) = client.port_by_name(own_port) {
                let _ = client.disconnect(&port);
            }
        }

        let Some(device) = device else {
            debug!(target: "output", "device cleared");
            return;
        };

        let destinations = device::matching_ports(client, &device, PortFlags::IS_INPUT);

        if destinations.is_empty() {
            let detail = format!("device not found: {device}");
            warn!(target: "output", "{detail}");
            let _ = self.warning_tx.unbounded_send(format!("Output: {detail}"));
            return;
        }

        for (own_port, dest) in ["namplay:out_1", "namplay:out_2"]
            .into_iter()
            .zip(destinations.iter())
        {
            if let Err(e) = client.connect_ports_by_name(own_port, dest) {
                let detail = format!("failed to connect {own_port} to {dest}: {e}");
                error!(target: "output", "{detail}");
                let _ = self.warning_tx.unbounded_send(format!("Output: {detail}"));
            } else {
                debug!(target: "output", "device={device} {own_port} -> {dest}");
            }
        }
    }

    pub fn set_gate_enabled(&self, enabled: bool) {
        debug!(target: "gate", "state={}", if enabled { "on" } else { "off" });
        self.gate_enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn set_gate_threshold_db(&self, db: f32) {
        debug!(target: "gate", "threshold={db}dB");
        self.gate_threshold_db.set(db);
    }

    pub fn load_pedal_profile(&self, path: Option<String>) {
        nam::load(
            "Pedal",
            self.pedal_profile_tx.clone(),
            path,
            self.sample_rate,
            Arc::clone(&self.pedal_loudness),
            self.warning_tx.clone(),
        );
    }

    pub fn set_pedal_in_gain_db(&self, db: f32) {
        debug!(target: "pedal", "in={db}dB");
        self.pedal_in_gain.set(db_to_gain(db));
    }

    pub fn set_pedal_out_gain_db(&self, db: f32) {
        debug!(target: "pedal", "out={db}dB");
        self.pedal_out_gain.set(db_to_gain(db));
    }

    pub fn load_amp_profile(&self, path: Option<String>) {
        nam::load(
            "Amp",
            self.amp_profile_tx.clone(),
            path,
            self.sample_rate,
            Arc::clone(&self.amp_loudness),
            self.warning_tx.clone(),
        );
    }

    pub fn set_amp_in_gain_db(&self, db: f32) {
        debug!(target: "amp", "in={db}dB");
        self.amp_in_gain.set(db_to_gain(db));
    }

    pub fn set_amp_out_gain_db(&self, db: f32) {
        debug!(target: "amp", "out={db}dB");
        self.amp_out_gain.set(db_to_gain(db));
    }

    pub fn load_cab(&self, path: Option<String>) {
        let tx = self.cab_tx.clone();
        let sample_rate = self.sample_rate;
        let block_size = self.block_size;
        let warning_tx = self.warning_tx.clone();
        std::thread::spawn(move || {
            let convolvers = match path {
                None => {
                    debug!(target: "cab", "file cleared");
                    None
                }
                Some(p) => {
                    debug!(target: "cab", "loading file: {p}");
                    let result = cab::load(&p, sample_rate, block_size, &warning_tx);
                    if result.is_some() {
                        debug!(target: "cab", "file loaded: {p}");
                    } else {
                        let detail = format!("failed to load file: {p}");
                        error!(target: "cab", "{detail}");
                        let _ = warning_tx.unbounded_send(format!("Cab: {detail}"));
                    }
                    result
                }
            };
            let _ = tx.send(convolvers);
        });
    }

    pub fn set_cab_level_db(&self, db: f32) {
        debug!(target: "cab", "level={db}dB");
        self.cab_level.set(db_to_gain(db));
    }

    pub fn set_eq_enabled(&self, enabled: bool) {
        debug!(target: "eq", "state={}", if enabled { "on" } else { "off" });
        self.eq_enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn set_eq_pos(&self, pos: EqPosition) {
        debug!(target: "eq", "position={}", pos.setting());
        self.eq_pos.store(pos.index(), Ordering::Relaxed);
    }

    pub fn set_eq_low_db(&self, db: f32) {
        debug!(target: "eq", "low={db}dB");
        self.eq_low_db.set(db);
    }

    pub fn set_eq_mid_db(&self, db: f32) {
        debug!(target: "eq", "mid={db}dB");
        self.eq_mid_db.set(db);
    }

    pub fn set_eq_high_db(&self, db: f32) {
        debug!(target: "eq", "high={db}dB");
        self.eq_high_db.set(db);
    }

    pub fn set_eq_hp_freq(&self, hz: f32) {
        debug!(target: "eq", "high-pass={hz}Hz");
        self.eq_hp_freq.set(hz);
    }

    pub fn set_eq_lp_freq(&self, hz: f32) {
        debug!(target: "eq", "low-pass={hz}Hz");
        self.eq_lp_freq.set(hz);
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.tuner_shutdown.store(true, Ordering::Relaxed);
    }
}
