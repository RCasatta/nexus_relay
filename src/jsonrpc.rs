use std::str::FromStr;

use jsonrpc_lite::Id;
use jsonrpc_lite::JsonRpc;
use serde::{Deserialize, Serialize};
use std::fmt;

use crate::message::Methods;

pub struct NexusRequest {
    pub method: Methods,
    pub params: Params,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub enum Params {
    Any(Any),
    Address(Address),
    Tx(Tx),
    Empty,
}

#[derive(Debug)]
pub enum Error {
    InvalidMethod,
    InvalidParams,
    InvalidParamsForThisMethod,
    ArrayParamsAreInvalid,
    ParamsMustContainASingleRootElement,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for Error {}

impl TryFrom<JsonRpc> for NexusRequest {
    type Error = Error;

    fn try_from(jsonrpc: JsonRpc) -> Result<Self, Self::Error> {
        let method = parse_method(&jsonrpc).ok_or(Error::InvalidMethod)?;
        let params = parse_params(&jsonrpc)?;
        params.validate_for_method(&method)?;

        Ok(NexusRequest { method, params })
    }
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct TopicContent {
    pub topic: String,
    pubcontent: String,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct Any {
    pub topic: String,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct Address {
    pub address: String,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct Tx {
    pub txid: String,
}

pub fn parse_params(jsonrpc: &JsonRpc) -> Result<Params, Error> {
    match jsonrpc.get_params() {
        None => Ok(Params::Empty),
        Some(params) => match params {
            jsonrpc_lite::Params::Array(_) => Err(Error::ArrayParamsAreInvalid),
            jsonrpc_lite::Params::Map(map) => {
                if map.len() != 1 {
                    return Err(Error::ParamsMustContainASingleRootElement);
                }
                match map.into_iter().next() {
                    Some((key, value)) => match key.as_str() {
                        "any" => {
                            let any: Any = serde_json::from_value(value).unwrap();
                            Ok(Params::Any(any))
                        }
                        "address" => {
                            let address: Address = serde_json::from_value(value).unwrap();
                            Ok(Params::Address(address))
                        }
                        "tx" => {
                            let tx: Tx = serde_json::from_value(value).unwrap();
                            Ok(Params::Tx(tx))
                        }
                        _ => Err(Error::InvalidParams),
                    },
                    None => Ok(Params::Empty),
                }
            }
            jsonrpc_lite::Params::None(_) => Ok(Params::Empty),
        },
    }
}

pub fn parse_method(jsonrpc: &JsonRpc) -> Option<Methods> {
    match jsonrpc.get_method() {
        Some(s) => Methods::from_str(s).ok(),
        _ => None,
    }
}

impl Params {
    pub fn validate_for_method(&self, method: &Methods) -> Result<(), Error> {
        match (self, method) {
            (Params::Any(_), Methods::Subscribe) => Ok(()),
            (Params::Address(_), Methods::Subscribe) => Ok(()),
            (Params::Tx(_), Methods::Subscribe) => Ok(()),
            (Params::Empty, Methods::Ping) => Ok(()),
            _ => Err(Error::InvalidParamsForThisMethod),
        }
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
    fn test_nexus_request_ping() {
        let ping = json!({
            "id": 10,
            "jsonrpc": "2.0",
            "method": "ping"
        });
        let jsonrpc: JsonRpc = serde_json::from_value(ping).unwrap();
        let request = NexusRequest::try_from(jsonrpc).unwrap();
        assert_eq!(request.method, Methods::Ping);
        assert_eq!(request.params, Params::Empty);
    }

    #[test]
    fn test_nexus_request_subscribe_any() {
        let ping = json!({
            "id": 10,
            "jsonrpc": "2.0",
            "method": "subscribe",
            "params": {
                "any": {
                    "topic": "test"
                }
            }
        });
        let jsonrpc: JsonRpc = serde_json::from_value(ping).unwrap();
        let request = NexusRequest::try_from(jsonrpc).unwrap();
        assert_eq!(request.method, Methods::Subscribe);
        assert_eq!(
            request.params,
            Params::Any(Any {
                topic: "test".to_string()
            })
        );
    }

    #[test]
    fn test_nexus_request_subscribe_address() {
        let ping = json!({
            "id": 10,
            "jsonrpc": "2.0",
            "method": "subscribe",
            "params": {
                "address": {
                    "address": "test"
                }
            }
        });
        let jsonrpc: JsonRpc = serde_json::from_value(ping).unwrap();
        let request = NexusRequest::try_from(jsonrpc).unwrap();
        assert_eq!(request.method, Methods::Subscribe);
        assert_eq!(
            request.params,
            Params::Address(Address {
                address: "test".to_string()
            })
        );
    }

    #[test]
    fn test_nexus_request_subscribe_tx() {
        let ping = json!({
            "id": 10,
            "jsonrpc": "2.0",
            "method": "subscribe",
            "params": {
                "tx": {
                    "txid": "test"
                }
            }
        });
        let jsonrpc: JsonRpc = serde_json::from_value(ping).unwrap();
        let request = NexusRequest::try_from(jsonrpc).unwrap();
        assert_eq!(request.method, Methods::Subscribe);
        assert_eq!(
            request.params,
            Params::Tx(Tx {
                txid: "test".to_string()
            })
        );
    }

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
