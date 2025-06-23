use lwk_wollet::elements::AssetId;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use crate::message::{proposal_topic, Error, Message, Methods};
use crate::node::Node;
use crate::{Topic, TopicRegistry};

/// Process a publish proposal message
pub async fn process_publish_proposal<'a>(
    message_request: &'a Message<'a>,
    registry: Arc<Mutex<TopicRegistry>>,
    client: Option<&Node>,
    random_id: Option<u64>,
) -> Result<Message<'a>, Box<dyn std::error::Error>> {
    let proposal = message_request.proposal()?;
    let proposal = if let Some(client) = client {
        let txid = proposal.needed_tx()?;
        log::info!("PublishProposal asking for txid: {}", txid);
        let tx = client.tx(txid).await.unwrap();
        let validated = proposal.validate(tx)?;
        log::info!("PublishProposal validated");
        // TODO verify it's unspent
        validated
    } else {
        log::info!("PublishProposal asking for insecure validation");
        proposal.insecure_validate()?
    };
    let topic = proposal_topic(&proposal)?;
    let proposal_str = format!("{}", proposal);
    let message_to_subscriber = Message::new(Methods::Result, None, &proposal_str);

    // Lock the mutex only when needed and release it immediately
    let sent_count = {
        let mut registry_guard = registry.lock().unwrap();
        let topic = Topic::Validated(topic.clone());
        registry_guard.publish(topic, message_to_subscriber)
    };

    log::info!(
        "PublishProposal sent to {} subscribers on topic: {}",
        sent_count,
        topic
    );
    let message_response = Message::ack(random_id);
    Ok(message_response)
}

/// Validate a topic to ensure it's a valid proposal topic
pub fn validate_proposal_topic(topic: &str) -> Result<(), Error> {
    if topic.len() >= 32 {
        // if the topic is longer or equal 32 chars, it must be a proposal topic
        let mut assets = topic.splitn(2, '|');
        if let (Some(asset1), Some(asset2)) = (assets.next(), assets.next()) {
            let asset1 = AssetId::from_str(asset1);
            let asset2 = AssetId::from_str(asset2);
            if let (Ok(_), Ok(_)) = (asset1, asset2) {
                // topic validated
                return Ok(());
            }
        }
        return Err(Error::InvalidTopic);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::runtime::Runtime;

    fn proposal_str() -> &'static str {
        include_str!("../test_data/proposal.json")
    }

    #[test]
    fn test_publish_proposal() {
        // Create a runtime
        let rt = Runtime::new().unwrap();

        let id = 12341234;
        let proposal_json = proposal_str();
        let message_publish = format!(
            "PUBLISH_PROPOSAL||{id}|{}|{}",
            proposal_json.len(),
            proposal_json
        );
        let message_publish = Message::parse(&message_publish).unwrap();
        let registry = Arc::new(Mutex::new(TopicRegistry::new()));

        // Publish the proposal
        let message_response = rt
            .block_on(process_publish_proposal(
                &message_publish,
                registry,
                None,
                message_publish.random_id,
            ))
            .unwrap();
        assert_eq!(message_response.to_string(), format!("ACK||{id}||"));
    }
}
