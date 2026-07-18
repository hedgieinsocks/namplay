//! JACK audio engine: owns the client and hands parameter changes to the
//! real-time processor via atomics and channels.

mod dsp;
mod ir;
mod processor;

use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc, Arc,
};

use jack::{AudioIn, AudioOut, Client, ClientOptions};
use log::{debug, error, warn};
use nam_rs::{Model, NamModel};

use dsp::{db_to_gain, AtomicF32, Eq, NoiseGate};
use ir::IrConvolvers;
use processor::NamProcessor;

/// Where the EQ sits in the signal chain. The discriminants match both the
/// order of the EQ position dropdown in `window.blp` and the values stored
/// in the processor's atomic.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum EqPosition {
    PrePedal = 0,
    PreAmp = 1,
    PostIr = 2,
}

impl EqPosition {
    pub fn from_index(index: u32) -> Self {
        match index {
            0 => Self::PrePedal,
            2 => Self::PostIr,
            _ => Self::PreAmp,
        }
    }

    pub fn from_setting(setting: &str) -> Self {
        match setting {
            "pre-pedal" => Self::PrePedal,
            "post-ir" => Self::PostIr,
            _ => Self::PreAmp,
        }
    }

    pub fn index(self) -> u32 {
        self as u32
    }

    pub fn setting(self) -> &'static str {
        match self {
            Self::PrePedal => "pre-pedal",
            Self::PreAmp => "pre-amp",
            Self::PostIr => "post-ir",
        }
    }
}

pub struct InitialParams {
    pub gate_enabled: bool,
    pub gate_threshold_db: f32,
    pub pedal_profile_path: Option<String>,
    pub pedal_in_gain_db: f32,
    pub pedal_out_gain_db: f32,
    pub amp_profile_path: Option<String>,
    pub amp_in_gain_db: f32,
    pub amp_out_gain_db: f32,
    pub ir_path: Option<String>,
    pub ir_level_db: f32,
    pub eq_enabled: bool,
    pub eq_pos: EqPosition,
    pub eq_low_db: f32,
    pub eq_mid_db: f32,
    pub eq_high_db: f32,
    pub eq_hp_freq: f32,
    pub eq_lp_freq: f32,
}

pub struct AudioEngine {
    gate_enabled: Arc<AtomicBool>,
    gate_threshold_db: Arc<AtomicF32>,
    pedal_profile_tx: mpsc::Sender<Option<Model>>,
    pedal_in_gain: Arc<AtomicF32>,
    pedal_out_gain: Arc<AtomicF32>,
    amp_profile_tx: mpsc::Sender<Option<Model>>,
    amp_in_gain: Arc<AtomicF32>,
    amp_out_gain: Arc<AtomicF32>,
    ir_tx: mpsc::Sender<Option<IrConvolvers>>,
    ir_level: Arc<AtomicF32>,
    eq_enabled: Arc<AtomicBool>,
    eq_pos: Arc<AtomicU32>,
    eq_low_db: Arc<AtomicF32>,
    eq_mid_db: Arc<AtomicF32>,
    eq_high_db: Arc<AtomicF32>,
    eq_hp_freq: Arc<AtomicF32>,
    eq_lp_freq: Arc<AtomicF32>,
    _client: jack::AsyncClient<(), NamProcessor>,
    sample_rate: u32,
    block_size: usize,
}

