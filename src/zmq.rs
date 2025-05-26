use crate::message::{Message, MessageType};
use crate::{Network, TopicRegistry};
use elements::encode::Decodable;
use futures_util::StreamExt;
use log::{error, info};
use std::sync::{Arc, Mutex};
use tmq::{subscribe, Context, Multipart};

/// Process a single ZMQ message.
///
/// This function handles the raw message content and processes it based on the topic.
pub fn process_zmq_message(
    msg: Multipart,
    registry: &Arc<Mutex<TopicRegistry>>,
    network: &Network,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut iter = msg.iter().map(|item| item.as_ref());
    let topic = iter.next().unwrap();
    let tx = iter.next().unwrap();
    // TODO check there are no other parts of the message
    if topic == b"rawtx" {
        let tx = elements::Transaction::consensus_decode(tx)?;
        let txid = tx.txid().to_string();

        log::info!("Processing ZMQ message with txid: {}", txid);

        for out in tx.output {
            if let Some(addr) =
                elements::Address::from_script(&out.script_pubkey, None, network.address_params())
            {
                // Get the address as a string to use as the topic
                let address_str = addr.to_string();

                if let Ok(mut registry) = registry.lock() {
                    // Use the txid as the content
                    let content = txid.clone();

                    // Create a message to publish
                    let message = Message::new(MessageType::Result, None, &content);

                    log::info!("Publishing message to address: {}", address_str);
                    // Publish using the address as the topic
                    registry.publish(&address_str, message);
                } else {
                    error!("Failed to lock registry");
                }
            }
        }
    }

    Ok(())
}

pub async fn start_zmq_listener(
    registry: Arc<Mutex<TopicRegistry>>,
    endpoint: &str,
    network: Network,
) -> Result<(), Box<dyn std::error::Error>> {
    let ctx = Context::new();

    // Create the subscriber socket correctly
    let socket_builder = subscribe(&ctx);
    let socket_builder = socket_builder.connect(endpoint)?;
    let mut socket = socket_builder.subscribe(b"rawtx").unwrap();

    info!("Async ZMQ subscriber listening on {}", endpoint);

    // Process messages asynchronously
    while let Some(msg) = socket.next().await {
        match msg {
            Ok(multipart) => {
                if let Err(e) = process_zmq_message(multipart, &registry, &network) {
                    error!("Error processing ZMQ message: {}", e);
                }
            }
            Err(e) => {
                error!("Error receiving ZMQ message: {}", e);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use elements::hashes::{hash160, Hash};
    use elements::{encode::Encodable, Script};
    use std::collections::VecDeque;
    use std::time::Duration;
    use tokio::sync::mpsc;

    #[test]
    fn test_process_zmq_message() {
        // Create a P2PKH script (pay to public key hash)
        let hash_bytes = [
            0x16u8, 0xe1, 0xae, 0x70, 0xff, 0x0f, 0xa1, 0x02, 0x90, 0x5d, 0x4a, 0xf2, 0x97, 0xf6,
            0x91, 0x2b, 0xda, 0x6c, 0xce, 0x19,
        ];
        let hash = hash160::Hash::from_slice(&hash_bytes).unwrap();
        let p2pkh_script = Script::new_p2pkh(&elements::PubkeyHash::from_raw_hash(hash));
        let address = "2dbWjcMkJE1fHvuSxjEafEa3rctBAfpeysW";

        let tx_hex = "0200000001010000000000000000000000000000000000000000000000000000000000000000ffffffff0401650101ffffffff020125b251070e29ca19043cf33ccd7324e2ddab03ecc4ae0b5e77c4fc0e5cf6c95a01000000000000000000016a0125b251070e29ca19043cf33ccd7324e2ddab03ecc4ae0b5e77c4fc0e5cf6c95a01000000000000000000266a24aa21a9ed94f15ed3a62165e4a0b99699cc28b48e19cb5bc1b1f47155db62d63f1e047d45000000000000012000000000000000000000000000000000000000000000000000000000000000000000000000";
        // Create a simple Elements transaction
        let mut tx =
            elements::Transaction::consensus_decode(&hex::decode(tx_hex).unwrap()[..]).unwrap();

        // Replace the script_pubkey with our P2PKH script
        tx.output[0].script_pubkey = p2pkh_script;

        // Serialize the transaction
        let mut tx_bytes = Vec::new();
        tx.consensus_encode(&mut tx_bytes).unwrap();

        // Create a mock ZMQ message
        let topic = tmq::Message::from(b"rawtx".to_vec());
        let data = tmq::Message::from(tx_bytes);

        let mut parts = VecDeque::new();
        parts.push_back(topic);
        parts.push_back(data);
        let multipart = Multipart(parts);

        // Create a registry
        let registry = Arc::new(Mutex::new(TopicRegistry::new()));

        // Create a channel to receive messages
        let (tx_sender, mut rx_receiver) = mpsc::unbounded_channel();

        // Subscribe to the address
        {
            let mut registry_guard = registry.lock().unwrap();
            registry_guard.subscribe(address.to_string(), tx_sender);
        }

        // Process the message
        let result = process_zmq_message(multipart, &registry, &Network::ElementsRegtest);

        // Verify the processing succeeded
        assert!(result.is_ok());

        // Get the txid
        let txid = tx.txid().to_string();

        // Check if a message was published
        let (sender, receiver) = std::sync::mpsc::channel::<String>();

        // Create a tokio runtime for the async receiver
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            if let Some(message) = rx_receiver.recv().await {
                sender.send(message).unwrap();
            }
        });

        // Wait for the message with a timeout
        let received_message = receiver.recv_timeout(Duration::from_secs(1));
        assert!(
            received_message.is_ok(),
            "No message was published to the address"
        );

        // Parse the message and verify it contains the txid
        let message = received_message.unwrap();
        let parsed = Message::parse(&message).unwrap();
        assert_eq!(parsed.type_, MessageType::Result);
        assert_eq!(parsed.content(), txid);
    }
}
