use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc, Arc,
};

use fft_convolver::FFTConvolver;
use jack::{AudioIn, AudioOut, Client, ClientOptions, Control, ProcessHandler, ProcessScope};
use nam_rs::{Model, NamModel};

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

struct NamProcessor {
    model_rx: mpsc::Receiver<Option<Model>>,
    current_model: Option<Model>,
    ir_rx: mpsc::Receiver<Option<FFTConvolver<f32>>>,
    current_ir: Option<FFTConvolver<f32>>,
    conv_buf: Vec<f32>,
    noise_gate: Option<NoiseGate>,
    in_gain: Arc<AtomicU32>,
    out_gain: Arc<AtomicU32>,
    ir_level: Arc<AtomicU32>,
    gate_enabled: Arc<AtomicBool>,
    gate_threshold_db: Arc<AtomicU32>,
    last_gate_enabled: bool,
    last_gate_threshold_db: f32,
    sample_rate: u32,
    in_port: jack::Port<AudioIn>,
    out_port: jack::Port<AudioOut>,
}

impl ProcessHandler for NamProcessor {
    fn process(&mut self, _: &Client, ps: &ProcessScope) -> Control {
        // Swap in any newly loaded model (non-blocking)
        while let Ok(new_model) = self.model_rx.try_recv() {
            self.current_model = new_model;
        }
        // Swap in any newly loaded IR (non-blocking)
        while let Ok(new_ir) = self.ir_rx.try_recv() {
            self.current_ir = new_ir;
        }

        // Recreate noise gate when enabled/threshold changes
        let gate_enabled = self.gate_enabled.load(Ordering::Relaxed);
        let gate_threshold_db = f32::from_bits(self.gate_threshold_db.load(Ordering::Relaxed));
        if gate_enabled != self.last_gate_enabled
            || gate_threshold_db != self.last_gate_threshold_db
        {
            self.last_gate_enabled = gate_enabled;
            self.last_gate_threshold_db = gate_threshold_db;
            self.noise_gate = gate_enabled
                .then(|| NoiseGate::new(gate_threshold_db, self.sample_rate));
        }

        let in_gain = f32::from_bits(self.in_gain.load(Ordering::Relaxed));
        let out_gain = f32::from_bits(self.out_gain.load(Ordering::Relaxed));
        let ir_level = f32::from_bits(self.ir_level.load(Ordering::Relaxed));

        let input = self.in_port.as_slice(ps);
        let output = self.out_port.as_mut_slice(ps);

        match &mut self.current_model {
            Some(model) => {
                for (o, &i) in output.iter_mut().zip(input) {
                    let gated = match &mut self.noise_gate {
                        Some(g) => g.process_sample(i),
                        None => i,
                    };
                    *o = gated * in_gain;
                }
                model.process_buffer(output);
                // Apply cabinet IR convolution after amp model
                if let Some(ir) = &mut self.current_ir {
                    let n = output.len().min(self.conv_buf.len());
                    self.conv_buf[..n].copy_from_slice(&output[..n]);
                    let _ = ir.process(&self.conv_buf[..n], &mut output[..n]);
                    for s in output[..n].iter_mut() {
                        *s *= ir_level;
                    }
                }
                for s in output.iter_mut() {
                    *s *= out_gain;
                }
            }
            None => {
                for s in output.iter_mut() {
                    *s = 0.0;
                }
            }
        }

        Control::Continue
    }
}

pub struct InitialParams {
    pub gate_enabled: bool,
    pub gate_threshold_db: f32,
    pub in_gain_db: f32,
    pub out_gain_db: f32,
    pub model_path: Option<String>,
    pub ir_path: Option<String>,
    pub ir_level_db: f32,
}

pub struct AudioEngine {
    _client: jack::AsyncClient<(), NamProcessor>,
    model_tx: mpsc::Sender<Option<Model>>,
    ir_tx: mpsc::Sender<Option<FFTConvolver<f32>>>,
    in_gain: Arc<AtomicU32>,
    out_gain: Arc<AtomicU32>,
    ir_level: Arc<AtomicU32>,
    gate_enabled: Arc<AtomicBool>,
    gate_threshold_db: Arc<AtomicU32>,
    sample_rate: u32,
    block_size: usize,
}