impl AudioEngine {
    pub fn new(params: InitialParams) -> Result<Self, String> {
        let (client, _status) = Client::new("namplay", ClientOptions::NO_START_SERVER)
            .map_err(|e| format!("JACK connection failed: {e}"))?;

        let sample_rate = client.sample_rate();
        let block_size = client.buffer_size() as usize;
        debug!("JACK: state=connected sample_rate={sample_rate} block_size={block_size}");

        let in_port = client
            .register_port("input", AudioIn::default())
            .map_err(|e| format!("register input port: {e}"))?;
        let out_port_l = client
            .register_port("output_l", AudioOut::default())
            .map_err(|e| format!("register output_l port: {e}"))?;
        let out_port_r = client
            .register_port("output_r", AudioOut::default())
            .map_err(|e| format!("register output_r port: {e}"))?;

        let (pedal_profile_tx, pedal_profile_rx) = mpsc::channel();
        let (amp_profile_tx, amp_profile_rx) = mpsc::channel();
        let (ir_tx, ir_rx) = mpsc::channel::<Option<IrConvolvers>>();

        let gate_enabled = Arc::new(AtomicBool::new(params.gate_enabled));
        let gate_threshold_db = Arc::new(AtomicF32::new(params.gate_threshold_db));
        let pedal_in_gain = Arc::new(AtomicF32::new(db_to_gain(params.pedal_in_gain_db)));
        let pedal_out_gain = Arc::new(AtomicF32::new(db_to_gain(params.pedal_out_gain_db)));
        let amp_in_gain = Arc::new(AtomicF32::new(db_to_gain(params.amp_in_gain_db)));
        let amp_out_gain = Arc::new(AtomicF32::new(db_to_gain(params.amp_out_gain_db)));
        let ir_level = Arc::new(AtomicF32::new(db_to_gain(params.ir_level_db)));
        let eq_enabled = Arc::new(AtomicBool::new(params.eq_enabled));
        let eq_pos = Arc::new(AtomicU32::new(params.eq_pos.index()));
        let eq_low_db = Arc::new(AtomicF32::new(params.eq_low_db));
        let eq_mid_db = Arc::new(AtomicF32::new(params.eq_mid_db));
        let eq_high_db = Arc::new(AtomicF32::new(params.eq_high_db));
        let eq_hp_freq = Arc::new(AtomicF32::new(params.eq_hp_freq));
        let eq_lp_freq = Arc::new(AtomicF32::new(params.eq_lp_freq));

        debug!(
            "GATE: state={} threshold={}dB",
            if params.gate_enabled { "on" } else { "off" },
            params.gate_threshold_db
        );
        debug!(
            "PEDAL: in={}dB out={}dB",
            params.pedal_in_gain_db, params.pedal_out_gain_db
        );
        debug!(
            "AMP: in={}dB out={}dB",
            params.amp_in_gain_db, params.amp_out_gain_db
        );
        debug!("IR: level={}dB", params.ir_level_db);
        debug!(
            "EQ: state={} position={} low={}dB mid={}dB high={}dB hp={}Hz lp={}Hz",
            if params.eq_enabled { "on" } else { "off" },
            params.eq_pos.setting(),
            params.eq_low_db,
            params.eq_mid_db,
            params.eq_high_db,
            params.eq_hp_freq,
            params.eq_lp_freq,
        );

        let make_eq = || {
            Eq::new(
                params.eq_low_db,
                params.eq_mid_db,
                params.eq_high_db,
                params.eq_hp_freq,
                params.eq_lp_freq,
                sample_rate as f32,
            )
        };

        let processor = NamProcessor {
            gate_enabled: Arc::clone(&gate_enabled),
            gate_threshold_db: Arc::clone(&gate_threshold_db),
            noise_gate: NoiseGate::new(params.gate_threshold_db, sample_rate),
            pedal_profile_rx,
            current_pedal_profile: None,
            pedal_in_gain: Arc::clone(&pedal_in_gain),
            pedal_out_gain: Arc::clone(&pedal_out_gain),
            amp_profile_rx,
            current_amp_profile: None,
            amp_in_gain: Arc::clone(&amp_in_gain),
            amp_out_gain: Arc::clone(&amp_out_gain),
            ir_rx,
            current_ir_l: None,
            current_ir_r: None,
            ir_level: Arc::clone(&ir_level),
            eq_enabled: Arc::clone(&eq_enabled),
            eq_pos: Arc::clone(&eq_pos),
            eq_low_db: Arc::clone(&eq_low_db),
            eq_mid_db: Arc::clone(&eq_mid_db),
            eq_high_db: Arc::clone(&eq_high_db),
            eq_hp_freq: Arc::clone(&eq_hp_freq),
            eq_lp_freq: Arc::clone(&eq_lp_freq),
            eq_l: make_eq(),
            eq_r: make_eq(),
            conv_buf: vec![0.0f32; block_size],
            in_port,
            out_port_l,
            out_port_r,
        };

        let active_client = client
            .activate_async((), processor)
            .map_err(|e| format!("JACK: activation failed: {e}"))?;

        let engine = AudioEngine {
            gate_enabled,
            gate_threshold_db,
            pedal_profile_tx,
            pedal_in_gain,
            pedal_out_gain,
            amp_profile_tx,
            amp_in_gain,
            amp_out_gain,
            ir_tx,
            ir_level,
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
        };

        engine.load_pedal_profile(params.pedal_profile_path);
        engine.load_amp_profile(params.amp_profile_path);
        engine.load_ir(params.ir_path);

        Ok(engine)
    }

