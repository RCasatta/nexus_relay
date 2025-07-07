use clap::Parser;
use elements::bitcoin::NetworkKind;
use elements::AddressParams;
use futures_util::{SinkExt, StreamExt};
use jsonrpc_lite::JsonRpc;
use node::Node;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as TokioMessage;

use crate::error::Error;
use crate::jsonrpc::{Method, NexusRequest, NexusResponse, Params};

pub mod error;
pub mod jsonrpc;
pub mod node;
pub mod proposal;
pub mod zmq;

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

#[derive(Eq, PartialEq, Hash, Debug, Clone)]
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
    pub fn publish(&mut self, topic: Topic, message: String) -> usize {
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
    tokio::spawn(async move {
        if let Err(e) =
            zmq::start_zmq_listener(registry_clone, &zmq_endpoint_clone, network_clone, 1000).await
        {
            log::error!("Error in ZMQ listener: {}", e);
        }
    });

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
) -> Result<NexusResponse, Error> {
    log::debug!("Processing jsonrpc: {:?}", message_request);
    let request = NexusRequest::try_from(message_request)?;
    log::debug!("Processing message: {:?}", request);

    let response = match request.method {
        Method::Subscribe => {
            let topic = request.topic()?;
            subscribe_to_topic(request.id, topic, registry, client_tx_clone)?
        }
        Method::Publish => match request.params {
            Params::Ping(()) => NexusResponse::new_pong(request.id),
            Params::Proposal(proposal) => {
                proposal::process_publish_proposal(proposal, registry, client, request.id).await?
            }
            Params::Any(any) => {
                // Handle publishing arbitrary content to topics
                let topic = Topic::Unvalidated(any.topic);
                let content = any
                    .content
                    .ok_or(Error::JsonRpc(jsonrpc::Error::InvalidParams))?;

                let sent_count = {
                    let mut registry_guard = registry.lock().unwrap();
                    let response = NexusResponse::notification(serde_json::Value::String(content));
                    registry_guard.publish(topic, response.to_string())
                };

                log::debug!("Published to {} subscribers", sent_count);
                NexusResponse::new_published(request.id)
            }
            _ => return Err(Error::JsonRpc(jsonrpc::Error::InvalidParams)),
        },
        _ => return Err(Error::JsonRpc(jsonrpc::Error::InvalidParams)),
    };
    log::debug!("Response: {:?}", response);
    Ok(response)
}

