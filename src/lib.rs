use clap::Parser;
use elements::bitcoin::NetworkKind;
use elements::AddressParams;
use futures_util::{SinkExt, StreamExt};
use jsonrpc_lite::JsonRpc;
use message::{Error, Message, Methods};
use node::Node;
use serde_json::Value;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as TokioMessage;

use crate::jsonrpc::{parse_id, parse_method};

pub mod jsonrpc;
pub mod message;
pub mod node;
// pub mod proposal;
// pub mod zmq;

/// Configuration for Nexus Relay
#[derive(Parser, Debug)]
pub struct Config {
    /// Port number to listen on
    #[clap(long, default_value = "8080")]
    pub port: u16,

    /// Base URL for the client
    #[clap(long, default_value = "http://localhost:8332")]
    pub base_url: String,

    /// ZMQ endpoint
    #[clap(long, default_value = "tcp://127.0.0.1:29000")]
    pub zmq_endpoint: String,

    /// Network to use
    #[clap(long, value_enum, default_value = "liquid")]
    pub network: Network,
}

#[derive(Clone, clap::ValueEnum, Debug, PartialEq, Eq, Copy)]
pub enum Network {
    Liquid,
    LiquidTestnet,
    ElementsRegtest,
}

impl Network {
    pub fn as_network_kind(&self) -> NetworkKind {
        match self {
            Network::Liquid => NetworkKind::Main,
            _ => NetworkKind::Test,
        }
    }

    pub fn default_elements_listen_port(&self) -> u16 {
        match self {
            Network::Liquid => 7041,
            Network::LiquidTestnet => 7039,
            Network::ElementsRegtest => 7043, // TODO: check this
        }
    }

    pub fn default_listen_port(&self) -> u16 {
        match self {
            Network::Liquid => 3100,
            Network::LiquidTestnet => 3101,
            Network::ElementsRegtest => 3102,
        }
    }

    fn address_params(&self) -> &'static AddressParams {
        match self {
            Network::Liquid => &AddressParams::LIQUID,
            Network::LiquidTestnet => &AddressParams::LIQUID_TESTNET,
            Network::ElementsRegtest => &AddressParams::ELEMENTS,
        }
    }
}

// Our global state to track topic subscribers
pub struct TopicRegistry {
    topics: HashMap<Topic, Vec<mpsc::UnboundedSender<String>>>,
}

#[derive(Eq, PartialEq, Hash, Debug)]
pub enum Topic {
    Validated(String),
    Unvalidated(String),
}

impl Default for TopicRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TopicRegistry {
    pub fn new() -> Self {
        Self {
            topics: HashMap::new(),
        }
    }

    // Add a subscriber to a topic
    pub fn subscribe(&mut self, topic: Topic, sender: mpsc::UnboundedSender<String>) {
        self.topics.entry(topic).or_default().push(sender);
    }

    // Send a message to all subscribers of a topic
    pub fn publish(&mut self, topic: Topic, message: Message) -> usize {
        let subscribers = self.topics.entry(topic).or_default();
        let mut sent_count = 0;

        // Remove subscribers that are closed
        subscribers.retain(|sender| {
            match sender.send(message.to_string()) {
                Ok(_) => {
                    sent_count += 1;
                    true
                }
                Err(_) => false, // Receiver was dropped
            }
        });

        sent_count
    }
}

// The actual async implementation
pub async fn async_main(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let addr = format!("0.0.0.0:{}", config.port);
    let listener = TcpListener::bind(&addr).await?;
    log::info!("WebSocket server listening on: {}", addr);
    log::info!("Client connecting to: {}", config.base_url);
    log::info!("ZMQ subscriber listening on: {}", config.zmq_endpoint);
    log::info!("Using network: {:?}", config.network);

    // Create our shared topic registry
    let topic_registry = Arc::new(Mutex::new(TopicRegistry::new()));

    // Create our shared client - no need for Mutex since methods only take &self
    let client = Arc::new(Node::new(config.base_url));

    // Start ZMQ listener
    let registry_clone = topic_registry.clone();
    let zmq_endpoint_clone = config.zmq_endpoint.clone();
    let network_clone = config.network;
    // tokio::spawn(async move {
    //     if let Err(e) =
    //         zmq::start_zmq_listener(registry_clone, &zmq_endpoint_clone, network_clone).await
    //     {
    //         log::error!("Error in ZMQ listener: {}", e);
    //     }
    // });

    while let Ok((stream, addr)) = listener.accept().await {
        let registry = topic_registry.clone();
        let client_clone = client.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, addr, registry, client_clone).await {
                log::error!("Error handling connection from {}: {}", addr, e);
            }
        });
    }

    Ok(())
}

