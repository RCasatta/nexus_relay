use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use lwk_wollet::elements::AssetId;
use lwk_wollet::{LiquidexProposal, Validated};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

// Define the message format for topic subscription and publishing
#[derive(Debug)]
enum ClientMessage {
    Subscribe(String),
    Publish(LiquidexProposal<Validated>),
    Error(String),
}

// Parse text messages into our message format
fn parse_client_message(text: &str) -> ClientMessage {
    if text.starts_with("SUBSCRIBE:") {
        let topic = text
            .strip_prefix("SUBSCRIBE:")
            .unwrap_or("")
            .trim()
            .to_string();
        let mut assets = topic.splitn(2, ':');
        if let (Some(asset1), Some(asset2)) = (assets.next(), assets.next()) {
            let asset1 = AssetId::from_str(asset1);
            let asset2 = AssetId::from_str(asset2);
            if let (Ok(asset1), Ok(asset2)) = (asset1, asset2) {
                ClientMessage::Subscribe(format!("{}:{}", asset1, asset2))
            } else {
                ClientMessage::Error("Invalid asset ID".to_string())
            }
        } else {
            ClientMessage::Error("Cannot parse SUBSCRIBE message".to_string())
        }
    } else if text.starts_with("PUBLISH:") {
        let content = text
            .strip_prefix("PUBLISH:")
            .expect("just checkt with starts with");

        let proposal = LiquidexProposal::from_str(content);
        match proposal {
            Ok(proposal) => {
                if let Ok(proposal) = proposal.insecure_validate() {
                    ClientMessage::Publish(proposal)
                } else {
                    ClientMessage::Error("proposal does not validate".to_string())
                }
            }
            Err(error) => ClientMessage::Error(format!("Invalid proposal: {}", error)),
        }
    } else {
        ClientMessage::Error("Cannot parse message".to_string())
    }
}

// Our global state to track topic subscribers
struct TopicRegistry {
    topics: HashMap<String, Vec<mpsc::UnboundedSender<String>>>,
}

impl TopicRegistry {
    fn new() -> Self {
        Self {
            topics: HashMap::new(),
        }
    }

    // Add a subscriber to a topic
    fn subscribe(&mut self, topic: String, sender: mpsc::UnboundedSender<String>) {
        self.topics
            .entry(topic)
            .or_insert_with(Vec::new)
            .push(sender);
    }

    // Send a message to all subscribers of a topic
    fn publish(&mut self, topic: &str, message: String) -> usize {
        let subscribers = self
            .topics
            .entry(topic.to_string())
            .or_insert_with(Vec::new);
        let mut sent_count = 0;

        // Remove subscribers that are closed
        subscribers.retain(|sender| {
            let is_open = match sender.send(message.clone()) {
                Ok(_) => {
                    sent_count += 1;
                    true
                }
                Err(_) => false, // Receiver was dropped
            };
            is_open
        });

        sent_count
    }
}

// Remove the tokio::main macro and implement a manual entry point
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a runtime for async code execution
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    // Run the async main function
    rt.block_on(async_main())
}

// The actual async implementation
async fn async_main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command line arguments for port
    let args: Vec<String> = std::env::args().collect();
    let port = if args.len() > 1 {
        match args[1].parse::<u16>() {
            Ok(p) => p,
            Err(_) => {
                eprintln!("Invalid port number, using default 8080");
                8080
            }
        }
    } else {
        8080
    };

    let addr = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&addr).await?;
    println!("WebSocket server listening on: {}", addr);

    // Create our shared topic registry
    let topic_registry = Arc::new(Mutex::new(TopicRegistry::new()));

    while let Ok((stream, addr)) = listener.accept().await {
        let registry = topic_registry.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, addr, registry).await {
                eprintln!("Error handling connection from {}: {}", addr, e);
            }
        });
    }

    Ok(())
}

async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    registry: Arc<Mutex<TopicRegistry>>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Incoming connection from: {}", addr);

    // Upgrade connection to WebSocket
    let ws_stream = tokio_tungstenite::accept_async(stream).await?;
    println!("WebSocket connection established: {}", addr);

    // Create channel for sending messages to this client
    let (client_tx, mut client_rx) = mpsc::unbounded_channel();

    // Split the WebSocket
    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    // Handle incoming messages from WebSocket
    let registry_clone = registry.clone();
    let client_tx_clone = client_tx.clone();

    // Spawn task for forwarding messages from client_rx to WebSocket
    let forward_task = tokio::spawn(async move {
        while let Some(msg) = client_rx.recv().await {
            if ws_tx.send(Message::Text(msg)).await.is_err() {
                break;
            }
        }
    });

    // Create a collection of topics this client has subscribed to
    let mut client_topics = Vec::new();

    // Process incoming WebSocket messages
    while let Some(result) = ws_rx.next().await {
        let msg = match result {
            Ok(msg) => msg,
            Err(e) => {
                eprintln!("Error receiving message from {}: {}", addr, e);
                break;
            }
        };

        if let Message::Text(text) = msg {
            match parse_client_message(&text) {
                ClientMessage::Subscribe(topic) => {
                    if topic.is_empty() {
                        if client_tx_clone
                            .send("Error: Empty topic name".to_string())
                            .is_err()
                        {
                            break;
                        }
                        continue;
                    }

                    println!("Client {} subscribing to topic: {}", addr, topic);

                    // Add to client's topic list
                    client_topics.push(topic.clone());

                    // Add client to the topic registry
                    {
                        let mut registry = registry_clone.lock().unwrap();
                        registry.subscribe(topic.clone(), client_tx_clone.clone());
                    }

                    // Send confirmation
                    if client_tx_clone
                        .send("RESULT:subscribed".to_string())
                        .is_err()
                    {
                        break;
                    }
                }

                ClientMessage::Publish(proposal) => {
                    let topic = format!("{}:{}", proposal.input().asset, proposal.output().asset);
                    let content = format!("{}", proposal);

                    let sent_count = {
                        let mut registry = registry_clone.lock().unwrap();
                        registry.publish(&topic, content)
                    };

                    // Send confirmation
                    if client_tx_clone
                        .send(format!(
                            "RESULT:Message sent to {} subscribers on topic: {}",
                            sent_count, topic
                        ))
                        .is_err()
                    {
                        break;
                    }
                }

                ClientMessage::Error(error) => {
                    if client_tx_clone.send(format!("ERROR:{}", error)).is_err() {
                        break;
                    }
                }
            }
        }
    }

    println!("WebSocket connection closed: {}", addr);

    // Clean up by dropping the sender, which will cause the forward task to terminate
    drop(client_tx_clone);

    // Wait for the forward task to complete
    let _ = forward_task.await;

    Ok(())
}
