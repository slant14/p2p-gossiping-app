mod network;
mod utils;

use clap::{Arg, Command};
use rand::Rng; 
use rand::SeedableRng;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

type SharedPeers = Arc<Mutex<std::collections::HashSet<SocketAddr>>>;

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

    utils::log_with_timestamp(start_time, &format!("My address is \"{}\"", addr));

    let peers: SharedPeers = Arc::new(Mutex::new(std::collections::HashSet::new()));
    let (tx, _) = broadcast::channel(16);

    if let Some(connect_addr) = connect_addr {
        let connect_addr: SocketAddr = connect_addr.parse().unwrap();
        network::peer::connect_to_peer(connect_addr, port, peers.clone(), tx.clone(), addr, start_time).await;
    }

    tokio::spawn(network::peer::accept_connections(listener, peers.clone(), tx.clone(), addr, start_time));

    let peers_clone = peers.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(period));
        let mut rng = rand::rngs::StdRng::from_entropy();

        loop {
            interval.tick().await;
            let message = network::message::Message {
                content: rng.gen::<u32>().to_string(),
                from: addr,
                timestamp: utils::current_timestamp(),
            };
            let network_data = network::message::NetworkData::Message(message);
            let message_json = serde_json::to_string(&network_data).unwrap() + "\n"; // Add a delimiter
            let peers = peers_clone.lock().unwrap().clone();

            utils::log_with_timestamp(start_time, &format!(
                "Sending message [{}] to {:?}",
                if let network::message::NetworkData::Message(ref msg) = network_data { &msg.content } else { "" }, peers
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
                    let peer_info = network::message::PeerInfo { port: addr.port(), known_peers: known_peers.clone() };
                    let network_data = network::message::NetworkData::PeerInfo(peer_info);
                    let peer_info_json = serde_json::to_string(&network_data).unwrap() + "\n";
                    let _ = tx_clone.send((peer_info_json, *peer));
                }
            }
        }
    });

    // Show received messages in a separate task to allow for reset of the subscription
    tokio::spawn(network::peer::show_received_messages(addr, tx.subscribe(), start_time));

    // Keep the main function alive
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}
