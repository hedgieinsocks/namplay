use biquad::{Biquad, Coefficients, DirectForm1, ToHertz, Type, Q_BUTTERWORTH_F32};

const EQ_LOW_FREQ: f32 = 150.0;
const EQ_MID_FREQ: f32 = 425.0;
const EQ_HIGH_FREQ: f32 = 1800.0;
const EQ_MID_Q_CUT: f32 = 1.5;
const EQ_MID_Q_BOOST: f32 = 0.7;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum EqPosition {
    PrePedal = 0,
    PreAmp = 1,
    PostCab = 2,
}

impl EqPosition {
    pub fn from_index(index: u32) -> Self {
        match index {
            0 => Self::PrePedal,
            2 => Self::PostCab,
            _ => Self::PreAmp,
        }
    }

    pub fn from_setting(setting: &str) -> Self {
        match setting {
            "pre-pedal" => Self::PrePedal,
            "post-cab" => Self::PostCab,
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
            Self::PostCab => "post-cab",
        }
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