fn subscribe_to_topic(
    id: u32,
    topic: Topic,
    registry: Arc<Mutex<TopicRegistry>>,
    client_tx: &mpsc::UnboundedSender<String>,
) -> Result<NexusResponse, Error> {
    let mut registry_guard = registry.lock().unwrap();
    registry_guard.subscribe(topic, client_tx.clone());

    let message_response = NexusResponse::new_subscribed(id);
    Ok(message_response)
}

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
            let response: JsonRpc = match process_message(
                raw_message,
                registry_clone.clone(),
                Some(&client_clone),
                &client_tx_clone,
            )
            .await
            {
                Ok(response) => response.into(),
                Err(e) => match e {
                    Error::JsonRpc(e) => e.into(),
                    _ => jsonrpc_lite::JsonRpc::error(
                        0,
                        jsonrpc_lite::Error::new(jsonrpc_lite::ErrorCode::ParseError), // TODO
                    ),
                },
            };
            let response_str = serde_json::to_string(&response).unwrap();

            // Send response back to client
            if client_tx_clone.send(response_str).is_err() {
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
    use jsonrpc_lite::Id;
    use tokio::runtime::Runtime;

    fn proposal_str() -> &'static str {
        include_str!("../test_data/proposal.json")
    }

    #[test]
    fn test_process_message_ping() {
        // Create a runtime
        let rt = Runtime::new().unwrap();

        // Create test message and registry
        let message = NexusRequest::new_ping(12341234);
        assert_eq!(
            message.to_string(),
            "{\"jsonrpc\":\"2.0\",\"method\":\"publish\",\"params\":{\"ping\":null},\"id\":12341234}"
        );
        let raw_message = JsonRpc::parse(&message.to_string()).unwrap();
        let registry = Arc::new(Mutex::new(TopicRegistry::new()));
        let (client_tx, _client_rx) = mpsc::unbounded_channel();

        // Process the message
        let response = rt
            .block_on(process_message(raw_message, registry, None, &client_tx))
            .unwrap();

        // Verify response
        assert_eq!(
            response.to_string(),
            "{\"jsonrpc\":\"2.0\",\"result\":\"pong\",\"id\":12341234}"
        );
    }

    #[test]
    fn test_process_message_subscribe() {
        // Create a runtime
        let rt = Runtime::new().unwrap();

        // Create test message with a topic using the new JSON-RPC format
        let message_json = r#"{
            "id": 1,
            "jsonrpc": "2.0",
            "method": "subscribe",
            "params": {
                "any": {
                    "topic": "topic1"
                }
            }
        }"#;
        let raw_message = JsonRpc::parse(message_json).unwrap();
        let registry = Arc::new(Mutex::new(TopicRegistry::new()));
        let (client_tx, _client_rx) = mpsc::unbounded_channel();

        // Process the message
        let response = rt
            .block_on(process_message(raw_message, registry, None, &client_tx))
            .unwrap();

        // Verify response - should be subscribed confirmation in JSON-RPC format
        assert_eq!(
            response.to_string(),
            "{\"jsonrpc\":\"2.0\",\"result\":\"subscribed\",\"id\":1}"
        );

        // TODO verify the send to subscribers
    }

    #[test]
    fn test_process_message_subscribe_empty_topic() {
        // Create a runtime
        let rt = Runtime::new().unwrap();

        // Create test message with missing params to trigger an error
        let message_json = r#"{
            "id": 1,
            "jsonrpc": "2.0",
            "method": "subscribe"
        }"#;
        let raw_message = JsonRpc::parse(message_json).unwrap();
        let registry = Arc::new(Mutex::new(TopicRegistry::new()));
        let (client_tx, _client_rx) = mpsc::unbounded_channel();

        // Process the message - this should now fail due to missing parameters
        let response = rt.block_on(process_message(raw_message, registry, None, &client_tx));

        // Verify response is an error
        assert!(response.is_err(), "Expected error but got success");
        let error = response.unwrap_err();
        match error {
            Error::JsonRpc(jsonrpc::Error::InvalidParamsForThisMethod) => {
                // This is the expected error for empty params with subscribe method
            }
            _ => panic!(
                "Expected InvalidParamsForThisMethod error, got: {:?}",
                error
            ),
        }
    }

    #[tokio::test]
    async fn test_subscribe_publish() {
        env_logger::init();

        // Create registry
        let registry = Arc::new(Mutex::new(TopicRegistry::new()));

        // Create channels for all clients
        let (client_subscribe_tx, mut client_subscribe_rx) = mpsc::unbounded_channel();
        let (client_subscribe_any_tx, mut client_subscribe_any_rx) = mpsc::unbounded_channel();
        let (client_publish_proposal_tx, _) = mpsc::unbounded_channel();
        let (client_publish_any_tx, _) = mpsc::unbounded_channel();

        // Step 1: Set up subscriptions first

        // Get the topic from a test proposal
        // Use a simple mock proposal instead of the complex one to test parsing
        let proposal_json = proposal_str();
        let proposal: lwk_wollet::LiquidexProposal<lwk_wollet::Unvalidated> =
            std::str::FromStr::from_str(proposal_json).unwrap();
        let validated = proposal.insecure_validate().unwrap();
        let topic = crate::jsonrpc::topic_from_proposal(&validated).unwrap();
        let topic_str = match &topic {
            Topic::Validated(s) => s.clone(),
            Topic::Unvalidated(s) => s.clone(),
        };

        // SUBSCRIBE client: Subscribe to the validated topic using "pair" subscription
        // This will create a validated topic subscription
        let subscribe_message = format!(
            r#"{{
            "id": 1,
            "jsonrpc": "2.0",
            "method": "subscribe",
            "params": {{
                "pair": {{
                    "input": "{}",
                    "output": "{}"
                }}
            }}
        }}"#,
            validated.input().asset,
            validated.output().asset
        );

        let raw_message = JsonRpc::parse(&subscribe_message).unwrap();
        let message_response =
            process_message(raw_message, registry.clone(), None, &client_subscribe_tx)
                .await
                .unwrap();
        assert_eq!(
            message_response.to_string(),
            "{\"jsonrpc\":\"2.0\",\"result\":\"subscribed\",\"id\":1}"
        );

        // SUBSCRIBE_ANY client: Subscribe to the same topic with "any" (unvalidated)
        let subscribe_any_message = format!(
            r#"{{
            "id": 2,
            "jsonrpc": "2.0",
            "method": "subscribe",
            "params": {{
                "any": {{
                    "topic": "{}"
                }}
            }}
        }}"#,
            topic_str
        );

        let raw_message = JsonRpc::parse(&subscribe_any_message).unwrap();
        let message_response = process_message(
            raw_message,
            registry.clone(),
            None,
            &client_subscribe_any_tx,
        )
        .await
        .unwrap();
        assert_eq!(
            message_response.to_string(),
            "{\"jsonrpc\":\"2.0\",\"result\":\"subscribed\",\"id\":2}"
        );

        // Step 2: Test proposal publishing

        // Parse the proposal as a JSON value first
        let proposal_value: serde_json::Value = serde_json::from_str(proposal_json).unwrap();

        // Create the JSON-RPC message properly
        let publish_proposal_json = serde_json::json!({
            "id": 3,
            "jsonrpc": "2.0",
            "method": "publish",
            "params": {
                "proposal": proposal_value
            }
        });
        let s = publish_proposal_json.to_string();
        println!("publish_proposal_json: {s}",);
        let raw_message = JsonRpc::parse(&s).unwrap();
        let message_response = process_message(
            raw_message,
            registry.clone(),
            None,
            &client_publish_proposal_tx,
        )
        .await
        .unwrap();
        assert_eq!(
            message_response.to_string(),
            "{\"jsonrpc\":\"2.0\",\"result\":\"published\",\"id\":3}"
        );

        // SUBSCRIBE client should receive the proposal
        let message_received = client_subscribe_rx.recv().await.unwrap();
        println!("message_received: {message_received}");

        // Parse the received message as JsonRpc to check it's a notification
        let parsed_notification: JsonRpc = serde_json::from_str(&message_received).unwrap();
        // Notifications have id = -1
        match parsed_notification.get_id() {
            Some(Id::Num(-1)) => {} // This is what we expect for notifications
            other => panic!("Expected notification with id = -1, got: {:?}", other),
        }

        // Extract the result and verify it's the proposal
        if let Some(result) = parsed_notification.get_result() {
            let result_str = result.to_string();
            // Parse both strings to ensure we compare JSON content, not formatting
            let json1: serde_json::Value = serde_json::from_str(&result_str).unwrap();
            let json2: serde_json::Value = serde_json::from_str(proposal_json).unwrap();
            assert_eq!(json1, json2);
        } else {
            panic!("Expected notification with result");
        }

        // SUBSCRIBE_ANY client should NOT receive the proposal (different topic types)
        assert!(
            client_subscribe_any_rx.try_recv().is_err(),
            "SUBSCRIBE_ANY client incorrectly received the proposal"
        );

        // Step 3: Test publishing arbitrary content

        // Publish a message with "any" content
        let content = "Hello World";
        let publish_any_message = format!(
            r#"{{
            "id": 4,
            "jsonrpc": "2.0",
            "method": "publish",
            "params": {{
                "any": {{
                    "topic": "{}",
                    "content": "{}"
                }}
            }}
        }}"#,
            topic_str, content
        );

        let raw_message = JsonRpc::parse(&publish_any_message).unwrap();
        let message_response = process_message(raw_message, registry, None, &client_publish_any_tx)
            .await
            .unwrap();
        assert_eq!(
            message_response.to_string(),
            "{\"jsonrpc\":\"2.0\",\"result\":\"published\",\"id\":4}"
        );

        // SUBSCRIBE client should NOT receive the ANY message (different topic types)
        assert!(
            client_subscribe_rx.try_recv().is_err(),
            "SUBSCRIBE client incorrectly received the ANY message"
        );

        // SUBSCRIBE_ANY client should receive the ANY message
        let message_received = client_subscribe_any_rx.recv().await.unwrap();

        // Parse the received message as JsonRpc to check it's a notification
        let parsed_notification: JsonRpc = serde_json::from_str(&message_received).unwrap();
        // Notifications have id = -1
        match parsed_notification.get_id() {
            Some(Id::Num(-1)) => {} // This is what we expect for notifications
            other => panic!("Expected notification with id = -1, got: {:?}", other),
        }

        // Extract the result and verify it's our content
        if let Some(result) = parsed_notification.get_result() {
            let result_str = result.as_str().unwrap();
            assert_eq!(result_str, content);
        } else {
            panic!("Expected notification with result");
        }
    }

    #[test]
    fn test_jsonrpc_lite() {
        let a = serde_json::json!({
            "id": 3,
            "jsonrpc": "2.0",
            "method": "publish",
            "params": {
                "proposal": "ciao"
            }
        });
        let _ = JsonRpc::parse(&a.to_string()).unwrap();

        let b = serde_json::json!({
            "id": 3,
            "jsonrpc": "2.0",
            "method": "publish",
            "params": {
                "proposal": a
            }
        });
        let _ = JsonRpc::parse(&b.to_string()).unwrap();

        let proposal_json = proposal_str();
        let proposal_value: serde_json::Value = serde_json::from_str(proposal_json).unwrap();
        let c = serde_json::json!({
            "id": 3,
            "jsonrpc": "2.0",
            "method": "publish",
            "params": {
                "proposal": proposal_value
            }
        });
        let _ = JsonRpc::parse(&c.to_string()).unwrap();
    }
}