// Process a message and return the response to send back to the client
pub async fn process_message(
    message_request: JsonRpc,
    registry: Arc<Mutex<TopicRegistry>>,
    client: Option<&Node>,
    client_tx_clone: &mpsc::UnboundedSender<String>,
) -> Result<JsonRpc, Box<dyn std::error::Error>> {
    let method = parse_method(&message_request).ok_or(Error::NotImplemented)?;
    match method {
        Methods::Ping => {
            let id = parse_id(message_request.get_id().ok_or(Error::InvalidId)?)
                .ok_or(Error::InvalidId)?;
            let response = JsonRpc::success(id, &Value::String("pong".to_string()));
            Ok(response)
        }
        _ => Err(Box::new(Error::NotImplemented)),
    }
    //     MessageType::PublishAny => {
    //         process_publish_any(message_request, registry, message_request.random_id)
    //     }
    //     MessageType::PublishProposal => {
    //         proposal::process_publish_proposal(
    //             message_request,
    //             registry,
    //             client,
    //             message_request.random_id,
    //         )
    //         .await
    //     }
    //     MessageType::Subscribe => {
    //         subscribe_to_topic(message_request, registry, client_tx_clone, true)
    //     }
    //     MessageType::SubscribeAny => {
    //         subscribe_to_topic(message_request, registry, client_tx_clone, false)
    //     }
    //     MessageType::Result => Err(Box::new(Error::ResponseMessageUsedAsRequest)),
    //     MessageType::Error => Err(Box::new(Error::ResponseMessageUsedAsRequest)),
    //     MessageType::Ping => {
    //         let message_response = Message::new(MessageType::Pong, message_request.random_id, "");
    //         Ok(message_response)
    //     }
    //     MessageType::Pong => Err(Box::new(Error::ResponseMessageUsedAsRequest)),
    //     MessageType::PublishPset => Err(Box::new(Error::NotImplemented)),
    //     MessageType::Publish => Err(Box::new(Error::NotImplemented)),

    //     MessageType::Ack => Err(Box::new(Error::ResponseMessageUsedAsRequest)),
    // }
}

// fn subscribe_to_topic<'a>(
//     message_request: &'a Message<'a>,
//     registry: Arc<Mutex<TopicRegistry>>,
//     client_tx: &mpsc::UnboundedSender<String>,
//     is_validated: bool,
// ) -> Result<Message<'a>, Box<dyn std::error::Error>> {
//     let topic = message_request.content().to_string();
//     if topic.is_empty() {
//         return Err(Box::new(Error::MissingTopic));
//     }
//     if topic.len() > 129 {
//         return Err(Box::new(Error::InvalidTopic));
//     }

//     // Lock the mutex only when needed and release it immediately
//     {
//         let mut registry_guard = registry.lock().unwrap();
//         let topic = if is_validated {
//             Topic::Validated(topic)
//         } else {
//             Topic::Unvalidated(topic)
//         };
//         registry_guard.subscribe(topic, client_tx.clone());
//     }

//     let message_response = Message::ack(message_request.random_id);
//     Ok(message_response)
// }

