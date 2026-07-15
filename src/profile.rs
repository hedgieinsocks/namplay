use gio::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct ProfileGate {
    pub enabled: bool,
    pub threashold: f64,
}

#[derive(Serialize, Deserialize)]
pub struct ProfileEq {
    pub enabled: bool,
    pub position: String,
    pub hp: f64,
    pub low: f64,
    pub mid: f64,
    pub high: f64,
    pub lp: f64,
}

#[derive(Serialize, Deserialize)]
pub struct ProfilePedal {
    pub file: String,
    pub input: f64,
    pub output: f64,
}

#[derive(Serialize, Deserialize)]
pub struct ProfileAmp {
    pub file: String,
    pub input: f64,
    pub output: f64,
}

#[derive(Serialize, Deserialize)]
pub struct ProfileIr {
    pub file: String,
    pub level: f64,
}

#[derive(Serialize, Deserialize)]
pub struct Profile {
    pub gate: ProfileGate,
    pub eq: ProfileEq,
    pub pedal: ProfilePedal,
    pub amp: ProfileAmp,
    pub ir: ProfileIr,
}

pub fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

pub fn build_preset_from_settings(settings: &gio::Settings) -> Profile {
    Profile {
        gate: ProfileGate {
            enabled: settings.boolean("noise-gate-enabled"),
            threashold: round1(settings.double("noise-gate-threshold")),
        },
        eq: ProfileEq {
            enabled: settings.boolean("eq-enabled"),
            position: settings.string("eq-position").to_string(),
            hp: settings.double("eq-hp").round(),
            low: round1(settings.double("eq-low")),
            mid: round1(settings.double("eq-mid")),
            high: round1(settings.double("eq-high")),
            lp: settings.double("eq-lp").round(),
        },
        pedal: ProfilePedal {
            file: settings.string("pedal-profile-path").to_string(),
            input: round1(settings.double("pedal-profile-input")),
            output: round1(settings.double("pedal-profile-output")),
        },
        amp: ProfileAmp {
            file: settings.string("amp-profile-path").to_string(),
            input: round1(settings.double("amp-profile-input")),
            output: round1(settings.double("amp-profile-output")),
        },
        ir: ProfileIr {
            file: settings.string("ir-path").to_string(),
            level: round1(settings.double("ir-level")),
        },
    }
}

pub fn apply_preset_to_settings(profile: &Profile, settings: &gio::Settings) {
    let _ = settings.set_boolean("noise-gate-enabled", profile.gate.enabled);
    let _ = settings.set_double("noise-gate-threshold", profile.gate.threashold);
    let _ = settings.set_boolean("eq-enabled", profile.eq.enabled);
    let eq_pos = match profile.eq.position.as_str() {
        pos @ ("pre-pedal" | "post-ir") => pos,
        _ => "pre-amp",
    };
    let _ = settings.set_string("eq-position", eq_pos);
    let _ = settings.set_double("eq-hp", profile.eq.hp);
    let _ = settings.set_double("eq-low", profile.eq.low);
    let _ = settings.set_double("eq-mid", profile.eq.mid);
    let _ = settings.set_double("eq-high", profile.eq.high);
    let _ = settings.set_double("eq-lp", profile.eq.lp);
    let _ = settings.set_string("pedal-profile-path", &profile.pedal.file);
    let _ = settings.set_double("pedal-profile-input", profile.pedal.input);
    let _ = settings.set_double("pedal-profile-output", profile.pedal.output);
    let _ = settings.set_string("amp-profile-path", &profile.amp.file);
    let _ = settings.set_double("amp-profile-input", profile.amp.input);
    let _ = settings.set_double("amp-profile-output", profile.amp.output);
    let _ = settings.set_string("ir-path", &profile.ir.file);
    let _ = settings.set_double("ir-level", profile.ir.level);
}
