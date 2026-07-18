//! Real-time JACK process callback running the mono signal chain
//! gate -> EQ (pre-pedal) -> pedal -> EQ (pre-amp) -> amp -> IR -> EQ (post-IR),
//! fanning out to stereo at the IR stage when the IR file has two channels.

use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc, Arc,
};

use fft_convolver::FFTConvolver;
use jack::{AudioIn, AudioOut, Client, Control, ProcessHandler, ProcessScope};
use nam_rs::Model;

use super::dsp::{AtomicF32, Eq, NoiseGate};
use super::ir::IrConvolvers;
use super::EqPosition;

pub(super) struct NamProcessor {
    pub(super) gate_enabled: Arc<AtomicBool>,
    pub(super) gate_threshold_db: Arc<AtomicF32>,
    pub(super) noise_gate: NoiseGate,
    pub(super) pedal_profile_rx: mpsc::Receiver<Option<Model>>,
    pub(super) current_pedal_profile: Option<Model>,
    pub(super) pedal_in_gain: Arc<AtomicF32>,
    pub(super) pedal_out_gain: Arc<AtomicF32>,
    pub(super) amp_profile_rx: mpsc::Receiver<Option<Model>>,
    pub(super) current_amp_profile: Option<Model>,
    pub(super) amp_in_gain: Arc<AtomicF32>,
    pub(super) amp_out_gain: Arc<AtomicF32>,
    pub(super) ir_rx: mpsc::Receiver<Option<IrConvolvers>>,
    pub(super) current_ir_l: Option<FFTConvolver<f32>>,
    pub(super) current_ir_r: Option<FFTConvolver<f32>>,
    pub(super) ir_level: Arc<AtomicF32>,
    pub(super) eq_enabled: Arc<AtomicBool>,
    pub(super) eq_pos: Arc<AtomicU32>,
    pub(super) eq_low_db: Arc<AtomicF32>,
    pub(super) eq_mid_db: Arc<AtomicF32>,
    pub(super) eq_high_db: Arc<AtomicF32>,
    pub(super) eq_hp_freq: Arc<AtomicF32>,
    pub(super) eq_lp_freq: Arc<AtomicF32>,
    pub(super) eq_l: Eq,
    pub(super) eq_r: Eq,
    pub(super) conv_buf: Vec<f32>,
    pub(super) in_port: jack::Port<AudioIn>,
    pub(super) out_port_l: jack::Port<AudioOut>,
    pub(super) out_port_r: jack::Port<AudioOut>,
}

fn apply_gain(buf: &mut [f32], gain: f32) {
    for s in buf {
        *s *= gain;
    }
}

impl ProcessHandler for NamProcessor {
    fn process(&mut self, _: &Client, ps: &ProcessScope) -> Control {
        while let Ok(new_profile) = self.pedal_profile_rx.try_recv() {
            self.current_pedal_profile = new_profile;
        }
        while let Ok(new_profile) = self.amp_profile_rx.try_recv() {
            self.current_amp_profile = new_profile;
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
        self.noise_gate.update(self.gate_threshold_db.get());

        let eq_enabled = self.eq_enabled.load(Ordering::Relaxed);
        let eq_pos = EqPosition::from_index(self.eq_pos.load(Ordering::Relaxed));
        let eq_low_db = self.eq_low_db.get();
        let eq_mid_db = self.eq_mid_db.get();
        let eq_high_db = self.eq_high_db.get();
        let eq_hp_freq = self.eq_hp_freq.get();
        let eq_lp_freq = self.eq_lp_freq.get();
        self.eq_l
            .update(eq_low_db, eq_mid_db, eq_high_db, eq_hp_freq, eq_lp_freq);
        self.eq_r
            .update(eq_low_db, eq_mid_db, eq_high_db, eq_hp_freq, eq_lp_freq);

        let pedal_in_gain = self.pedal_in_gain.get();
        let pedal_out_gain = self.pedal_out_gain.get();
        let amp_in_gain = self.amp_in_gain.get();
        let amp_out_gain = self.amp_out_gain.get();
        let ir_level = self.ir_level.get();

        let input = self.in_port.as_slice(ps);
        let out_l = self.out_port_l.as_mut_slice(ps);
        let out_r = self.out_port_r.as_mut_slice(ps);

        for (o, &i) in out_l.iter_mut().zip(input) {
            *o = if gate_enabled {
                self.noise_gate.process_sample(i)
            } else {
                i
            };
        }

        if eq_enabled && eq_pos == EqPosition::PrePedal {
            self.eq_l.process_buffer(out_l);
        }

        if let Some(pedal) = &mut self.current_pedal_profile {
            apply_gain(out_l, pedal_in_gain);
            pedal.process_buffer(out_l);
            apply_gain(out_l, pedal_out_gain);
        }

        if eq_enabled && eq_pos == EqPosition::PreAmp {
            self.eq_l.process_buffer(out_l);
        }

        if let Some(amp) = &mut self.current_amp_profile {
            apply_gain(out_l, amp_in_gain);
            amp.process_buffer(out_l);
            apply_gain(out_l, amp_out_gain);
        }

        let n = out_l.len().min(self.conv_buf.len());
        let mut stereo_ir = false;

        if let Some(ir_l) = &mut self.current_ir_l {
            self.conv_buf[..n].copy_from_slice(&out_l[..n]);
            let _ = ir_l.process(&self.conv_buf[..n], &mut out_l[..n]);
            apply_gain(&mut out_l[..n], ir_level);
            if let Some(ir_r) = &mut self.current_ir_r {
                let _ = ir_r.process(&self.conv_buf[..n], &mut out_r[..n]);
                apply_gain(&mut out_r[..n], ir_level);
                stereo_ir = true;
            }
        }

        if eq_enabled && eq_pos == EqPosition::PostIr {
            self.eq_l.process_buffer(out_l);
            if stereo_ir {
                self.eq_r.process_buffer(out_r);
            }
        }

        if !stereo_ir {
            out_r.copy_from_slice(out_l);
        }

        Control::Continue
    }
}
