//! YAML preset files: serialization to and from GSettings.

use gio::prelude::*;
use serde::{Deserialize, Serialize};

use crate::audio::EqPosition;

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
}

#[derive(Serialize, Deserialize)]
pub struct PresetIr {
    pub file: String,
    pub level: f64,
}

#[derive(Serialize, Deserialize)]
pub struct Preset {
    pub gate: PresetGate,
    pub eq: PresetEq,
    pub pedal: PresetProfile,
    pub amp: PresetProfile,
    pub ir: PresetIr,
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

impl Preset {
    pub fn from_settings(settings: &gio::Settings) -> Self {
        Preset {
            gate: PresetGate {
                enabled: settings.boolean("noise-gate-enabled"),
                threshold: round1(settings.double("noise-gate-threshold")),
            },
            eq: PresetEq {
                enabled: settings.boolean("eq-enabled"),
                position: settings.string("eq-position").to_string(),
                hp: settings.double("eq-hp").round() as u32,
                low: round1(settings.double("eq-low")),
                mid: round1(settings.double("eq-mid")),
                high: round1(settings.double("eq-high")),
                lp: settings.double("eq-lp").round() as u32,
            },
            pedal: PresetProfile {
                file: settings.string("pedal-profile-path").to_string(),
                input: round1(settings.double("pedal-profile-input")),
                output: round1(settings.double("pedal-profile-output")),
            },
            amp: PresetProfile {
                file: settings.string("amp-profile-path").to_string(),
                input: round1(settings.double("amp-profile-input")),
                output: round1(settings.double("amp-profile-output")),
            },
            ir: PresetIr {
                file: settings.string("ir-path").to_string(),
                level: round1(settings.double("ir-level")),
            },
        }
    }

    pub fn apply(&self, settings: &gio::Settings) {
        let _ = settings.set_boolean("noise-gate-enabled", self.gate.enabled);
        let _ = settings.set_double("noise-gate-threshold", self.gate.threshold);
        let _ = settings.set_boolean("eq-enabled", self.eq.enabled);
        let _ = settings.set_string(
            "eq-position",
            EqPosition::from_setting(&self.eq.position).setting(),
        );
        let _ = settings.set_double("eq-hp", self.eq.hp as f64);
        let _ = settings.set_double("eq-low", self.eq.low);
        let _ = settings.set_double("eq-mid", self.eq.mid);
        let _ = settings.set_double("eq-high", self.eq.high);
        let _ = settings.set_double("eq-lp", self.eq.lp as f64);
        let _ = settings.set_string("pedal-profile-path", &self.pedal.file);
        let _ = settings.set_double("pedal-profile-input", self.pedal.input);
        let _ = settings.set_double("pedal-profile-output", self.pedal.output);
        let _ = settings.set_string("amp-profile-path", &self.amp.file);
        let _ = settings.set_double("amp-profile-input", self.amp.input);
        let _ = settings.set_double("amp-profile-output", self.amp.output);
        let _ = settings.set_string("ir-path", &self.ir.file);
        let _ = settings.set_double("ir-level", self.ir.level);
    }
}
