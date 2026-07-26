use std::sync::atomic::{AtomicU32, Ordering};

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

pub(super) fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}
