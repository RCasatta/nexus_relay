use crate::message::{Message, MessageType};
use crate::TopicRegistry;
use elements::encode::Decodable;
use futures_util::StreamExt;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tmq::{subscribe, Context, Multipart};

/// Process a single ZMQ message.
///
/// This function handles the raw message content and processes it based on the topic.
pub fn process_zmq_message(
    msg: Multipart,
    _registry: &Arc<Mutex<TopicRegistry>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut iter = msg.iter().map(|item| item.as_ref());
    let topic = iter.next().unwrap();
    let tx = iter.next().unwrap();
    if topic == b"rawtx" {
        let tx = elements::Transaction::consensus_decode(tx)?;
        println!("Received rawtx: {:?}", tx.txid());
        for out in tx.output {
            println!("Output : {:?}", out);
            if let Some(addr) = elements::Address::from_script(
                &out.script_pubkey,
                None,
                &elements::AddressParams::ELEMENTS,
            ) {
                println!("Output address: {}", addr);
            }
        }
    }

    Ok(())
}

pub async fn start_zmq_listener(
    registry: Arc<Mutex<TopicRegistry>>,
    endpoint: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let ctx = Context::new();

    // Create the subscriber socket correctly
    let socket_builder = subscribe(&ctx);
    let socket_builder = socket_builder.connect(endpoint)?;
    let mut socket = socket_builder.subscribe(b"rawtx").unwrap();

    println!("Async ZMQ subscriber listening on {}", endpoint);

    // Process messages asynchronously
    while let Some(msg) = socket.next().await {
        match msg {
            Ok(multipart) => {
                if let Err(e) = process_zmq_message(multipart, &registry) {
                    eprintln!("Error processing ZMQ message: {}", e);
                }
            }
            Err(e) => {
                eprintln!("Error receiving ZMQ message: {}", e);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use elements::{encode::Encodable, Script};
    use std::str::FromStr;

    #[test]
    fn test_process_zmq_message() {
        let script_pubkey = Script::from_str("522102ebc62c20f1e09e169a88745f60f6dac878c92db5c7ed78c6703d2d0426a01f942102c2d59d677122bc292048833003fd5cb19d27d32896b1d0feec654c291f7ede9e52ae").unwrap();
        let tx_hex = "0200000001010000000000000000000000000000000000000000000000000000000000000000ffffffff0401650101ffffffff020125b251070e29ca19043cf33ccd7324e2ddab03ecc4ae0b5e77c4fc0e5cf6c95a01000000000000000000016a0125b251070e29ca19043cf33ccd7324e2ddab03ecc4ae0b5e77c4fc0e5cf6c95a01000000000000000000266a24aa21a9ed94f15ed3a62165e4a0b99699cc28b48e19cb5bc1b1f47155db62d63f1e047d45000000000000012000000000000000000000000000000000000000000000000000000000000000000000000000";
        // Create a simple Elements transaction
        let tx =
            elements::Transaction::consensus_decode(&hex::decode(tx_hex).unwrap()[..]).unwrap();

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

        // Create a mock registry
        let registry = Arc::new(Mutex::new(TopicRegistry::new()));

        // Process the message
        let result = process_zmq_message(multipart, &registry);

        // Verify the processing succeeded
        assert!(result.is_ok());
    }
}
