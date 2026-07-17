mod dsp;
mod ir;
mod processor;

use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc, Arc,
};

use fft_convolver::FFTConvolver;
use jack::{AudioIn, AudioOut, Client, ClientOptions};
use log::{debug, error, warn};
use nam_rs::{Model, NamModel};

use dsp::{db_to_gain, AtomicF32, Eq, NoiseGate};
use ir::load_wav_channels;
use processor::NamProcessor;

pub struct InitialParams {
    pub gate_enabled: bool,
    pub gate_threshold_db: f32,
    pub pedal_profile_path: Option<String>,
    pub pedal_in_gain_db: f32,
    pub pedal_out_gain_db: f32,
    pub amp_in_gain_db: f32,
    pub amp_out_gain_db: f32,
    pub amp_profile_path: Option<String>,
    pub ir_path: Option<String>,
    pub ir_level_db: f32,
    pub eq_enabled: bool,
    pub eq_pos: u32,
    pub eq_low_db: f32,
    pub eq_mid_db: f32,
    pub eq_high_db: f32,
    pub eq_hp_freq: f32,
    pub eq_lp_freq: f32,
}

pub struct AudioEngine {
    _client: jack::AsyncClient<(), NamProcessor>,
    pedal_profile_tx: mpsc::Sender<Option<Model>>,
    pedal_in_gain: Arc<AtomicF32>,
    pedal_out_gain: Arc<AtomicF32>,
    amp_profile_tx: mpsc::Sender<Option<Model>>,
    ir_tx: mpsc::Sender<Option<(FFTConvolver<f32>, Option<FFTConvolver<f32>>)>>,
    amp_in_gain: Arc<AtomicF32>,
    amp_out_gain: Arc<AtomicF32>,
    ir_level: Arc<AtomicF32>,
    gate_enabled: Arc<AtomicBool>,
    gate_threshold_db: Arc<AtomicF32>,
    eq_enabled: Arc<AtomicBool>,
    eq_pos: Arc<AtomicU32>,
    eq_low_db: Arc<AtomicF32>,
    eq_mid_db: Arc<AtomicF32>,
    eq_high_db: Arc<AtomicF32>,
    eq_hp_freq: Arc<AtomicF32>,
    eq_lp_freq: Arc<AtomicF32>,
    sample_rate: u32,
    block_size: usize,
}

