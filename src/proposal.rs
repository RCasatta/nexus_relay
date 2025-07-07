use std::sync::{Arc, Mutex};

use crate::error::Error;
use crate::jsonrpc::{topic_from_proposal, NexusResponse, Proposal};
use crate::node::Node;
use crate::TopicRegistry;

/// Process a publish proposal message
pub async fn process_publish_proposal(
    proposal: Proposal,
    registry: Arc<Mutex<TopicRegistry>>,
    client: Option<&Node>,
    id: u32,
) -> Result<NexusResponse, Error> {
    let proposal = proposal.proposal;
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
    let topic = topic_from_proposal(&proposal)?;
    let message_to_subscriber = NexusResponse::new_proposal(proposal).to_string();

    // Lock the mutex only when needed and release it immediately
    let sent_count = {
        let mut registry_guard = registry.lock().unwrap();
        registry_guard.publish(topic.clone(), message_to_subscriber)
    };

    log::info!(
        "PublishProposal sent to {} subscribers on topic: {:?}",
        sent_count,
        topic
    );
    let message_response = NexusResponse::new_published(id);
    Ok(message_response)
}

/// Validate a topic to ensure it's a valid proposal topic
// pub fn validate_proposal_topic(topic: &str) -> Result<(), Error> {
//     if topic.len() >= 32 {
//         // if the topic is longer or equal 32 chars, it must be a proposal topic
//         let mut assets = topic.splitn(2, ':');
//         if let (Some(asset1), Some(asset2)) = (assets.next(), assets.next()) {
//             let asset1 = AssetId::from_str(asset1);
//             let asset2 = AssetId::from_str(asset2);
//             if let (Ok(_), Ok(_)) = (asset1, asset2) {
//                 // topic validated
//                 return Ok(());
//             }
//         }
//         return Err(Error::InvalidParams);
//     }
//     Ok(())
// }

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use lwk_wollet::LiquidexProposal;
    use tokio::runtime::Runtime;

    fn proposal_str() -> &'static str {
        include_str!("../test_data/proposal.json")
    }

    fn proposal() -> Proposal {
        Proposal {
            proposal: LiquidexProposal::from_str(proposal_str()).unwrap(),
        }
    }

    #[test]
    fn test_publish_proposal() {
        // Create a runtime
        let rt = Runtime::new().unwrap();

        let id = 12341234;
        let proposal = proposal();
        let registry = Arc::new(Mutex::new(TopicRegistry::new()));

        // Publish the proposal
        let message_response = rt
            .block_on(process_publish_proposal(proposal, registry, None, id))
            .unwrap();
        assert_eq!(
            message_response.to_string(),
            "{\"jsonrpc\":\"2.0\",\"result\":\"published\",\"id\":12341234}"
        );
    }
}
