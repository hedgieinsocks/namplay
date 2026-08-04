use gio::prelude::*;
use serde::{Deserialize, Serialize};

use crate::audio::EqPosition;
use crate::keys::*;

#[derive(Serialize, Deserialize)]
pub struct PresetGate {
    pub enabled: bool,
    pub threshold: f64,
}

#[derive(Serialize, Deserialize)]
pub struct PresetEq {
    pub enabled: bool,
    pub position: String,
    pub hp: u32,
    pub low: f64,
    pub mid: f64,
    pub high: f64,
    pub lp: u32,
}

#[derive(Serialize, Deserialize)]
pub struct PresetProfile {
    pub file: String,
    pub input: f64,
    pub output: f64,
    pub bypass: bool,
}

#[derive(Serialize, Deserialize)]
pub struct PresetCab {
    pub file: String,
    pub level: f64,
    pub bypass: bool,
}

#[derive(Serialize, Deserialize)]
pub struct Preset {
    pub mute: bool,
    pub gate: PresetGate,
    pub eq: PresetEq,
    pub pedal: PresetProfile,
    pub amp: PresetProfile,
    pub cab: PresetCab,
}

pub(crate) fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

impl Preset {
    pub fn from_settings(settings: &gio::Settings) -> Self {
        Preset {
            mute: settings.boolean(MUTE),
            gate: PresetGate {
                enabled: settings.boolean(GATE_ENABLED),
                threshold: round1(settings.double(GATE_THRESHOLD)),
            },
            eq: PresetEq {
                enabled: settings.boolean(EQ_ENABLED),
                position: settings.string(EQ_POSITION).to_string(),
                hp: settings.double(EQ_HP).round() as u32,
                low: round1(settings.double(EQ_LOW)),
                mid: round1(settings.double(EQ_MID)),
                high: round1(settings.double(EQ_HIGH)),
                lp: settings.double(EQ_LP).round() as u32,
            },
            pedal: PresetProfile {
                file: settings.string(PEDAL_PATH).to_string(),
                input: round1(settings.double(PEDAL_INPUT)),
                output: round1(settings.double(PEDAL_OUTPUT)),
                bypass: settings.boolean(PEDAL_BYPASS),
            },
            amp: PresetProfile {
                file: settings.string(AMP_PATH).to_string(),
                input: round1(settings.double(AMP_INPUT)),
                output: round1(settings.double(AMP_OUTPUT)),
                bypass: settings.boolean(AMP_BYPASS),
            },
            cab: PresetCab {
                file: settings.string(CAB_PATH).to_string(),
                level: round1(settings.double(CAB_LEVEL)),
                bypass: settings.boolean(CAB_BYPASS),
            },
        }
    }

    pub fn apply(&self, settings: &gio::Settings) {
        let _ = settings.set_boolean(MUTE, self.mute);
        let _ = settings.set_boolean(GATE_ENABLED, self.gate.enabled);
        let _ = settings.set_double(GATE_THRESHOLD, self.gate.threshold);
        let _ = settings.set_boolean(EQ_ENABLED, self.eq.enabled);
        let _ = settings.set_string(
            EQ_POSITION,
            EqPosition::from_setting(&self.eq.position).setting(),
        );
        let _ = settings.set_double(EQ_HP, self.eq.hp as f64);
        let _ = settings.set_double(EQ_LOW, self.eq.low);
        let _ = settings.set_double(EQ_MID, self.eq.mid);
        let _ = settings.set_double(EQ_HIGH, self.eq.high);
        let _ = settings.set_double(EQ_LP, self.eq.lp as f64);
        let _ = settings.set_string(PEDAL_PATH, &self.pedal.file);
        let _ = settings.set_double(PEDAL_INPUT, self.pedal.input);
        let _ = settings.set_double(PEDAL_OUTPUT, self.pedal.output);
        let _ = settings.set_boolean(PEDAL_BYPASS, self.pedal.bypass);
        let _ = settings.set_string(AMP_PATH, &self.amp.file);
        let _ = settings.set_double(AMP_INPUT, self.amp.input);
        let _ = settings.set_double(AMP_OUTPUT, self.amp.output);
        let _ = settings.set_boolean(AMP_BYPASS, self.amp.bypass);
        let _ = settings.set_string(CAB_PATH, &self.cab.file);
        let _ = settings.set_double(CAB_LEVEL, self.cab.level);
        let _ = settings.set_boolean(CAB_BYPASS, self.cab.bypass);
    }
}
