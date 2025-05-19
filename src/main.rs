use futures_util::{SinkExt, StreamExt};
use lwk_wollet::elements::AssetId;
use message::{proposal_topic, Error, Message, MessageType};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
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
    fn publish(&mut self, topic: &str, message: Message) -> usize {
        let subscribers = self
            .topics
            .entry(topic.to_string())
            .or_insert_with(Vec::new);
        let mut sent_count = 0;

        // Remove subscribers that are closed
        subscribers.retain(|sender| {
            let is_open = match sender.send(message.to_string()) {
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
    message_request: &'a Message<'a>,
    registry: &mut TopicRegistry,
    client_tx_clone: &mpsc::UnboundedSender<String>,
) -> Result<Message<'a>, Box<dyn std::error::Error>> {
    match message_request.type_ {
        MessageType::Publish => {
            let (topic, content) = message_request.topic_content()?;
            let message_to_subscriber = Message::new(MessageType::Result, None, &content);
            let sent_count = registry.publish(&topic, message_to_subscriber);
            println!(
                "Message sent to {} subscribers on topic: {}",
                sent_count, topic
            );
            let message_response = Message::new(
                MessageType::Result,
                message_request.random_id,
                "message published",
            );
            Ok(message_response)
        }
        MessageType::PublishProposal => {
            let proposal = message_request.proposal()?;
            let proposal = proposal.insecure_validate()?; // TODO: validate properly
            let topic = proposal_topic(&proposal)?;
            let proposal_str = format!("{}", proposal);
            let message_to_subscriber = Message::new(MessageType::Result, None, &proposal_str);
            let sent_count = registry.publish(&topic, message_to_subscriber);
            println!(
                "Message sent to {} subscribers on topic: {}",
                sent_count, topic
            );
            let message_response = Message::new(
                MessageType::Result,
                message_request.random_id,
                "proposal published",
            );
            Ok(message_response)
        }
        MessageType::Subscribe => {
            let topic = message_request.content().to_string();
            if topic.is_empty() {
                return Err(Box::new(Error::MissingTopic));
            }
            if topic.len() > 64 {
                // if the topic is longer than 64 chars, it must be a proposal topic
                let mut assets = topic.splitn(2, '|');
                if let (Some(asset1), Some(asset2)) = (assets.next(), assets.next()) {
                    let asset1 = AssetId::from_str(asset1);
                    let asset2 = AssetId::from_str(asset2);
                    if let (Ok(_), Ok(_)) = (asset1, asset2) {
                        // topic validated
                    } else {
                        return Err(Box::new(Error::InvalidTopic));
                    }
                } else {
                    return Err(Box::new(Error::InvalidTopic));
                }
            }

            registry.subscribe(topic, client_tx_clone.clone());
            let message_response =
                Message::new(MessageType::Result, message_request.random_id, "subscribed");
            Ok(message_response)
        }
        MessageType::Result => Err(Box::new(Error::ResponseMessageUsedAsRequest)),
        MessageType::Error => Err(Box::new(Error::ResponseMessageUsedAsRequest)),
        MessageType::Ping => {
            let message_response = Message::new(MessageType::Pong, message_request.random_id, "");
            Ok(message_response)
        }
        MessageType::Pong => Err(Box::new(Error::ResponseMessageUsedAsRequest)),
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
                    let error_string = e.to_string();
                    let message_response = Message::new(MessageType::Error, None, &error_string);
                    if client_tx_clone.send(message_response.to_string()).is_err() {
                        break;
                    }
                    continue;
                }
            };

            // Process the message and get response
            let response = {
                let mut registry = registry_clone.lock().unwrap();
                match process_message(&raw_message, &mut registry, &client_tx_clone) {
                    Ok(response) => response.to_string(),
                    Err(e) => {
                        let error_string = e.to_string();
                        Message::new(MessageType::Error, raw_message.random_id, &error_string)
                            .to_string()
                    }
                }
            };

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

    fn proposal_str() -> &'static str {
        include_str!("../test_data/proposal.json")
    }

    fn process_message_test<'a>(
        message_request: &'a Message<'a>,
        registry: &mut TopicRegistry,
    ) -> String {
        let (client_tx, _client_rx) = mpsc::unbounded_channel();

        match process_message(message_request, registry, &client_tx) {
            Ok(response) => response.to_string(),
            Err(e) => {
                let error_string = e.to_string();
                Message::new(MessageType::Error, message_request.random_id, &error_string)
                    .to_string()
            }
        }
    }

    #[test]
    fn test_process_message_not_properly_formatted() {

        // TODO refactor out a fn process_str_message() which takes a string and returns a Message so that this is unit testable
    }

    #[test]
    fn test_process_message_ping() {
        // Create test message and registry
        let message_str = "PING|||0|";
        let raw_message = Message::parse(message_str).unwrap();
        let mut registry = TopicRegistry::new();
        let (client_tx, _client_rx) = mpsc::unbounded_channel();

        // Process the message
        let response = process_message(&raw_message, &mut registry, &client_tx).unwrap();

        // Verify response
        assert_eq!(response.to_string(), "PONG|||0|");
    }

    #[test]
    fn test_process_message_subscribe() {
        // Create test message with a topic
        let message_str = "SUBSCRIBE||1|6|topic1";
        let raw_message = Message::parse(message_str).unwrap();
        let mut registry = TopicRegistry::new();
        let (client_tx, _client_rx) = mpsc::unbounded_channel();

        // Process the message
        let response = process_message(&raw_message, &mut registry, &client_tx).unwrap();

        // Verify response
        assert_eq!(response.to_string(), "RESULT||1|10|subscribed");

        // TODO verify the send to subscribers
    }

    #[test]
    fn test_process_message_subscribe_empty_topic() {
        // Create test message with empty topic
        let message_str = "SUBSCRIBE||1|0|";
        let raw_message = Message::parse(message_str).unwrap();
        let mut registry = TopicRegistry::new();

        // Process the message
        let err = process_message_test(&raw_message, &mut registry);

        // Verify response
        assert_eq!(err.to_string(), "ERROR||1|13|Missing topic");
    }

    #[test]
    fn test_subscribe_publish() {
        let id1 = 12341234;
        let id2 = 12341235;
        let proposal_json = proposal_str();
        let message_publish = format!(
            "PUBLISH_PROPOSAL||{id1}|{}|{}",
            proposal_json.len(),
            proposal_json
        );
        let message_publish = Message::parse(&message_publish).unwrap();
        let proposal = message_publish.proposal().unwrap();
        let validated = proposal.insecure_validate().unwrap();
        let topic = proposal_topic(&validated).unwrap();
        assert_eq!(topic, "6921c799f7b53585025ae8205e376bfd2a7c0571f781649fb360acece252a6a7|f13806d2ab6ef8ba56fc4680c1689feb21d7596700af1871aef8c2d15d4bfd28");

        // Subscribe to the topic of the proposal
        let message_str = format!("SUBSCRIBE||{id2}|129|{topic}");
        let message = Message::parse(&message_str).unwrap();
        let mut registry = TopicRegistry::new();
        let (client_tx1, mut client_rx1) = mpsc::unbounded_channel();
        // Process the message
        let message_response = process_message(&message, &mut registry, &client_tx1).unwrap();

        // Verify response
        assert_eq!(
            message_response.to_string(),
            format!("RESULT||{id2}|10|subscribed")
        );

        // Publish the proposal as another client
        let (client_tx2, _client_rx2) = mpsc::unbounded_channel();
        let message_response =
            process_message(&message_publish, &mut registry, &client_tx2).unwrap();
        assert_eq!(
            message_response.to_string(),
            format!("RESULT||{id1}|18|proposal published")
        );
        let message_received = client_rx1.blocking_recv().unwrap();
        let parsed_message = Message::parse(&message_received).unwrap();

        // Parse both strings to ensure we compare JSON content, not formatting
        let json1: serde_json::Value = serde_json::from_str(parsed_message.content()).unwrap();
        let json2: serde_json::Value = serde_json::from_str(proposal_str()).unwrap();

        assert_eq!(json1, json2);
    }
}
