use clap::{Arg, Command};
use rand::Rng;
use rand::rngs::StdRng;
use rand::SeedableRng;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio::time::interval;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Message {
    content: String,
    from: SocketAddr,
    timestamp: u64,
}

#[derive(Serialize, Deserialize, Debug)]
struct PeerInfo {
    port: u16,
    known_peers: Vec<SocketAddr>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", content = "data")]
enum NetworkData {
    Message(Message),
    PeerInfo(PeerInfo),
}

type SharedPeers = Arc<Mutex<HashSet<SocketAddr>>>;

#[tokio::main]
async fn main() {
    let matches = Command::new("P2P Network")
        .arg(Arg::new("period")
            .long("period")
            .required(true)
            .value_name("SECONDS")
            .help("Set the messaging period in seconds"))
        .arg(Arg::new("port")
            .long("port")
            .required(true)
            .value_name("PORT")
            .help("Set the port number"))
        .arg(Arg::new("connect")
            .long("connect")
            .value_name("ADDRESS")
            .help("Connect to a peer at ADDRESS"))
        .get_matches();

    let period: u64 = matches.get_one::<String>("period").unwrap()
        .parse::<u64>().expect("Invalid period");
    let port: u16 = matches.get_one::<String>("port").unwrap()
        .parse::<u16>().expect("Invalid port");
    let connect_addr = matches.get_one::<String>("connect");

    let addr = format!("127.0.0.1:{}", port).parse().unwrap();
    let listener = TcpListener::bind(&addr).await.unwrap();

    let start_time = Instant::now();

    log_with_timestamp(start_time, &format!("My address is \"{}\"", addr));

    let peers: SharedPeers = Arc::new(Mutex::new(HashSet::new()));
    let (tx, _) = broadcast::channel(16);

    if let Some(connect_addr) = connect_addr {
        let connect_addr: SocketAddr = connect_addr.parse().unwrap();
        connect_to_peer(connect_addr, port, peers.clone(), tx.clone(), addr, start_time).await;
    }

    tokio::spawn(accept_connections(listener, peers.clone(), tx.clone(), addr, start_time));

    let peers_clone = peers.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(period));
        let mut rng = StdRng::from_entropy();

        loop {
            interval.tick().await;
            let message = Message {
                content: rng.gen::<u32>().to_string(),
                from: addr,
                timestamp: current_timestamp(),
            };
            let network_data = NetworkData::Message(message);
            let message_json = serde_json::to_string(&network_data).unwrap() + "\n"; // Add a delimiter
            let peers = peers_clone.lock().unwrap().clone();

            log_with_timestamp(start_time, &format!(
                "Sending message [{}] to {:?}",
                if let NetworkData::Message(ref msg) = network_data { &msg.content } else { "" }, peers
            ));

            for peer in &peers {
                if peer != &addr {
                    let _ = tx_clone.send((message_json.clone(), *peer));
                }
            }

            // Share known peers excluding self
            let known_peers: Vec<SocketAddr> = peers_clone.lock().unwrap()
                .iter().cloned().filter(|p| p != &addr).collect();
            for peer in &peers {
                if peer != &addr {
                    let peer_info = PeerInfo { port: addr.port(), known_peers: known_peers.clone() };
                    let network_data = NetworkData::PeerInfo(peer_info);
                    let peer_info_json = serde_json::to_string(&network_data).unwrap() + "\n";
                    let _ = tx_clone.send((peer_info_json, *peer));
                }
            }
        }
    });

    // Show received messages in a separate task to allow for reset of the subscription
    tokio::spawn(show_received_messages(addr, tx.subscribe(), start_time));

    // To keep the main function alive
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

async fn show_received_messages(addr: SocketAddr, mut rx: broadcast::Receiver<(String, SocketAddr)>, start_time: Instant) {
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

async fn accept_connections(listener: TcpListener, peers: SharedPeers, tx: broadcast::Sender<(String, SocketAddr)>, self_addr: SocketAddr, start_time: Instant) {
    loop {
        if let Ok((socket, _)) = listener.accept().await {
            let addr = socket.peer_addr().unwrap();

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

async fn connect_to_peer(addr: SocketAddr, port: u16, peers: SharedPeers, tx: broadcast::Sender<(String, SocketAddr)>, self_addr: SocketAddr, start_time: Instant) {
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

async fn handle_connection(mut socket: TcpStream, peers: SharedPeers, tx: broadcast::Sender<(String, SocketAddr)>, self_addr: SocketAddr, start_time: Instant) {
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

fn current_timestamp() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn is_recent(timestamp: u64) -> bool {
    let now = current_timestamp();
    now <= timestamp + 10 // Accept messages that are at most 10 seconds old
}

fn log_with_timestamp(start_time: Instant, message: &str) {
    let elapsed = start_time.elapsed();
    let seconds = elapsed.as_secs();
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let formatted_time = format!("{:02}:{:02}:{:02}", hours % 24, minutes % 60, seconds % 60);
    println!("{} - {}", formatted_time, message);
}