    pub fn set_gate_enabled(&self, enabled: bool) {
        debug!("GATE: state={}", if enabled { "on" } else { "off" });
        self.gate_enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn set_gate_threshold_db(&self, db: f32) {
        debug!("GATE: threshold={db}dB");
        self.gate_threshold_db.set(db);
    }

    pub fn load_pedal_profile(&self, path: Option<String>) {
        load_profile(
            "PEDAL",
            self.pedal_profile_tx.clone(),
            path,
            self.sample_rate,
        );
    }

    pub fn set_pedal_in_gain_db(&self, db: f32) {
        debug!("PEDAL: in={db}dB");
        self.pedal_in_gain.set(db_to_gain(db));
    }

    pub fn set_pedal_out_gain_db(&self, db: f32) {
        debug!("PEDAL: out={db}dB");
        self.pedal_out_gain.set(db_to_gain(db));
    }

    pub fn load_amp_profile(&self, path: Option<String>) {
        load_profile("AMP", self.amp_profile_tx.clone(), path, self.sample_rate);
    }

    pub fn set_amp_in_gain_db(&self, db: f32) {
        debug!("AMP: in={db}dB");
        self.amp_in_gain.set(db_to_gain(db));
    }

    pub fn set_amp_out_gain_db(&self, db: f32) {
        debug!("AMP: out={db}dB");
        self.amp_out_gain.set(db_to_gain(db));
    }

    pub fn load_ir(&self, path: Option<String>) {
        let tx = self.ir_tx.clone();
        let sample_rate = self.sample_rate;
        let block_size = self.block_size;
        std::thread::spawn(move || {
            let convolvers = match path {
                None => {
                    debug!("IR: file cleared");
                    None
                }
                Some(p) => {
                    debug!("IR: loading file: {p}");
                    let result = ir::load(&p, sample_rate, block_size);
                    if result.is_some() {
                        debug!("IR: file loaded: {p}");
                    } else {
                        error!("IR: failed to load file: {p}");
                    }
                    result
                }
            };
            let _ = tx.send(convolvers);
        });
    }

    pub fn set_ir_level_db(&self, db: f32) {
        debug!("IR: level={db}dB");
        self.ir_level.set(db_to_gain(db));
    }

    pub fn set_eq_enabled(&self, enabled: bool) {
        debug!("EQ: state={}", if enabled { "on" } else { "off" });
        self.eq_enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn set_eq_pos(&self, pos: EqPosition) {
        debug!("EQ: position={}", pos.setting());
        self.eq_pos.store(pos.index(), Ordering::Relaxed);
    }

    pub fn set_eq_low_db(&self, db: f32) {
        debug!("EQ: low={db}dB");
        self.eq_low_db.set(db);
    }

    pub fn set_eq_mid_db(&self, db: f32) {
        debug!("EQ: mid={db}dB");
        self.eq_mid_db.set(db);
    }

    pub fn set_eq_high_db(&self, db: f32) {
        debug!("EQ: high={db}dB");
        self.eq_high_db.set(db);
    }

    pub fn set_eq_hp_freq(&self, hz: f32) {
        debug!("EQ: high-pass={hz}Hz");
        self.eq_hp_freq.set(hz);
    }

    pub fn set_eq_lp_freq(&self, hz: f32) {
        debug!("EQ: low-pass={hz}Hz");
        self.eq_lp_freq.set(hz);
    }
}

fn load_profile(
    label: &'static str,
    tx: mpsc::Sender<Option<Model>>,
    path: Option<String>,
    sample_rate: u32,
) {
    std::thread::spawn(move || {
        let profile = match path {
            None => {
                debug!("{label}: profile cleared");
                None
            }
            Some(p) => {
                debug!("{label}: loading profile: {p}");
                let model = NamModel::from_file(&p).ok().and_then(|nm| {
                    let model_sr = nm.expected_sample_rate() as u32;
                    if model_sr != sample_rate {
                        warn!("{label}: profile sample rate {model_sr} != JACK sample rate {sample_rate}");
                    }
                    Model::from_nam(&nm).ok()
                });
                if model.is_some() {
                    debug!("{label}: profile loaded: {p}");
                } else {
                    error!("{label}: failed to load profile: {p}");
                }
                model
            }
        };
        let _ = tx.send(profile);
    });
}
