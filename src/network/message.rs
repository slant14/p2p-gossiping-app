use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// Represents a message in the network
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Message {
    pub content: String,
    pub from: SocketAddr,
    pub timestamp: u64,
}

/// Represents peer information for discovery
#[derive(Serialize, Deserialize, Debug)]
pub struct PeerInfo {
    pub port: u16,
    pub known_peers: Vec<SocketAddr>,
}

/// Enum to differentiate between message types
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", content = "data")]
pub enum NetworkData {
    Message(Message),
    PeerInfo(PeerInfo),
}
