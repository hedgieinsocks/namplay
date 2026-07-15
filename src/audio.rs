use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc, Arc,
};

use log::{debug, error, warn};

use fft_convolver::FFTConvolver;

const EQ_LOW_FREQ: f32 = 150.0;
const EQ_MID_FREQ: f32 = 425.0;
const EQ_HIGH_FREQ: f32 = 1800.0;
const EQ_MID_Q_CUT: f32 = 1.5;
const EQ_MID_Q_BOOST: f32 = 0.7;

use jack::{AudioIn, AudioOut, Client, ClientOptions, Control, ProcessHandler, ProcessScope};
use nam_rs::{Model, NamModel};

struct AtomicF32(AtomicU32);

impl AtomicF32 {
    fn new(val: f32) -> Self {
        AtomicF32(AtomicU32::new(val.to_bits()))
    }
    fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    fn set(&self, val: f32) {
        self.0.store(val.to_bits(), Ordering::Relaxed)
    }
}

struct NoiseGate {
    open_threshold: f32,
    close_threshold: f32,
    attack_coeff: f32,
    release_coeff: f32,
    hold_samples: u32,
    envelope: f32,
    gain: f32,
    gate_open: bool,
    hold_counter: u32,
}

impl NoiseGate {
    fn new(threshold_db: f32, sample_rate: u32) -> Self {
        let sr = sample_rate as f32;
        NoiseGate {
            open_threshold: db_to_gain(threshold_db),
            close_threshold: db_to_gain(threshold_db - 6.0),
            attack_coeff: (-1.0_f32 / (0.001 * sr)).exp(),
            release_coeff: (-1.0_f32 / (0.100 * sr)).exp(),
            hold_samples: (0.050 * sr) as u32,
            envelope: 0.0,
            gain: 0.0,
            gate_open: false,
            hold_counter: 0,
        }
    }

    fn process_sample(&mut self, sample: f32) -> f32 {
        let abs = sample.abs();
        let env_coeff = if abs > self.envelope {
            self.attack_coeff
        } else {
            self.release_coeff
        };
        self.envelope = env_coeff * self.envelope + (1.0 - env_coeff) * abs;

        if self.gate_open {
            if self.envelope > self.open_threshold {
                self.hold_counter = self.hold_samples;
            } else if self.hold_counter > 0 {
                self.hold_counter -= 1;
            } else if self.envelope < self.close_threshold {
                self.gate_open = false;
            }
        } else if self.envelope > self.open_threshold {
            self.gate_open = true;
            self.hold_counter = self.hold_samples;
        }

        let target = if self.gate_open { 1.0_f32 } else { 0.0 };
        let gain_coeff = if target > self.gain {
            self.attack_coeff
        } else {
            self.release_coeff
        };
        self.gain = gain_coeff * self.gain + (1.0 - gain_coeff) * target;

        sample * self.gain
    }
}