pub async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    registry: Arc<Mutex<TopicRegistry>>,
    client: Arc<Node>,
) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Incoming connection from: {}", addr);

    // Upgrade connection to WebSocket
    let ws_stream = tokio_tungstenite::accept_async(stream).await?;
    log::info!("WebSocket connection established: {}", addr);

    // Create channel for sending messages to this client
    let (client_tx, mut client_rx) = mpsc::unbounded_channel();

    // Split the WebSocket
    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    // Handle incoming messages from WebSocket
    let registry_clone = registry.clone();
    let client_clone = client.clone();
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
                log::error!("Error receiving message from {}: {}", addr, e);
                break;
            }
        };

        if let TokioMessage::Text(text) = msg {
            let raw_message = match JsonRpc::parse(&text) {
                Ok(msg) => msg,
                Err(_) => {
                    let rpc_error = jsonrpc_lite::Error::new(jsonrpc_lite::ErrorCode::ParseError);
                    let err = JsonRpc::error(0, rpc_error);
                    let err_str = serde_json::to_string(&err).unwrap();
                    if client_tx_clone.send(err_str).is_err() {
                        break;
                    }
                    continue;
                }
            };

            // Process the message and get response
            let response = match process_message(
                raw_message,
                registry_clone.clone(),
                Some(&client_clone),
                &client_tx_clone,
            )
            .await
            {
                Ok(response) => serde_json::to_string(&response).unwrap(),
                Err(e) => {
                    let rpc_error =
                        jsonrpc_lite::Error::new(jsonrpc_lite::ErrorCode::InternalError);
                    let err = JsonRpc::error((), rpc_error);
                    let err_str = serde_json::to_string(&err).unwrap();
                    err_str
                }
            };

            // Send response back to client
            if client_tx_clone.send(response).is_err() {
                break;
            }
        }
    }

    log::info!("WebSocket connection closed: {}", addr);

    // Clean up by dropping the sender, which will cause the forward task to terminate
    drop(client_tx_clone);

    // Wait for the forward task to complete
    let _ = forward_task.await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Message, Methods};
    use tokio::runtime::Runtime;

    fn proposal_str() -> &'static str {
        include_str!("../test_data/proposal.json")
    }

    // async fn process_message_test<'a>(
    //     message_request: &'a Message<'a>,
    //     registry: Arc<Mutex<TopicRegistry>>,
    // ) -> String {
    //     let (client_tx, _client_rx) = mpsc::unbounded_channel();

    //     match process_message(message_request, registry, None, &client_tx).await {
    //         Ok(response) => response.to_string(),
    //         Err(e) => {
    //             let error_string = e.to_string();
    //             Message::new(MessageType::Error, message_request.random_id, &error_string)
    //                 .to_string()
    //         }
    //     }
    // }

    // #[test]
    // fn test_process_message_not_properly_formatted() {
    //     // TODO refactor out a fn process_str_message() which takes a string and returns a Message so that this is unit testable
    // }

    // #[test]
    // fn test_process_message_ping() {
    //     // Create a runtime
    //     let rt = Runtime::new().unwrap();

    //     // Create test message and registry
    //     let message_str = "PING||||";
    //     let raw_message = Message::parse(message_str).unwrap();
    //     let registry = Arc::new(Mutex::new(TopicRegistry::new()));
    //     let (client_tx, _client_rx) = mpsc::unbounded_channel();

    //     // Process the message
    //     let response = rt
    //         .block_on(process_message(&raw_message, registry, None, &client_tx))
    //         .unwrap();

    //     // Verify response
    //     assert_eq!(response.to_string(), "PONG||||");
    // }

    // #[test]
    // fn test_process_message_subscribe() {
    //     // Create a runtime
    //     let rt = Runtime::new().unwrap();

    //     // Create test message with a topic
    //     let message_str = "SUBSCRIBE||1|6|topic1";
    //     let raw_message = Message::parse(message_str).unwrap();
    //     let registry = Arc::new(Mutex::new(TopicRegistry::new()));
    //     let (client_tx, _client_rx) = mpsc::unbounded_channel();

    //     // Process the message
    //     let response = rt
    //         .block_on(process_message(&raw_message, registry, None, &client_tx))
    //         .unwrap();

    //     // Verify response
    //     assert_eq!(response.to_string(), "ACK||1||");

    //     // TODO verify the send to subscribers
    // }

    // #[test]
    // fn test_process_message_subscribe_empty_topic() {
    //     // Create a runtime
    //     let rt = Runtime::new().unwrap();

    //     // Create test message with empty topic
    //     let message_str = "SUBSCRIBE||1|0|";
    //     let raw_message = Message::parse(message_str).unwrap();
    //     let registry = Arc::new(Mutex::new(TopicRegistry::new()));

    //     // Process the message
    //     let err = rt.block_on(process_message_test(&raw_message, registry));

    //     // Verify response
    //     assert_eq!(err, "ERROR||1|13|Missing topic");
    // }

    // #[tokio::test]
    // async fn test_subscribe_publish() {
    //     env_logger::init();

    //     let id1 = 0;
    //     let id2 = 1;
    //     let id3 = 2;
    //     let id4 = 3;

    //     let proposal_json = proposal_str();
    //     let message_publish = format!(
    //         "PUBLISH_PROPOSAL||{id1}|{}|{}",
    //         proposal_json.len(),
    //         proposal_json
    //     );
    //     let message_publish = Message::parse(&message_publish).unwrap();

    //     // Get the proposal topic
    //     let proposal = message_publish.proposal().unwrap();
    //     let validated = proposal.insecure_validate().unwrap();
    //     let topic = proposal_topic(&validated).unwrap();

    //     // Create registry
    //     let registry = Arc::new(Mutex::new(TopicRegistry::new()));

    //     // Create channels for all clients
    //     let (client_subscribe_tx, mut client_subscribe_rx) = mpsc::unbounded_channel();
    //     let (client_subscribe_any_tx, mut client_subscribe_any_rx) = mpsc::unbounded_channel();
    //     let (client_publish_proposal_tx, _) = mpsc::unbounded_channel();
    //     let (client_publish_any_tx, _) = mpsc::unbounded_channel();

    //     // Step 1: Set up subscriptions first

    //     // SUBSCRIBE client: Subscribe to the validated topic
    //     let message_str = format!("SUBSCRIBE||{id2}|129|{topic}");
    //     let message = Message::parse(&message_str).unwrap();

    //     let message_response =
    //         process_message(&message, registry.clone(), None, &client_subscribe_tx)
    //             .await
    //             .unwrap();
    //     assert_eq!(message_response.to_string(), format!("ACK||{id2}||"));

    //     // SUBSCRIBE_ANY client: Subscribe to the same topic with SUBSCRIBE_ANY (unvalidated)
    //     let message_str = format!("SUBSCRIBE_ANY||{id3}|129|{topic}");
    //     let message = Message::parse(&message_str).unwrap();

    //     let message_response =
    //         process_message(&message, registry.clone(), None, &client_subscribe_any_tx)
    //             .await
    //             .unwrap();
    //     assert_eq!(message_response.to_string(), format!("ACK||{id3}||"));

    //     // Step 2: Test proposal publishing

    //     // Publish the proposal
    //     let message_response = process_message(
    //         &message_publish,
    //         registry.clone(),
    //         None,
    //         &client_publish_proposal_tx,
    //     )
    //     .await
    //     .unwrap();
    //     assert_eq!(message_response.to_string(), format!("ACK||{id1}||"));

    //     // SUBSCRIBE client should receive the proposal
    //     let message_received = client_subscribe_rx.recv().await.unwrap();

    //     let parsed_message = Message::parse(&message_received).unwrap();
    //     assert_eq!(parsed_message.type_, MessageType::Result);

    //     // Parse both strings to ensure we compare JSON content, not formatting
    //     let json1: serde_json::Value = serde_json::from_str(parsed_message.content()).unwrap();
    //     let json2: serde_json::Value = serde_json::from_str(proposal_str()).unwrap();
    //     assert_eq!(json1, json2);

    //     // SUBSCRIBE_ANY client should NOT receive the proposal
    //     assert!(
    //         client_subscribe_any_rx.try_recv().is_err(),
    //         "SUBSCRIBE_ANY client incorrectly received the proposal"
    //     );

    //     // Publish a message with PUBLISH_ANY
    //     let content = "Hello";
    //     let publish_content = format!("{}|{}", topic, content);
    //     let message_str = format!(
    //         "PUBLISH_ANY||{id4}|{}|{}",
    //         publish_content.len(),
    //         publish_content
    //     );
    //     let message = Message::parse(&message_str).unwrap();

    //     let message_response = process_message(&message, registry, None, &client_publish_any_tx)
    //         .await
    //         .unwrap();
    //     assert_eq!(message_response.to_string(), format!("ACK||{id4}||"));

    //     // SUBSCRIBE client should NOT receive the PUBLISH_ANY message
    //     assert!(
    //         client_subscribe_rx.try_recv().is_err(),
    //         "SUBSCRIBE client incorrectly received the PUBLISH_ANY message"
    //     );

    //     // SUBSCRIBE_ANY client should receive the PUBLISH_ANY message
    //     let message_received = client_subscribe_any_rx.recv().await.unwrap();

    //     let parsed_message = Message::parse(&message_received).unwrap();
    //     assert_eq!(parsed_message.type_, MessageType::Result);
    //     assert_eq!(parsed_message.content(), content);
    // }
}

