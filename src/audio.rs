use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc, Arc,
};

use jack::{AudioIn, AudioOut, Client, ClientOptions, Control, ProcessHandler, ProcessScope};
use nam_rs::{Model, NamModel};

struct NoiseGate {
    threshold: f32,
    attack_coeff: f32,
    release_coeff: f32,
    envelope: f32,
    gain: f32,
}

impl NoiseGate {
    fn new(threshold_db: f32, sample_rate: u32) -> Self {
        let sr = sample_rate as f32;
        NoiseGate {
            threshold: db_to_gain(threshold_db),
            attack_coeff: (-1.0_f32 / (0.001 * sr)).exp(),
            release_coeff: (-1.0_f32 / (0.100 * sr)).exp(),
            envelope: 0.0,
            gain: 0.0,
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

        let target = if self.envelope > self.threshold {
            1.0_f32
        } else {
            0.0
        };
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
    noise_gate: Option<NoiseGate>,
    in_gain: Arc<AtomicU32>,
    out_gain: Arc<AtomicU32>,
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
}

pub struct AudioEngine {
    _client: jack::AsyncClient<(), NamProcessor>,
    model_tx: mpsc::Sender<Option<Model>>,
    in_gain: Arc<AtomicU32>,
    out_gain: Arc<AtomicU32>,
    gate_enabled: Arc<AtomicBool>,
    gate_threshold_db: Arc<AtomicU32>,
}

impl AudioEngine {
    pub fn new(params: InitialParams) -> Result<Self, String> {
        let (client, _status) = Client::new("namplay", ClientOptions::NO_START_SERVER)
            .map_err(|e| format!("JACK connection failed: {e}"))?;

        let sample_rate = client.sample_rate() as u32;

        let in_port = client
            .register_port("input", AudioIn::default())
            .map_err(|e| format!("register input port: {e}"))?;
        let out_port = client
            .register_port("output", AudioOut::default())
            .map_err(|e| format!("register output port: {e}"))?;

        let (model_tx, model_rx) = mpsc::channel();

        let in_gain = Arc::new(AtomicU32::new(db_to_gain(params.in_gain_db).to_bits()));
        let out_gain = Arc::new(AtomicU32::new(db_to_gain(params.out_gain_db).to_bits()));
        let gate_enabled = Arc::new(AtomicBool::new(params.gate_enabled));
        let gate_threshold_db = Arc::new(AtomicU32::new(params.gate_threshold_db.to_bits()));

        let initial_gate = params
            .gate_enabled
            .then(|| NoiseGate::new(params.gate_threshold_db, sample_rate));

        let processor = NamProcessor {
            model_rx,
            current_model: None,
            noise_gate: initial_gate,
            in_gain: Arc::clone(&in_gain),
            out_gain: Arc::clone(&out_gain),
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
            in_gain,
            out_gain,
            gate_enabled,
            gate_threshold_db,
        };

        if let Some(path) = params.model_path {
            engine.load_model(Some(path));
        }

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

fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}
