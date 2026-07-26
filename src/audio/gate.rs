use super::util::db_to_gain;

pub(super) struct Gate {
    open_threshold: f32,
    close_threshold: f32,
    attack_coeff: f32,
    release_coeff: f32,
    hold_samples: u32,
    envelope: f32,
    gain: f32,
    gate_open: bool,
    hold_counter: u32,
    last_threshold_db: f32,
}

impl Gate {
    pub(super) fn new(threshold_db: f32, sample_rate: u32) -> Self {
        let sr = sample_rate as f32;
        Gate {
            open_threshold: db_to_gain(threshold_db),
            close_threshold: db_to_gain(threshold_db - 6.0),
            attack_coeff: (-1.0_f32 / (0.001 * sr)).exp(),
            release_coeff: (-1.0_f32 / (0.100 * sr)).exp(),
            hold_samples: (0.050 * sr) as u32,
            envelope: 0.0,
            gain: 0.0,
            gate_open: false,
            hold_counter: 0,
            last_threshold_db: threshold_db,
        }
    }

    pub(super) fn update(&mut self, threshold_db: f32) {
        if threshold_db != self.last_threshold_db {
            self.open_threshold = db_to_gain(threshold_db);
            self.close_threshold = db_to_gain(threshold_db - 6.0);
            self.last_threshold_db = threshold_db;
        }
    }

    pub(super) fn process_sample(&mut self, sample: f32) -> f32 {
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
