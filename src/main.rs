use clap::{Arg, Command};
use rand::Rng;
use rand::rngs::StdRng;
use rand::SeedableRng;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio::time::interval;

#[derive(Serialize, Deserialize, Debug)]
struct Message {
    content: String,
    from: SocketAddr,
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
            };
            let message_json = serde_json::to_string(&message).unwrap();
            let peers = peers_clone.lock().unwrap().clone();

            println!(
                "00:00:0{} - Sending message [{}] to {:?}",
                period, message.content, peers
            );

            for peer in peers {
                let _ = tx_clone.send((message_json.clone(), peer));
            }
        }
    });

    let mut rx = tx.subscribe();
    loop {
        while let Ok((msg, _)) = rx.recv().await {
            let message: Message = serde_json::from_str(&msg).unwrap();
            if message.from != addr {
                println!(
                    "00:00:0{} - Received message [{}] from \"{}\"",
                    period, message.content, message.from
                );
            }
        }
    }
}

async fn accept_connections(listener: TcpListener, peers: SharedPeers, tx: broadcast::Sender<(String, SocketAddr)>) {
    loop {
        if let Ok((mut socket, _)) = listener.accept().await {
            let addr = socket.peer_addr().unwrap();

            // Read the peer's intended port
            let mut buf = vec![0; 1024];
            let n = socket.read(&mut buf).await.unwrap();
            let peer_info: PeerInfo = serde_json::from_slice(&buf[..n]).unwrap();
            let peer_addr = format!("127.0.0.1:{}", peer_info.port).parse().unwrap();

            println!("00:00:00 - Connected to the peer at \"{}\"", peer_addr);
            peers.lock().unwrap().insert(peer_addr);
            println!("{:?}", peers);

            tokio::spawn(handle_connection(socket, peers.clone(), tx.clone()));
        }
    }
}

async fn connect_to_peer(addr: SocketAddr, port: u16, peers: SharedPeers, tx: broadcast::Sender<(String, SocketAddr)>) {
    if let Ok(mut socket) = TcpStream::connect(addr).await {
        println!("00:00:00 - Connected to the peer at \"{}\"", addr);

        // Send this peer's intended port
        let peer_info = PeerInfo { port };
        let peer_info_json = serde_json::to_string(&peer_info).unwrap();
        socket.write_all(peer_info_json.as_bytes()).await.unwrap();

        peers.lock().unwrap().insert(addr);
        tokio::spawn(handle_connection(socket, peers, tx));
    }
}

async fn handle_connection(mut socket: TcpStream, peers: SharedPeers, tx: broadcast::Sender<(String, SocketAddr)>) {
    let addr = socket.peer_addr().unwrap();
    let (mut reader, mut writer) = tokio::io::split(socket);
    let mut rx = tx.subscribe();

    let peers_clone = peers.clone();
    tokio::spawn(async move {
        let mut buf = vec![0; 1024];
        loop {
            match reader.read_exact(&mut buf).await {
                Ok(_) => {
                    let msg = String::from_utf8(buf.clone()).unwrap();
                    let message: Message = serde_json::from_str(&msg).unwrap();
                    peers_clone.lock().unwrap().insert(message.from);
                    let _ = tx.send((msg, addr));
                }
                Err(_) => {
                    peers_clone.lock().unwrap().remove(&addr);
                    break;
                }
            }
        }
    });

    loop {
        if let Ok((msg, _)) = rx.recv().await {
            if let Err(_) = writer.write_all(msg.as_bytes()).await {
                break;
            }
        }
    }
}
