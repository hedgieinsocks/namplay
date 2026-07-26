use jack::{Client, PortFlags};

const PORT_TYPE: &str = "32 bit float mono audio";

pub(super) fn audio_devices(client: &Client, flags: PortFlags) -> Vec<String> {
    let own_name = client.name();
    let mut names: Vec<String> = client
        .ports(None, Some(PORT_TYPE), flags)
        .iter()
        .filter_map(|port| port.split_once(':').map(|(node, _)| node.to_string()))
        .filter(|node| node != own_name)
        .collect();
    names.sort();
    names.dedup();
    names
}

pub(super) fn matching_ports(client: &Client, device: &str, flags: PortFlags) -> Vec<String> {
    let prefix = format!("{device}:");
    let mut ports: Vec<String> = client
        .ports(None, Some(PORT_TYPE), flags)
        .into_iter()
        .filter(|p| p.starts_with(&prefix))
        .collect();
    ports.sort_by_key(|p| port_channel_index(p));
    ports
}

fn port_channel_index(port: &str) -> u32 {
    port.rsplit(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(0)
}
