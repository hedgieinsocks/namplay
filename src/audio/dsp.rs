//! DSP building blocks: noise gate, 3-band EQ with high/low-pass filters,
//! and the atomic f32 used to share parameters with the real-time thread.

use std::sync::atomic::{AtomicU32, Ordering};

use biquad::{Biquad, Coefficients, DirectForm1, ToHertz, Type, Q_BUTTERWORTH_F32};

const EQ_LOW_FREQ: f32 = 150.0;
const EQ_MID_FREQ: f32 = 425.0;
const EQ_HIGH_FREQ: f32 = 1800.0;
const EQ_MID_Q_CUT: f32 = 1.5;
const EQ_MID_Q_BOOST: f32 = 0.7;

pub(crate) struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub(crate) fn new(val: f32) -> Self {
        AtomicF32(AtomicU32::new(val.to_bits()))
    }
    pub(crate) fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    pub(crate) fn set(&self, val: f32) {
        self.0.store(val.to_bits(), Ordering::Relaxed)
    }
}

pub(super) struct NoiseGate {
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

impl NoiseGate {
    pub(super) fn new(threshold_db: f32, sample_rate: u32) -> Self {
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
            last_threshold_db: threshold_db,
        }
    }

    // update() runs every RT process() call; skip recompute when threshold unchanged
    // since last call, same dirty-check pattern as Eq::update.
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

        // close_threshold sits below open_threshold and hold_counter delays closing,
        // so envelope dips near the threshold don't chatter the gate open/closed.
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

fn passthrough() -> Coefficients<f32> {
    Coefficients {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    }
}

// L and R share one set of EQ controls, so coefficients (the trig-heavy part of
// from_params) are computed once here and applied to both channels' filter state
// in EqChannel, instead of each channel recomputing identical coefficients.
pub(super) struct EqCoeffs {
    hp: Coefficients<f32>,
    low: Coefficients<f32>,
    mid: Coefficients<f32>,
    high: Coefficients<f32>,
    lp: Coefficients<f32>,
    sample_rate: f32,
    last_low_db: f32,
    last_mid_db: f32,
    last_high_db: f32,
    last_hp_freq: f32,
    last_lp_freq: f32,
}

impl EqCoeffs {
    pub(super) fn new(
        low_db: f32,
        mid_db: f32,
        high_db: f32,
        hp_freq: f32,
        lp_freq: f32,
        sample_rate: f32,
    ) -> Self {
        let mut coeffs = EqCoeffs {
            hp: passthrough(),
            low: passthrough(),
            mid: passthrough(),
            high: passthrough(),
            lp: passthrough(),
            sample_rate,
            last_low_db: f32::NAN,
            last_mid_db: f32::NAN,
            last_high_db: f32::NAN,
            last_hp_freq: f32::NAN,
            last_lp_freq: f32::NAN,
        };
        coeffs.update(low_db, mid_db, high_db, hp_freq, lp_freq);
        coeffs
    }

    // update() runs every RT process() call; recomputing biquad coefficients involves
    // trig and is only needed when a param actually changed since the last call.
    pub(super) fn update(
        &mut self,
        low_db: f32,
        mid_db: f32,
        high_db: f32,
        hp_freq: f32,
        lp_freq: f32,
    ) {
        let fs = self.sample_rate.hz();

        if hp_freq != self.last_hp_freq {
            if let Ok(c) = Coefficients::<f32>::from_params(
                Type::HighPass,
                fs,
                hp_freq.hz(),
                Q_BUTTERWORTH_F32,
            ) {
                self.hp = c;
            }
            self.last_hp_freq = hp_freq;
        }
        if low_db != self.last_low_db {
            if let Ok(c) = Coefficients::<f32>::from_params(
                Type::LowShelf(low_db),
                fs,
                EQ_LOW_FREQ.hz(),
                Q_BUTTERWORTH_F32,
            ) {
                self.low = c;
            }
            self.last_low_db = low_db;
        }
        if mid_db != self.last_mid_db {
            let mid_q = if mid_db < 0.0 {
                EQ_MID_Q_CUT
            } else {
                EQ_MID_Q_BOOST
            };
            if let Ok(c) = Coefficients::<f32>::from_params(
                Type::PeakingEQ(mid_db),
                fs,
                EQ_MID_FREQ.hz(),
                mid_q,
            ) {
                self.mid = c;
            }
            self.last_mid_db = mid_db;
        }
        if high_db != self.last_high_db {
            if let Ok(c) = Coefficients::<f32>::from_params(
                Type::HighShelf(high_db),
                fs,
                EQ_HIGH_FREQ.hz(),
                Q_BUTTERWORTH_F32,
            ) {
                self.high = c;
            }
            self.last_high_db = high_db;
        }
        if lp_freq != self.last_lp_freq {
            if let Ok(c) =
                Coefficients::<f32>::from_params(Type::LowPass, fs, lp_freq.hz(), Q_BUTTERWORTH_F32)
            {
                self.lp = c;
            }
            self.last_lp_freq = lp_freq;
        }
    }
}

pub(super) struct EqChannel {
    hp: DirectForm1<f32>,
    low: DirectForm1<f32>,
    mid: DirectForm1<f32>,
    high: DirectForm1<f32>,
    lp: DirectForm1<f32>,
}

impl EqChannel {
    pub(super) fn new() -> Self {
        EqChannel {
            hp: DirectForm1::<f32>::new(passthrough()),
            low: DirectForm1::<f32>::new(passthrough()),
            mid: DirectForm1::<f32>::new(passthrough()),
            high: DirectForm1::<f32>::new(passthrough()),
            lp: DirectForm1::<f32>::new(passthrough()),
        }
    }

    fn process_sample(&mut self, x: f32) -> f32 {
        let x = self.hp.run(x);
        let x = self.low.run(x);
        let x = self.mid.run(x);
        let x = self.high.run(x);
        self.lp.run(x)
    }

    pub(super) fn process_buffer(&mut self, buf: &mut [f32], coeffs: &EqCoeffs) {
        self.hp.update_coefficients(coeffs.hp);
        self.low.update_coefficients(coeffs.low);
        self.mid.update_coefficients(coeffs.mid);
        self.high.update_coefficients(coeffs.high);
        self.lp.update_coefficients(coeffs.lp);
        for s in buf {
            *s = self.process_sample(*s);
        }
    }
}

pub(super) fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}
