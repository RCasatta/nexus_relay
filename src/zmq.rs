use crate::message::{Message, MessageType};
use crate::TopicRegistry;
use futures_util::StreamExt;
use std::sync::{Arc, Mutex};
use tmq::{subscribe, Context};

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
        println!(
            "Subscribe: {:?}",
            msg?.iter()
                .map(|item| item.as_str().unwrap_or("invalid text"))
                .collect::<Vec<&str>>()
        );
    }

    Ok(())
}