// Process a publish any message
// fn process_publish_any<'a>(
//     message_request: &'a Message<'a>,
//     registry: Arc<Mutex<TopicRegistry>>,
//     random_id: Option<u64>,
// ) -> Result<Message<'a>, Box<dyn std::error::Error>> {
//     let (topic, content) = message_request.topic_content()?;

//     // Log the raw topic and content to help debug
//     log::debug!("PublishAny raw topic: '{}', content: '{}'", topic, content);

//     let message_to_subscriber = Message::new(Methods::Result, None, content);

//     // Lock the mutex only when needed and release it immediately
//     let sent_count = {
//         let mut registry_guard = registry.lock().unwrap();
//         let unvalidated_topic = Topic::Unvalidated(topic.to_string());

//         // Log the registered topics for comparison
//         for (existing_topic, subscribers) in &registry_guard.topics {
//             match existing_topic {
//                 Topic::Unvalidated(t) => log::debug!(
//                     "Found registered Unvalidated topic: '{}' with {} subscribers",
//                     t,
//                     subscribers.len()
//                 ),
//                 Topic::Validated(t) => log::debug!(
//                     "Found registered Validated topic: '{}' with {} subscribers",
//                     t,
//                     subscribers.len()
//                 ),
//             }
//         }

//         registry_guard.publish(unvalidated_topic, message_to_subscriber)
//     };

//     log::info!(
//         "Message sent to {} subscribers on topic: {}",
//         sent_count,
//         topic
//     );
//     let message_response = Message::ack(random_id);
//     Ok(message_response)
// }
