use clap::{Arg, Command};
use rand::Rng;
use rand::rngs::StdRng;
use rand::SeedableRng;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
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

    println!("00:00:00 - My address is \"{}\"", addr);

    let peers: SharedPeers = Arc::new(Mutex::new(HashSet::new()));
    let (tx, _) = broadcast::channel(16);

    if let Some(connect_addr) = connect_addr {
        let connect_addr: SocketAddr = connect_addr.parse().unwrap();
        connect_to_peer(connect_addr, port, peers.clone(), tx.clone()).await;
    }

    tokio::spawn(accept_connections(listener, peers.clone(), tx.clone()));

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
            let message_json = serde_json::to_string(&message).unwrap() + "\n"; // Add a delimiter
            let peers = peers_clone.lock().unwrap().clone();

            println!(
                "00:00:0{} - Sending message [{}] to {:?}",
                period, message.content, peers
            );

            for peer in peers {
                if peer != addr {
                    let _ = tx_clone.send((message_json.clone(), peer));
                }
            }
        }
    });

    // Show received messages in a separate task to allow for reset of the subscription
    tokio::spawn(show_received_messages(addr, tx.subscribe()));

    // To keep the main function alive
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

async fn show_received_messages(addr: SocketAddr, mut rx: broadcast::Receiver<(String, SocketAddr)>) {
    let mut seen_messages = HashSet::new();
    loop {
        while let Ok((msg, _)) = rx.recv().await {
            let message: Message = serde_json::from_str(&msg.trim()).unwrap(); // Use trim to handle newlines
            if message.from != addr && is_recent(message.timestamp) {
                // Check if the message has already been seen
                if seen_messages.insert((message.content.clone(), message.timestamp)) {
                    println!(
                        "00:00:00 - Received message [{}] from \"{}\"",
                        message.content, message.from
                    );
                }
            }
        }
    }
}

async fn accept_connections(listener: TcpListener, peers: SharedPeers, tx: broadcast::Sender<(String, SocketAddr)>) {
    loop {
        if let Ok((socket, _)) = listener.accept().await {
            let addr = socket.peer_addr().unwrap();

            // Read the peer's intended port
            let mut reader = BufReader::new(socket);
            let mut buf = String::new();
            reader.read_line(&mut buf).await.unwrap();
            let peer_info: PeerInfo = serde_json::from_str(&buf).unwrap();
            let peer_addr = format!("127.0.0.1:{}", peer_info.port).parse().unwrap();

            println!("00:00:00 - Connected to the peer at \"{}\"", peer_addr);
            peers.lock().unwrap().insert(peer_addr);
            println!("{:?}", peers);

            tokio::spawn(handle_connection(reader.into_inner(), peers.clone(), tx.clone()));
        }
    }
}

async fn connect_to_peer(addr: SocketAddr, port: u16, peers: SharedPeers, tx: broadcast::Sender<(String, SocketAddr)>) {
    if let Ok(mut socket) = TcpStream::connect(addr).await {
        println!("00:00:00 - Connected to the peer at \"{}\"", addr);

        // Send this peer's intended port
        let peer_info = PeerInfo { port };
        let peer_info_json = serde_json::to_string(&peer_info).unwrap() + "\n"; // Add a delimiter
        socket.write_all(peer_info_json.as_bytes()).await.unwrap();

        peers.lock().unwrap().insert(addr);
        tokio::spawn(handle_connection(socket, peers, tx));
    }
}

async fn handle_connection(mut socket: TcpStream, peers: SharedPeers, tx: broadcast::Sender<(String, SocketAddr)>) {
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
                        let message: Message = serde_json::from_str(&msg).unwrap();
                        peers_clone.lock().unwrap().insert(message.from);
                        let _ = tx.send((msg, addr));
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
