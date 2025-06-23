use std::str::FromStr;

use jsonrpc_lite::Id;
use jsonrpc_lite::JsonRpc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::message::Methods;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
struct TopicContent {
    topic: String,
    content: String,
}

pub fn parse_method(jsonrpc: &JsonRpc) -> Option<Methods> {
    match jsonrpc.get_method() {
        Some(s) => Methods::from_str(s).ok(),
        _ => None,
    }
}

// fn subscribe(id: i64, topic: String, content: String) -> JsonRpc {
//     let params = TopicContent { topic, content };
//     let params: Value = serde_json::to_value(params).unwrap();
//     JsonRpc::request_with_params(id, &Methods::Subscribe.to_string(), params)
// }

// fn parse_subscribe_any(notification: JsonRpc) -> Option<(i64, TopicContent)> {
//     if notification.get_method() != Some(&Methods::Subscribe.to_string()) {
//         return None;
//     }
//     let id = parse_id(notification.get_id()?)?;
//     let params = notification.get_params()?;
//     let params_str = serde_json::to_string(&params).ok()?;
//     let topic_content: TopicContent = serde_json::from_str(&params_str).ok()?;

//     Some((id, topic_content))
// }

fn ping(id: i64) -> JsonRpc {
    JsonRpc::request(id, &Methods::Ping.to_string())
}

fn parse_ping(notification: JsonRpc) -> Option<i64> {
    if notification.get_method() != Some(&Methods::Ping.to_string()) {
        return None;
    }
    let id = parse_id(notification.get_id()?)?;
    Some(id)
}

pub fn parse_id(id: Id) -> Option<i64> {
    match id {
        Id::Num(n) => Some(n),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_jsonrpc_notification() {
        // let request = subscribe(10, "test".to_string(), "content".to_string());
        // let expected = json!({
        //     "id": 10,
        //     "jsonrpc": "2.0",
        //     "method": "subscribe",
        //     "params": {
        //         "any": {
        //             "topic": "test",
        //             "content": "content"
        //         }
        //     }
        // });
        // let result = serde_json::to_value(&request).unwrap();
        // assert_eq!(result, expected);
        // let (id, topic_content) = parse_subscribe_any(request).unwrap();
        // assert_eq!(id, 10);
        // assert_eq!(topic_content.topic, "test");
        // assert_eq!(topic_content.content, "content");

        let ping = ping(10);
        let expected = json!({
            "id": 10,
            "jsonrpc": "2.0",
            "method": "ping"
        });
        let result = serde_json::to_value(&ping).unwrap();
        assert_eq!(result, expected);
        let id = parse_ping(ping).unwrap();
        assert_eq!(id, 10);
    }
}
