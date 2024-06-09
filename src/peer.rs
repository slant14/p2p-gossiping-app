use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::SocketAddr;
use structopt::StructOpt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::{self, Duration};
use rand::Rng;

#[derive(StructOpt)]
struct Opt {
    /// Period for sending messages (seconds)
    #[structopt(long)]
    period: u64,

    /// Port to bind the peer
    #[structopt(long)]
    port: u16,

    /// Address to connect to initially (optional)
    #[structopt(long)]
    connect: Option<SocketAddr>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Message {
    content: String,
    from: SocketAddr,
}

#[tokio::main]
async fn main() {
    let opt = Opt::from_args();
    let addr = SocketAddr::from(([127, 0, 0, 1], opt.port));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut peers = HashSet::new();

    println!("00:00:00 - My address is \"{}\"", addr);

    if let Some(connect_addr) = opt.connect {
        let stream = TcpStream::connect(connect_addr).await.expect("Failed to connect");
        handle_connection(stream, tx.clone()).await;
        peers.insert(connect_addr);
    }

    let listener = TcpListener::bind(addr).await.expect("Failed to bind");
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.expect("Failed to accept");
            handle_connection(stream, tx_clone.clone()).await;
        }
    });

    let mut interval = time::interval(Duration::from_secs(opt.period));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let random_msg = generate_random_message();
                println!("00:00:07 - Sending message [{}] to {:?}", random_msg, peers);
                for &peer in &peers {
                    let stream = TcpStream::connect(peer).await.expect("Failed to connect");
                    let msg = Message { content: random_msg.clone(), from: addr };
                    let msg = serde_json::to_string(&msg).expect("Failed to serialize message");
                    stream.write_all(msg.as_bytes()).await.expect("Failed to send message");
                }
            }
            Some(msg) = rx.recv() => {
                let msg: Message = serde_json::from_str(&msg).expect("Failed to deserialize message");
                if peers.insert(msg.from) {
                    println!("00:00:00 - Connected to the peers at {:?}", peers);
                }
                println!("00:00:05 - Received message [{}] from \"{}\"", msg.content, msg.from);
            }
        }
    }
}

async fn handle_connection(mut stream: TcpStream, tx: mpsc::UnboundedSender<String>) {
    let (reader, mut writer) = stream.split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    tokio::spawn(async move {
        loop {
            line.clear();
            let bytes_read = reader.read_line(&mut line).await.expect("Failed to read line");
            if bytes_read == 0 {
                break;
            }
            tx.send(line.clone()).expect("Failed to send message");
        }
    });
}

fn generate_random_message() -> String {
    let random_number: u32 = rand::thread_rng().gen_range(0..10000);
    format!("Random message {}", random_number)
}
