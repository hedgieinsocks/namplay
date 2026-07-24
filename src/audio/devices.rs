//! JACK port <-> "device" name matching for the input/output device dropdowns.

use jack::{Client, PortFlags};

pub(super) fn audio_devices(client: &Client, flags: PortFlags) -> Vec<String> {
    let own_name = client.name();
    let mut names: Vec<String> = client
        .ports(None, Some("32 bit float mono audio"), flags)
        .iter()
        .filter_map(|port| port.split_once(':').map(|(node, _)| node.to_string()))
        .filter(|node| node != own_name)
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Ports belonging to `device` with `flags`, ordered by their trailing
/// channel number rather than lexicographically: a plain string sort puts
/// `playback_10` before `playback_2` on any device with 10+ ports, which
/// would connect `out_2` to the wrong physical channel.
pub(super) fn matching_ports(client: &Client, device: &str, flags: PortFlags) -> Vec<String> {
    let prefix = format!("{device}:");
    let mut ports: Vec<String> = client
        .ports(None, Some("32 bit float mono audio"), flags)
        .into_iter()
        .filter(|p| p.starts_with(&prefix))
        .collect();
    ports.sort_by_key(|p| port_channel_index(p));
    ports
}

/// The trailing run of digits in a port name (e.g. `10` for `..._10`), used
/// as a numeric sort key. Ports with no trailing digits sort first.
fn port_channel_index(port: &str) -> u32 {
    port.rsplit(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(0)
}