struct BiquadFilter {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl BiquadFilter {
    fn passthrough() -> Self {
        BiquadFilter {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    fn set_low_shelf(&mut self, gain_db: f32, freq: f32, sample_rate: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / 2.0 * 2.0f32.sqrt(); // shelf slope S=1
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
        let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha;
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    fn set_peaking(&mut self, gain_db: f32, freq: f32, q: f32, sample_rate: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q);
        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    fn set_highpass(&mut self, freq: f32, sample_rate: f32) {
        let w0 = 2.0 * std::f32::consts::PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * 0.707f32);
        let b0 = (1.0 + cos_w0) / 2.0;
        let b1 = -(1.0 + cos_w0);
        let b2 = (1.0 + cos_w0) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    fn set_lowpass(&mut self, freq: f32, sample_rate: f32) {
        let w0 = 2.0 * std::f32::consts::PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * 0.707f32);
        let b0 = (1.0 - cos_w0) / 2.0;
        let b1 = 1.0 - cos_w0;
        let b2 = (1.0 - cos_w0) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    fn set_high_shelf(&mut self, gain_db: f32, freq: f32, sample_rate: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / 2.0 * 2.0f32.sqrt(); // shelf slope S=1
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
        let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha;
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }
}

struct Eq {
    hp: BiquadFilter,
    low: BiquadFilter,
    mid: BiquadFilter,
    high: BiquadFilter,
    lp: BiquadFilter,
    sample_rate: f32,
}

impl Eq {
    fn new(
        low_db: f32,
        mid_db: f32,
        high_db: f32,
        hp_freq: f32,
        lp_freq: f32,
        sample_rate: f32,
    ) -> Self {
        let mut eq = Eq {
            hp: BiquadFilter::passthrough(),
            low: BiquadFilter::passthrough(),
            mid: BiquadFilter::passthrough(),
            high: BiquadFilter::passthrough(),
            lp: BiquadFilter::passthrough(),
            sample_rate,
        };
        eq.update(low_db, mid_db, high_db, hp_freq, lp_freq);
        eq
    }

    fn update(&mut self, low_db: f32, mid_db: f32, high_db: f32, hp_freq: f32, lp_freq: f32) {
        self.hp.set_highpass(hp_freq, self.sample_rate);
        self.low
            .set_low_shelf(low_db, EQ_LOW_FREQ, self.sample_rate);
        let mid_q = if mid_db < 0.0 {
            EQ_MID_Q_CUT
        } else {
            EQ_MID_Q_BOOST
        };
        self.mid
            .set_peaking(mid_db, EQ_MID_FREQ, mid_q, self.sample_rate);
        self.high
            .set_high_shelf(high_db, EQ_HIGH_FREQ, self.sample_rate);
        self.lp.set_lowpass(lp_freq, self.sample_rate);
    }

    fn process_sample(&mut self, x: f32) -> f32 {
        self.lp.process(
            self.high
                .process(self.mid.process(self.low.process(self.hp.process(x)))),
        )
    }
}

struct NamProcessor {
    pedal_profile_rx: mpsc::Receiver<Option<Model>>,
    current_pedal_profile: Option<Model>,
    pedal_in_gain: Arc<AtomicF32>,
    pedal_out_gain: Arc<AtomicF32>,
    profile_rx: mpsc::Receiver<Option<Model>>,
    current_profile: Option<Model>,
    ir_rx: mpsc::Receiver<Option<(FFTConvolver<f32>, Option<FFTConvolver<f32>>)>>,
    current_ir_l: Option<FFTConvolver<f32>>,
    current_ir_r: Option<FFTConvolver<f32>>,
    conv_buf: Vec<f32>,
    noise_gate: Option<NoiseGate>,
    in_gain: Arc<AtomicF32>,
    out_gain: Arc<AtomicF32>,
    ir_level: Arc<AtomicF32>,
    gate_enabled: Arc<AtomicBool>,
    gate_threshold_db: Arc<AtomicF32>,
    last_gate_enabled: bool,
    last_gate_threshold_db: f32,
    eq_l: Option<Eq>,
    eq_r: Option<Eq>,
    eq_enabled: Arc<AtomicBool>,
    eq_pos: Arc<AtomicU32>,
    eq_low_db: Arc<AtomicF32>,
    eq_mid_db: Arc<AtomicF32>,
    eq_high_db: Arc<AtomicF32>,
    eq_hp_freq: Arc<AtomicF32>,
    eq_lp_freq: Arc<AtomicF32>,
    last_eq_enabled: bool,
    last_eq_pos: u32,
    last_eq_low_db: f32,
    last_eq_mid_db: f32,
    last_eq_high_db: f32,
    last_eq_hp_freq: f32,
    last_eq_lp_freq: f32,
    sample_rate: u32,
    in_port: jack::Port<AudioIn>,
    out_port_l: jack::Port<AudioOut>,
    out_port_r: jack::Port<AudioOut>,
}

impl ProcessHandler for NamProcessor {
    fn process(&mut self, _: &Client, ps: &ProcessScope) -> Control {
        while let Ok(new_profile) = self.pedal_profile_rx.try_recv() {
            self.current_pedal_profile = new_profile;
        }
        while let Ok(new_profile) = self.profile_rx.try_recv() {
            self.current_profile = new_profile;
        }
        while let Ok(new_ir) = self.ir_rx.try_recv() {
            match new_ir {
                Some((l, r)) => {
                    self.current_ir_l = Some(l);
                    self.current_ir_r = r;
                }
                None => {
                    self.current_ir_l = None;
                    self.current_ir_r = None;
                }
            }
        }

        let gate_enabled = self.gate_enabled.load(Ordering::Relaxed);
        let gate_threshold_db = self.gate_threshold_db.get();
        if gate_enabled != self.last_gate_enabled
            || gate_threshold_db != self.last_gate_threshold_db
        {
            self.last_gate_enabled = gate_enabled;
            self.last_gate_threshold_db = gate_threshold_db;
            self.noise_gate =
                gate_enabled.then(|| NoiseGate::new(gate_threshold_db, self.sample_rate));
        }

        let pedal_in_gain = self.pedal_in_gain.get();
        let pedal_out_gain = self.pedal_out_gain.get();
        let in_gain = self.in_gain.get();
        let out_gain = self.out_gain.get();
        let ir_level = self.ir_level.get();

        let eq_enabled = self.eq_enabled.load(Ordering::Relaxed);
        let eq_pos = self.eq_pos.load(Ordering::Relaxed);
        let eq_low_db = self.eq_low_db.get();
        let eq_mid_db = self.eq_mid_db.get();
        let eq_high_db = self.eq_high_db.get();
        let eq_hp_freq = self.eq_hp_freq.get();
        let eq_lp_freq = self.eq_lp_freq.get();
        let eq_params_changed = eq_low_db != self.last_eq_low_db
            || eq_mid_db != self.last_eq_mid_db
            || eq_high_db != self.last_eq_high_db
            || eq_hp_freq != self.last_eq_hp_freq
            || eq_lp_freq != self.last_eq_lp_freq
            || eq_pos != self.last_eq_pos;
        if eq_enabled != self.last_eq_enabled || (eq_enabled && eq_params_changed) {
            self.last_eq_enabled = eq_enabled;
            self.last_eq_pos = eq_pos;
            self.last_eq_low_db = eq_low_db;
            self.last_eq_mid_db = eq_mid_db;
            self.last_eq_high_db = eq_high_db;
            self.last_eq_hp_freq = eq_hp_freq;
            self.last_eq_lp_freq = eq_lp_freq;
            if !eq_enabled {
                self.eq_l = None;
                self.eq_r = None;
            } else if let Some(eq) = &mut self.eq_l {
                eq.update(eq_low_db, eq_mid_db, eq_high_db, eq_hp_freq, eq_lp_freq);
                if let Some(eq) = &mut self.eq_r {
                    eq.update(eq_low_db, eq_mid_db, eq_high_db, eq_hp_freq, eq_lp_freq);
                }
            } else {
                self.eq_l = Some(Eq::new(
                    eq_low_db,
                    eq_mid_db,
                    eq_high_db,
                    eq_hp_freq,
                    eq_lp_freq,
                    self.sample_rate as f32,
                ));
                self.eq_r = Some(Eq::new(
                    eq_low_db,
                    eq_mid_db,
                    eq_high_db,
                    eq_hp_freq,
                    eq_lp_freq,
                    self.sample_rate as f32,
                ));
            }
        }

        let input = self.in_port.as_slice(ps);
        let out_l = self.out_port_l.as_mut_slice(ps);
        let out_r = self.out_port_r.as_mut_slice(ps);

        if self.current_pedal_profile.is_none() && self.current_profile.is_none() {
            for s in out_l.iter_mut() {
                *s = 0.0;
            }
            for s in out_r.iter_mut() {
                *s = 0.0;
            }
        } else {
            // Gate + optional pre-pedal EQ
            for (o, &i) in out_l.iter_mut().zip(input) {
                let mut s = match &mut self.noise_gate {
                    Some(g) => g.process_sample(i),
                    None => i,
                };
                if eq_pos == 0 {
                    if let Some(eq) = &mut self.eq_l {
                        s = eq.process_sample(s);
                    }
                }
                *o = s;
            }
            if let Some(pedal) = &mut self.current_pedal_profile {
                for s in out_l.iter_mut() {
                    *s *= pedal_in_gain;
                }
                pedal.process_buffer(out_l);
                for s in out_l.iter_mut() {
                    *s *= pedal_out_gain;
                }
            }
            // Optional pre-amp EQ
            if eq_pos == 1 {
                if let Some(eq) = &mut self.eq_l {
                    for s in out_l.iter_mut() {
                        *s = eq.process_sample(*s);
                    }
                }
            }
            if let Some(amp) = &mut self.current_profile {
                for s in out_l.iter_mut() {
                    *s *= in_gain;
                }
                amp.process_buffer(out_l);
            }
            let n = out_l.len().min(self.conv_buf.len());
            let stereo_ir = if let Some(ir_l) = &mut self.current_ir_l {
                self.conv_buf[..n].copy_from_slice(&out_l[..n]);
                let _ = ir_l.process(&self.conv_buf[..n], &mut out_l[..n]);
                for s in out_l[..n].iter_mut() {
                    *s *= ir_level;
                }
                if let Some(ir_r) = &mut self.current_ir_r {
                    let _ = ir_r.process(&self.conv_buf[..n], &mut out_r[..n]);
                    for s in out_r[..n].iter_mut() {
                        *s *= ir_level;
                    }
                    true
                } else {
                    false
                }
            } else {
                false
            };
            // Optional post-IR EQ
            if eq_pos == 2 {
                if let Some(eq) = &mut self.eq_l {
                    for s in out_l.iter_mut() {
                        *s = eq.process_sample(*s);
                    }
                }
            }
            for s in out_l.iter_mut() {
                *s *= out_gain;
            }
            if stereo_ir {
                if eq_pos == 2 {
                    if let Some(eq) = &mut self.eq_r {
                        for s in out_r.iter_mut() {
                            *s = eq.process_sample(*s);
                        }
                    }
                }
                for s in out_r.iter_mut() {
                    *s *= out_gain;
                }
            } else {
                out_r.copy_from_slice(out_l);
            }
        }

        Control::Continue
    }
}

pub struct InitialParams {
    pub gate_enabled: bool,
    pub gate_threshold_db: f32,
    pub pedal_path: Option<String>,
    pub pedal_in_gain_db: f32,
    pub pedal_out_gain_db: f32,
    pub in_gain_db: f32,
    pub out_gain_db: f32,
    pub profile_path: Option<String>,
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
    profile_tx: mpsc::Sender<Option<Model>>,
    ir_tx: mpsc::Sender<Option<(FFTConvolver<f32>, Option<FFTConvolver<f32>>)>>,
    in_gain: Arc<AtomicF32>,
    out_gain: Arc<AtomicF32>,
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
        let (profile_tx, profile_rx) = mpsc::channel();
        let (ir_tx, ir_rx) =
            mpsc::channel::<Option<(FFTConvolver<f32>, Option<FFTConvolver<f32>>)>>();

        let pedal_in_gain = Arc::new(AtomicF32::new(db_to_gain(params.pedal_in_gain_db)));
        let pedal_out_gain = Arc::new(AtomicF32::new(db_to_gain(params.pedal_out_gain_db)));
        let in_gain = Arc::new(AtomicF32::new(db_to_gain(params.in_gain_db)));
        let out_gain = Arc::new(AtomicF32::new(db_to_gain(params.out_gain_db)));
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
            params.in_gain_db, params.out_gain_db
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
            profile_rx,
            current_profile: None,
            ir_rx,
            current_ir_l: None,
            current_ir_r: None,
            conv_buf: vec![0.0f32; block_size],
            noise_gate: initial_gate,
            in_gain: Arc::clone(&in_gain),
            out_gain: Arc::clone(&out_gain),
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
            profile_tx,
            ir_tx,
            in_gain,
            out_gain,
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

        engine.load_pedal_profile(params.pedal_path);
        engine.load_amp_profile(params.profile_path);
        engine.load_ir(params.ir_path);

        Ok(engine)
    }

    pub fn load_pedal_profile(&self, path: Option<String>) {
        let tx = self.pedal_profile_tx.clone();
        std::thread::spawn(move || {
            let profile = match path {
                None => {
                    debug!("pedal profile cleared");
                    None
                }
                Some(p) => {
                    debug!("loading pedal profile: {p}");
                    let m = NamModel::from_file(&p)
                        .ok()
                        .and_then(|nm| Model::from_nam(&nm).ok());
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
        let tx = self.profile_tx.clone();
        std::thread::spawn(move || {
            let profile = match path {
                None => {
                    debug!("amp profile cleared");
                    None
                }
                Some(p) => {
                    debug!("loading amp profile: {p}");
                    let m = NamModel::from_file(&p)
                        .ok()
                        .and_then(|nm| Model::from_nam(&nm).ok());
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

    pub fn set_in_gain_db(&self, db: f32) {
        debug!("amp: in={db}dB");
        self.in_gain.set(db_to_gain(db));
    }

    pub fn set_out_gain_db(&self, db: f32) {
        debug!("amp: out={db}dB");
        self.out_gain.set(db_to_gain(db));
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

fn load_wav_channels(path: &str, jack_sample_rate: u32) -> Option<(Vec<f32>, Option<Vec<f32>>)> {
    let mut reader = hound::WavReader::open(path).ok()?;
    let spec = reader.spec();
    if spec.sample_rate != jack_sample_rate {
        warn!(
            "IR sample rate {} != JACK rate {}, pitch may differ",
            spec.sample_rate, jack_sample_rate
        );
    }
    let channels = spec.channels as usize;
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().filter_map(|s| s.ok()).collect(),
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / max)
                .collect()
        }
    };
    if channels == 1 {
        Some((samples, None))
    } else {
        let left: Vec<f32> = samples.chunks(channels).map(|c| c[0]).collect();
        let right: Vec<f32> = samples.chunks(channels).map(|c| c[1]).collect();
        Some((left, Some(right)))
    }
}

fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}
