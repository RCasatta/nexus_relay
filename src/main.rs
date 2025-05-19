use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use lwk_wollet::elements::AssetId;
use lwk_wollet::{LiquidexProposal, Validated};
use message::{Message, MessageType};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as TokioMessage;

mod message;

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

    let addr = format!("0.0.0.0:{}", port);
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

// Process a message and return the response to send back to the client
fn process_message<'a>(
    raw_message: &'a Message<'a>,
    registry: &mut TopicRegistry,
) -> Result<String, Box<dyn std::error::Error>> {
    match raw_message.type_ {
        MessageType::Publish => {
            todo!()
        }
        MessageType::PublishProposal => {
            let proposal = LiquidexProposal::from_str(raw_message.content)?;
            let proposal = proposal.insecure_validate()?;
            let topic = format!("{}:{}", proposal.input().asset, proposal.output().asset);
            let content = format!("{}", proposal);

            let sent_count = registry.publish(&topic, content);

            // Return confirmation message
            Ok(format!(
                "RESULT:Message sent to {} subscribers on topic: {}",
                sent_count, topic
            ))
        }
        MessageType::Subscribe => {
            let topic = raw_message.content.to_string();
            if topic.is_empty() {
                return Ok("Error: Empty topic name".to_string());
            }

            // Topic is valid, return success message
            Ok("RESULT:subscribed".to_string())
        }
        MessageType::Result => todo!(),
        MessageType::Ack => todo!(),
        MessageType::Error => Ok("ERROR".to_string()),
        MessageType::Ping => Ok("PONG".to_string()),
        MessageType::Pong => todo!(),
    }
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
            if ws_tx.send(TokioMessage::Text(msg)).await.is_err() {
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

        if let TokioMessage::Text(text) = msg {
            let raw_message = match Message::parse(&text) {
                Ok(msg) => msg,
                Err(e) => {
                    if client_tx_clone
                        .send(format!("Error parsing message: {}", e))
                        .is_err()
                    {
                        break;
                    }
                    continue;
                }
            };

            // Process the message and get response
            let response = {
                let mut registry = registry_clone.lock().unwrap();
                match process_message(&raw_message, &mut registry) {
                    Ok(response) => response,
                    Err(e) => format!("Error processing message: {}", e),
                }
            };

            // For subscription messages, update client's topics
            if raw_message.type_ == MessageType::Subscribe && !raw_message.content.is_empty() {
                let topic = raw_message.content.to_string();
                println!("Client {} subscribing to topic: {}", addr, topic);

                // Add to client's topic list
                client_topics.push(topic.clone());

                // Add client to the topic registry
                {
                    let mut registry = registry_clone.lock().unwrap();
                    registry.subscribe(topic, client_tx_clone.clone());
                }
            }

            // Send response back to client
            if client_tx_clone.send(response).is_err() {
                break;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Message, MessageType};

    #[test]
    fn test_process_message_ping() {
        // Create test message and registry
        let message_str = "PING|0|0|0|";
        let raw_message = Message::parse(message_str).unwrap();
        let mut registry = TopicRegistry::new();

        // Process the message
        let response = process_message(&raw_message, &mut registry).unwrap();

        // Verify response
        assert_eq!(response, "PONG");
    }

    #[test]
    fn test_process_message_subscribe() {
        // Create test message with a topic
        let message_str = "SUBSCRIBE|0|1|6|topic1";
        let raw_message = Message::parse(message_str).unwrap();
        let mut registry = TopicRegistry::new();

        // Process the message
        let response = process_message(&raw_message, &mut registry).unwrap();

        // Verify response
        assert_eq!(response, "RESULT:subscribed");
    }

    #[test]
    fn test_process_message_subscribe_empty_topic() {
        // Create test message with empty topic
        let message_str = "SUBSCRIBE|0|1|0|";
        let raw_message = Message::parse(message_str).unwrap();
        let mut registry = TopicRegistry::new();

        // Process the message
        let response = process_message(&raw_message, &mut registry).unwrap();

        // Verify response
        assert_eq!(response, "Error: Empty topic name");
    }
}