impl AudioEngine {
    pub fn new(params: InitialParams) -> Result<Self, String> {
        let (client, _status) = Client::new("namplay", ClientOptions::NO_START_SERVER)
            .map_err(|e| format!("JACK connection failed: {e}"))?;

        let sample_rate = client.sample_rate() as u32;
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
        let (ir_tx, ir_rx) =
            mpsc::channel::<Option<(FFTConvolver<f32>, Option<FFTConvolver<f32>>)>>();

        let pedal_in_gain = Arc::new(AtomicF32::new(db_to_gain(params.pedal_in_gain_db)));
        let pedal_out_gain = Arc::new(AtomicF32::new(db_to_gain(params.pedal_out_gain_db)));
        let amp_in_gain = Arc::new(AtomicF32::new(db_to_gain(params.amp_in_gain_db)));
        let amp_out_gain = Arc::new(AtomicF32::new(db_to_gain(params.amp_out_gain_db)));
        let ir_level = Arc::new(AtomicF32::new(db_to_gain(params.ir_level_db)));
        let gate_enabled = Arc::new(AtomicBool::new(params.gate_enabled));
        let gate_threshold_db = Arc::new(AtomicF32::new(params.gate_threshold_db));
        let eq_enabled = Arc::new(AtomicBool::new(params.eq_enabled));
        let eq_pos = Arc::new(AtomicU32::new(params.eq_pos));
        let eq_low_db = Arc::new(AtomicF32::new(params.eq_low_db));
        let eq_mid_db = Arc::new(AtomicF32::new(params.eq_mid_db));
        let eq_high_db = Arc::new(AtomicF32::new(params.eq_high_db));
        let eq_hp_freq = Arc::new(AtomicF32::new(params.eq_hp_freq));
        let eq_lp_freq = Arc::new(AtomicF32::new(params.eq_lp_freq));

        debug!(
            "pedal: in={}dB out={}dB",
            params.pedal_in_gain_db, params.pedal_out_gain_db
        );
        debug!(
            "amp: in={}dB out={}dB",
            params.amp_in_gain_db, params.amp_out_gain_db
        );
        debug!("IR: level={}dB", params.ir_level_db);
        debug!(
            "noise gate: state={} threshold={}dB",
            if params.gate_enabled { "on" } else { "off" },
            params.gate_threshold_db
        );
        debug!(
            "EQ: state={} position={} low={}dB mid={}dB high={}dB hp={}Hz lp={}Hz",
            if params.eq_enabled { "on" } else { "off" },
            match params.eq_pos {
                0 => "pre-pedal",
                2 => "post-IR",
                _ => "pre-amp",
            },
            params.eq_low_db,
            params.eq_mid_db,
            params.eq_high_db,
            params.eq_hp_freq,
            params.eq_lp_freq,
        );

        let initial_gate = params
            .gate_enabled
            .then(|| NoiseGate::new(params.gate_threshold_db, sample_rate));

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
        let initial_eq_l = params.eq_enabled.then(make_eq);
        let initial_eq_r = params.eq_enabled.then(make_eq);

        let processor = NamProcessor {
            pedal_profile_rx,
            current_pedal_profile: None,
            pedal_in_gain: Arc::clone(&pedal_in_gain),
            pedal_out_gain: Arc::clone(&pedal_out_gain),
            amp_profile_rx,
            current_profile: None,
            ir_rx,
            current_ir_l: None,
            current_ir_r: None,
            conv_buf: vec![0.0f32; block_size],
            noise_gate: initial_gate,
            amp_in_gain: Arc::clone(&amp_in_gain),
            amp_out_gain: Arc::clone(&amp_out_gain),
            ir_level: Arc::clone(&ir_level),
            gate_enabled: Arc::clone(&gate_enabled),
            gate_threshold_db: Arc::clone(&gate_threshold_db),
            last_gate_enabled: params.gate_enabled,
            last_gate_threshold_db: params.gate_threshold_db,
            eq_l: initial_eq_l,
            eq_r: initial_eq_r,
            eq_enabled: Arc::clone(&eq_enabled),
            eq_pos: Arc::clone(&eq_pos),
            eq_low_db: Arc::clone(&eq_low_db),
            eq_mid_db: Arc::clone(&eq_mid_db),
            eq_high_db: Arc::clone(&eq_high_db),
            eq_hp_freq: Arc::clone(&eq_hp_freq),
            eq_lp_freq: Arc::clone(&eq_lp_freq),
            last_eq_enabled: params.eq_enabled,
            last_eq_pos: params.eq_pos,
            last_eq_low_db: params.eq_low_db,
            last_eq_mid_db: params.eq_mid_db,
            last_eq_high_db: params.eq_high_db,
            last_eq_hp_freq: params.eq_hp_freq,
            last_eq_lp_freq: params.eq_lp_freq,
            sample_rate,
            in_port,
            out_port_l,
            out_port_r,
        };

        let active_client = client
            .activate_async((), processor)
            .map_err(|e| format!("JACK activation failed: {e}"))?;

        let engine = AudioEngine {
            _client: active_client,
            pedal_profile_tx,
            pedal_in_gain,
            pedal_out_gain,
            amp_profile_tx,
            ir_tx,
            amp_in_gain,
            amp_out_gain,
            ir_level,
            gate_enabled,
            gate_threshold_db,
            eq_enabled,
            eq_pos,
            eq_low_db,
            eq_mid_db,
            eq_high_db,
            eq_hp_freq,
            eq_lp_freq,
            sample_rate,
            block_size,
        };

        engine.load_pedal_profile(params.pedal_profile_path);
        engine.load_amp_profile(params.amp_profile_path);
        engine.load_ir(params.ir_path);

        Ok(engine)
    }

