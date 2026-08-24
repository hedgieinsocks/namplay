use std::path::Path;
use std::sync::mpsc;

use futures_channel::mpsc::UnboundedSender;
use neural_amp_modeler_rs::dsp::cabsim::adapter::CabSimAdapter;
use neural_amp_modeler_rs::dsp::cabsim::conv::ConvEngine;
use neural_amp_modeler_rs::dsp::cabsim::loader::CabSimIr;

use super::EngineEvent;

pub(super) type CabConvolver = CabSimAdapter;

pub(super) fn spawn(
    tx: mpsc::Sender<Option<CabConvolver>>,
    path: Option<String>,
    sample_rate: u32,
    block_size: usize,
    event_tx: UnboundedSender<EngineEvent>,
) {
    super::spawn_background_load(
        "cab",
        tx,
        path,
        move |p| load(p, sample_rate, block_size),
        || {},
        |p| format!("Cab: failed to load file: {p}"),
        event_tx,
    );
}

fn load(path: &str, sample_rate: u32, block_size: usize) -> Option<CabConvolver> {
    let ir = CabSimIr::load(Path::new(path), sample_rate, false).ok()?;
    let engine = ConvEngine::new(&ir.samples, block_size).ok()?;
    CabSimAdapter::new(Box::new(engine)).ok()
}