impl AudioEngine {
    pub fn new(params: InitialParams) -> Result<Self, String> {
        let (client, _status) = Client::new("namplay", ClientOptions::NO_START_SERVER)
            .map_err(|e| format!("JACK connection failed: {e}"))?;

        let sample_rate = client.sample_rate() as u32;
        let block_size = client.buffer_size() as usize;

        let in_port = client
            .register_port("input", AudioIn::default())
            .map_err(|e| format!("register input port: {e}"))?;
        let out_port = client
            .register_port("output", AudioOut::default())
            .map_err(|e| format!("register output port: {e}"))?;

        let (model_tx, model_rx) = mpsc::channel();
        let (ir_tx, ir_rx) = mpsc::channel::<Option<FFTConvolver<f32>>>();

        let in_gain = Arc::new(AtomicU32::new(db_to_gain(params.in_gain_db).to_bits()));
        let out_gain = Arc::new(AtomicU32::new(db_to_gain(params.out_gain_db).to_bits()));
        let ir_level = Arc::new(AtomicU32::new(db_to_gain(params.ir_level_db).to_bits()));
        let gate_enabled = Arc::new(AtomicBool::new(params.gate_enabled));
        let gate_threshold_db = Arc::new(AtomicU32::new(params.gate_threshold_db.to_bits()));

        let initial_gate = params
            .gate_enabled
            .then(|| NoiseGate::new(params.gate_threshold_db, sample_rate));

        let processor = NamProcessor {
            model_rx,
            current_model: None,
            ir_rx,
            current_ir: None,
            conv_buf: vec![0.0f32; block_size],
            noise_gate: initial_gate,
            in_gain: Arc::clone(&in_gain),
            out_gain: Arc::clone(&out_gain),
            ir_level: Arc::clone(&ir_level),
            gate_enabled: Arc::clone(&gate_enabled),
            gate_threshold_db: Arc::clone(&gate_threshold_db),
            last_gate_enabled: params.gate_enabled,
            last_gate_threshold_db: params.gate_threshold_db,
            sample_rate,
            in_port,
            out_port,
        };

        let active_client = client
            .activate_async((), processor)
            .map_err(|e| format!("JACK activation failed: {e}"))?;

        let engine = AudioEngine {
            _client: active_client,
            model_tx,
            ir_tx,
            in_gain,
            out_gain,
            ir_level,
            gate_enabled,
            gate_threshold_db,
            sample_rate,
            block_size,
        };

        engine.load_model(params.model_path);
        engine.load_ir(params.ir_path);

        Ok(engine)
    }

    /// Load (or clear) the amp model on a background thread.
    pub fn load_model(&self, path: Option<String>) {
        let tx = self.model_tx.clone();
        std::thread::spawn(move || {
            let model = path.and_then(|p| {
                let nm = NamModel::from_file(&p).ok()?;
                Model::from_nam(&nm).ok()
            });
            let _ = tx.send(model);
        });
    }

    /// Load (or clear) the impulse response on a background thread.
    pub fn load_ir(&self, path: Option<String>) {
        let tx = self.ir_tx.clone();
        let sample_rate = self.sample_rate;
        let block_size = self.block_size;
        std::thread::spawn(move || {
            let conv = path.and_then(|p| {
                let samples = load_wav_mono(&p, sample_rate)?;
                let mut c = FFTConvolver::<f32>::default();
                c.init(block_size, &samples).ok()?;
                Some(c)
            });
            let _ = tx.send(conv);
        });
    }

    pub fn set_ir_level_db(&self, db: f32) {
        self.ir_level
            .store(db_to_gain(db).to_bits(), Ordering::Relaxed);
    }

    pub fn set_in_gain_db(&self, db: f32) {
        self.in_gain
            .store(db_to_gain(db).to_bits(), Ordering::Relaxed);
    }

    pub fn set_out_gain_db(&self, db: f32) {
        self.out_gain
            .store(db_to_gain(db).to_bits(), Ordering::Relaxed);
    }

    pub fn set_gate_enabled(&self, enabled: bool) {
        self.gate_enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn set_gate_threshold_db(&self, db: f32) {
        self.gate_threshold_db
            .store(db.to_bits(), Ordering::Relaxed);
    }
}

fn load_wav_mono(path: &str, jack_sample_rate: u32) -> Option<Vec<f32>> {
    let mut reader = hound::WavReader::open(path).ok()?;
    let spec = reader.spec();
    if spec.sample_rate != jack_sample_rate {
        eprintln!(
            "namplay: IR sample rate {} != JACK rate {}, pitch may differ",
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
    // Use only the first (left) channel for stereo IRs
    Some(if channels == 1 {
        samples
    } else {
        samples.into_iter().step_by(channels).collect()
    })
}

fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}