    pub fn load_pedal_profile(&self, path: Option<String>) {
        let tx = self.pedal_profile_tx.clone();
        let sample_rate = self.sample_rate;
        std::thread::spawn(move || {
            let profile = match path {
                None => {
                    debug!("pedal profile cleared");
                    None
                }
                Some(p) => {
                    debug!("loading pedal profile: {p}");
                    let m = match NamModel::from_file(&p) {
                        Ok(nm) => {
                            let model_sr = nm.expected_sample_rate() as u32;
                            if model_sr != sample_rate {
                                warn!("pedal profile sample rate {model_sr} != JACK rate {sample_rate}, pitch may differ");
                            }
                            Model::from_nam(&nm).ok()
                        }
                        Err(_) => None,
                    };
                    if m.is_some() {
                        debug!("pedal profile loaded: {p}");
                    } else {
                        error!("failed to load pedal profile: {p}");
                    }
                    m
                }
            };
            let _ = tx.send(profile);
        });
    }

    pub fn set_pedal_in_gain_db(&self, db: f32) {
        debug!("pedal in={db}dB");
        self.pedal_in_gain.set(db_to_gain(db));
    }

    pub fn set_pedal_out_gain_db(&self, db: f32) {
        debug!("pedal out={db}dB");
        self.pedal_out_gain.set(db_to_gain(db));
    }

    pub fn load_amp_profile(&self, path: Option<String>) {
        let tx = self.amp_profile_tx.clone();
        let sample_rate = self.sample_rate;
        std::thread::spawn(move || {
            let profile = match path {
                None => {
                    debug!("amp profile cleared");
                    None
                }
                Some(p) => {
                    debug!("loading amp profile: {p}");
                    let m = match NamModel::from_file(&p) {
                        Ok(nm) => {
                            let model_sr = nm.expected_sample_rate() as u32;
                            if model_sr != sample_rate {
                                warn!("amp profile sample rate {model_sr} != JACK rate {sample_rate}, pitch may differ");
                            }
                            Model::from_nam(&nm).ok()
                        }
                        Err(_) => None,
                    };
                    if m.is_some() {
                        debug!("amp profile loaded: {p}");
                    } else {
                        error!("failed to load amp profile: {p}");
                    }
                    m
                }
            };
            let _ = tx.send(profile);
        });
    }

    pub fn load_ir(&self, path: Option<String>) {
        let tx = self.ir_tx.clone();
        let sample_rate = self.sample_rate;
        let block_size = self.block_size;
        std::thread::spawn(move || {
            let conv = match path {
                None => {
                    debug!("IR cleared");
                    None
                }
                Some(p) => {
                    debug!("loading IR: {p}");
                    let result =
                        load_wav_channels(&p, sample_rate).and_then(|(l_samples, r_samples)| {
                            let mut cl = FFTConvolver::<f32>::default();
                            cl.init(block_size, &l_samples).ok()?;
                            let cr = r_samples.and_then(|r| {
                                let mut c = FFTConvolver::<f32>::default();
                                c.init(block_size, &r).ok().map(|_| c)
                            });
                            Some((cl, cr))
                        });
                    if result.is_some() {
                        debug!("IR loaded: {p}");
                    } else {
                        error!("failed to load IR: {p}");
                    }
                    result
                }
            };
            let _ = tx.send(conv);
        });
    }

    pub fn set_ir_level_db(&self, db: f32) {
        debug!("IR: level={db}dB");
        self.ir_level.set(db_to_gain(db));
    }

    pub fn set_amp_in_gain_db(&self, db: f32) {
        debug!("amp: in={db}dB");
        self.amp_in_gain.set(db_to_gain(db));
    }

    pub fn set_amp_out_gain_db(&self, db: f32) {
        debug!("amp: out={db}dB");
        self.amp_out_gain.set(db_to_gain(db));
    }

    pub fn set_gate_enabled(&self, enabled: bool) {
        debug!("noise gate: state={}", if enabled { "on" } else { "off" });
        self.gate_enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn set_gate_threshold_db(&self, db: f32) {
        debug!("noise gate: threshold={db}dB");
        self.gate_threshold_db.set(db);
    }

    pub fn set_eq_enabled(&self, enabled: bool) {
        debug!("EQ: state={}", if enabled { "on" } else { "off" });
        self.eq_enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn set_eq_pos(&self, pos: u32) {
        let name = match pos {
            0 => "pre-pedal",
            2 => "post-IR",
            _ => "pre-amp",
        };
        debug!("EQ: position={name}");
        self.eq_pos.store(pos, Ordering::Relaxed);
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
