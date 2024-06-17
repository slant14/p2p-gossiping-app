use super::message::{NetworkData, PeerInfo};
use crate::utils::{log_with_timestamp, is_recent};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

/// Type alias for a shared list of peers
type SharedPeers = Arc<Mutex<HashSet<SocketAddr>>>;

/// Accept incoming connections and handle them
pub async fn accept_connections(listener: TcpListener, peers: SharedPeers, tx: broadcast::Sender<(String, SocketAddr)>, self_addr: SocketAddr, start_time: Instant) {
    loop {
        if let Ok((socket, _)) = listener.accept().await {
            let _addr = socket.peer_addr().unwrap();

            // Read the peer's intended port
            let mut reader = BufReader::new(socket);
            let mut buf = String::new();
            reader.read_line(&mut buf).await.unwrap();
            let network_data: NetworkData = serde_json::from_str(&buf).unwrap();
            if let NetworkData::PeerInfo(peer_info) = network_data {
                let peer_addr = format!("127.0.0.1:{}", peer_info.port).parse().unwrap();

                log_with_timestamp(start_time, &format!("Connected to the peer at \"{}\"", peer_addr));
                let mut peer_list = peers.lock().unwrap();
                peer_list.insert(peer_addr);
                for known_peer in peer_info.known_peers {
                    if known_peer != self_addr {
                        peer_list.insert(known_peer);
                    }
                }
                log_with_timestamp(start_time, &format!("{:?}", peer_list));

                tokio::spawn(handle_connection(reader.into_inner(), peers.clone(), tx.clone(), self_addr, start_time));
            }
        }
    }
}

/// Connect to a specified peer and handle the connection
pub async fn connect_to_peer(addr: SocketAddr, port: u16, peers: SharedPeers, tx: broadcast::Sender<(String, SocketAddr)>, self_addr: SocketAddr, start_time: Instant) {
    if let Ok(mut socket) = TcpStream::connect(addr).await {
        log_with_timestamp(start_time, &format!("Connected to the peer at \"{}\"", addr));

        let known_peers: Vec<SocketAddr> = peers.lock().unwrap()
            .iter().cloned().filter(|p| p != &self_addr).collect();
        let peer_info = PeerInfo { port, known_peers };
        let network_data = NetworkData::PeerInfo(peer_info);
        let peer_info_json = serde_json::to_string(&network_data).unwrap() + "\n"; // Add a delimiter
        socket.write_all(peer_info_json.as_bytes()).await.unwrap();

        peers.lock().unwrap().insert(addr);
        tokio::spawn(handle_connection(socket, peers, tx, self_addr, start_time));
    }
}

/// Handle connection for a peer, manage message passing and disconnection
pub async fn handle_connection(socket: TcpStream, peers: SharedPeers, tx: broadcast::Sender<(String, SocketAddr)>, self_addr: SocketAddr, start_time: Instant) {
    let _ = start_time;
    let addr = socket.peer_addr().unwrap();
    let (reader, mut writer) = tokio::io::split(socket);
    let mut rx = tx.subscribe();

    let peers_clone = peers.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    // Connection was closed
                    peers_clone.lock().unwrap().remove(&addr);
                    break;
                }
                Ok(_) => {
                    let msg = line.trim().to_string();
                    if !msg.is_empty() {
                        let network_data: NetworkData = serde_json::from_str(&msg).unwrap();
                        match network_data {
                            NetworkData::Message(message) => {
                                if message.from != self_addr {
                                    peers_clone.lock().unwrap().insert(message.from);
                                }
                                let _ = tx.send((msg, addr));
                            }
                            NetworkData::PeerInfo(peer_info) => {
                                let mut peer_list = peers_clone.lock().unwrap();
                                for known_peer in peer_info.known_peers {
                                    if known_peer != self_addr {
                                        peer_list.insert(known_peer);
                                    }
                                }
                            }
                        }
                    }
                }
                Err(_) => {
                    peers_clone.lock().unwrap().remove(&addr);
                    break;
                }
            }
        }
    });

    loop {
        if let Ok((msg, peer_addr)) = rx.recv().await {
            if peer_addr != addr {
                if let Err(_) = writer.write_all((msg + "\n").as_bytes()).await {
                    break;
                }
            }
        }
    }
}

/// Display received messages from peers
pub async fn show_received_messages(addr: SocketAddr, mut rx: broadcast::Receiver<(String, SocketAddr)>, start_time: Instant) {
    let mut seen_messages = HashSet::new();
    loop {
        while let Ok((msg, _)) = rx.recv().await {
            let network_data: NetworkData = serde_json::from_str(&msg.trim()).unwrap(); // Use trim to handle newlines
            if let NetworkData::Message(message) = network_data {
                if message.from != addr && is_recent(message.timestamp) {
                    // Check if the message has already been seen
                    if seen_messages.insert((message.content.clone(), message.timestamp)) {
                        log_with_timestamp(start_time, &format!(
                            "Received message [{}] from \"{}\"",
                            message.content, message.from
                        ));
                    }
                }
            }
        }
    }
}
