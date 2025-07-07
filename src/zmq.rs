use crate::{error::Error, jsonrpc::NexusResponse, Network, Topic, TopicRegistry};
use elements::encode::Decodable;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex},
};
use tmq::{subscribe, Context, Multipart};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Where {
    Mempool,
    Block,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AddressSeen {
    address: String,

    #[serde(rename = "where")]
    where_: Where,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TxSeen {
    txid: String,

    #[serde(rename = "where")]
    where_: Where,
}

/// Extract output addresses from a transaction.
///
/// Given a borrowed transaction and network, returns a vector of addresses
/// as strings for all outputs that can be decoded to valid addresses.
pub fn get_output_addresses(tx: &elements::Transaction, network: &Network) -> Vec<String> {
    let mut addresses = Vec::new();

    for out in &tx.output {
        if let Some(addr) =
            elements::Address::from_script(&out.script_pubkey, None, network.address_params())
        {
            addresses.push(addr.to_string());
        }
    }

    addresses
}

/// Publish an AddressSeen notification to the registry.
///
/// This function creates an AddressSeen notification and publishes it to the registry
/// using the address as the topic.
fn publish_address_seen(registry: &Arc<Mutex<TopicRegistry>>, address: String, where_: Where) {
    if let Ok(mut registry) = registry.lock() {
        log::debug!("Publishing message to address: {}", address);
        let topic = Topic::Validated(address.clone());
        let address_seen = AddressSeen {
            address: address.clone(),
            where_,
        };
        let address_seen = serde_json::to_value(&address_seen).unwrap();
        let response = NexusResponse::notification(address_seen);
        let sent_count = registry.publish(topic, response.to_string());
        if sent_count > 0 {
            log::info!("Sent {} messages to address: {}", sent_count, address);
        }
    } else {
        log::error!("Failed to lock registry");
    }
}

/// Publish a TxSeen notification to the registry.
///
/// This function creates a TxSeen notification and publishes it to the registry
/// using the txid as the topic.
fn publish_tx_seen(registry: &Arc<Mutex<TopicRegistry>>, txid: String, where_: Where) {
    if let Ok(mut registry) = registry.lock() {
        log::debug!("Publishing message to txid: {}", txid);
        let topic = Topic::Validated(txid.clone());
        let tx_seen = TxSeen {
            txid: txid.clone(),
            where_,
        };
        let tx_seen = serde_json::to_value(&tx_seen).unwrap();
        let response = NexusResponse::notification(tx_seen);
        let sent_count = registry.publish(topic, response.to_string());
        if sent_count > 0 {
            log::info!("Sent {} messages to txid: {}", sent_count, txid);
        }
    } else {
        log::error!("Failed to lock registry");
    }
}

/// Process a single ZMQ message.
///
/// This function handles the raw message content and processes it based on the topic.
pub fn process_zmq_message(
    msg: Multipart,
    registry: &Arc<Mutex<TopicRegistry>>,
    network: &Network,
    txs_seen: &mut BoundedSet<String>,
) -> Result<(), Error> {
    log::debug!("Processing ZMQ message");
    let mut iter = msg.iter().map(|item| item.as_ref());
    let topic = iter.next().unwrap();
    let data = iter.next().unwrap();
    // TODO check there are no other parts of the message
    if topic == b"rawtx" {
        let tx = elements::Transaction::consensus_decode(data)?;
        let txid = tx.txid().to_string();
        if !txs_seen.contains(&txid) {
            log::debug!("Processing ZMQ message with txid: {}", txid);
            txs_seen.insert(txid.clone());

            // Publish TxSeen notification for mempool transactions
            publish_tx_seen(registry, txid, Where::Mempool);

            let addresses = get_output_addresses(&tx, network);
            for address_str in addresses {
                publish_address_seen(registry, address_str, Where::Mempool);
            }
        }
    } else if topic == b"rawblock" {
        let block = elements::Block::consensus_decode(data)?;
        log::info!("Processing ZMQ message with block: {}", block.block_hash());
        for tx in block.txdata.iter() {
            let addresses = get_output_addresses(tx, network);
            let txid = tx.txid().to_string();

            // Publish TxSeen notification for block transactions
            publish_tx_seen(registry, txid, Where::Block);

            // Publish AddressSeen notifications for block transactions
            for address_str in addresses {
                publish_address_seen(registry, address_str, Where::Block);
            }
        }
    }
    Ok(())
}

pub async fn start_zmq_listener(
    registry: Arc<Mutex<TopicRegistry>>,
    endpoint: &str,
    network: Network,
    tx_cache_size: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let ctx = Context::new();
    let mut txs_seen = BoundedSet::new(tx_cache_size);

    // Create the subscriber socket correctly
    let socket_builder = subscribe(&ctx);
    let socket_builder = socket_builder.connect(endpoint)?;
    let mut socket = socket_builder.subscribe(b"rawtx")?;
    socket.subscribe(b"rawblock")?;

    log::info!("Async ZMQ subscriber listening on {}", endpoint);

    // Process messages asynchronously
    while let Some(msg) = socket.next().await {
        match msg {
            Ok(multipart) => {
                if let Err(e) = process_zmq_message(multipart, &registry, &network, &mut txs_seen) {
                    log::error!("Error processing ZMQ message: {}", e);
                }
            }
            Err(e) => {
                log::error!("Error receiving ZMQ message: {}", e);
            }
        }
    }

    Ok(())
}

/// A bounded set that automatically removes the oldest entry when capacity is exceeded.
///
/// This data structure maintains insertion order and provides O(1) contains() checks
/// while keeping memory usage bounded.
pub struct BoundedSet<T> {
    set: HashSet<T>,
    queue: VecDeque<T>,
    capacity: usize,
}

impl<T> BoundedSet<T>
where
    T: Clone + std::hash::Hash + Eq,
{
    fn new(capacity: usize) -> Self {
        Self {
            set: HashSet::new(),
            queue: VecDeque::new(),
            capacity,
        }
    }

    fn contains(&self, item: &T) -> bool {
        self.set.contains(item)
    }

    fn insert(&mut self, item: T) -> bool {
        if self.set.contains(&item) {
            return false; // Item already exists
        }

        // If at capacity, remove the oldest item
        if self.queue.len() >= self.capacity {
            if let Some(oldest) = self.queue.pop_front() {
                self.set.remove(&oldest);
            }
        }

        // Add the new item
        self.set.insert(item.clone());
        self.queue.push_back(item);
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::jsonrpc::Method;

    use super::*;
    use elements::hashes::{hash160, Hash};
    use elements::{encode::Encodable, Script};
    use jsonrpc_lite::JsonRpc;
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
            registry_guard.subscribe(Topic::Validated(address.to_string()), tx_sender);
        }

        let mut tx_seen = BoundedSet::new(100);
        // Process the message
        let result = process_zmq_message(
            multipart,
            &registry,
            &Network::ElementsRegtest,
            &mut tx_seen,
        );

        // Verify the processing succeeded
        assert!(result.is_ok());

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
        let jsonrpc = JsonRpc::parse(&message).unwrap();
        let tx_seen: AddressSeen =
            serde_json::from_value(jsonrpc.get_result().unwrap().clone()).unwrap();
        assert_eq!(tx_seen.address, address);
        assert_eq!(tx_seen.where_, Where::Mempool);
        // let parsed = Message::parse(&message).unwrap();
        // assert_eq!(parsed.type_, Methods::Result);
        // assert_eq!(parsed.content(), address);
    }
}
