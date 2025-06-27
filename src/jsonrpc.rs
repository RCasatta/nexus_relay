use std::str::FromStr;

use jsonrpc_lite::Id;
use jsonrpc_lite::JsonRpc;
use lwk_wollet::LiquidexProposal;
use lwk_wollet::Unvalidated;
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
    ProposalPair(ProposalPair),
    Wallet(Wallet),
    Empty,
    Proposal(Proposal),
    Pset(Pset),
    Ping,
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

    /// It's absent for subscribe, present for publish
    pub content: Option<String>,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct PublishAny {
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

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct ProposalPair {
    /// Asset id in input (the maker is selling this)
    pub input: String,

    /// Asset id in output (the maker is buying this)
    pub output: String,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct Wallet {
    pub id: String,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct Pset {
    pub wallet_id: String,
    pub pset: String,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub proposal: LiquidexProposal<Unvalidated>,
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
                        "pair" => {
                            let proposal: ProposalPair = serde_json::from_value(value).unwrap();
                            Ok(Params::ProposalPair(proposal))
                        }
                        "proposal" => {
                            let proposal: LiquidexProposal<Unvalidated> =
                                serde_json::from_value(value).unwrap();
                            Ok(Params::Proposal(Proposal { proposal: proposal }))
                        }
                        "wallet" => {
                            let pset: Wallet = serde_json::from_value(value).unwrap();
                            Ok(Params::Wallet(pset))
                        }
                        "pset" => {
                            let pset: Pset = serde_json::from_value(value).unwrap();
                            Ok(Params::Pset(pset))
                        }
                        "ping" => Ok(Params::Ping),
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
        println!("validate_for_method: {:?}, {:?}", self, method);
        match (self, method) {
            (Params::Any(any), e) => match e {
                Methods::Subscribe => {
                    if any.content.is_some() {
                        Err(Error::InvalidParams)
                    } else {
                        Ok(())
                    }
                }
                Methods::Publish => {
                    if any.content.is_none() {
                        Err(Error::InvalidParams)
                    } else {
                        Ok(())
                    }
                }
                _ => Err(Error::InvalidParams),
            },
            (Params::Address(_), Methods::Subscribe) => Ok(()),
            (Params::Tx(_), Methods::Subscribe) => Ok(()),
            (Params::Wallet(_), Methods::Subscribe) => Ok(()),
            (Params::ProposalPair(_), Methods::Subscribe) => Ok(()),
            (Params::Proposal(_), Methods::Publish) => Ok(()),
            (Params::Pset(_), Methods::Publish) => Ok(()),
            (Params::Ping, Methods::Publish) => Ok(()),
            _ => Err(Error::InvalidParamsForThisMethod),
        }
    }
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

    fn proposal_str() -> &'static str {
        include_str!("../test_data/proposal.json")
    }

    #[test]
    fn test_nexus_request_ping() {
        let jsonrpc = json!({
            "id": 10,
            "jsonrpc": "2.0",
            "method": "publish",
            "params": {
                "ping": {}
            }
        });
        let jsonrpc: JsonRpc = serde_json::from_value(jsonrpc).unwrap();
        let request = NexusRequest::try_from(jsonrpc).unwrap();
        assert_eq!(request.method, Methods::Publish);
        assert_eq!(request.params, Params::Ping);
    }

    #[test]
    fn test_nexus_request_subscribe_any() {
        let jsonrpc = json!({
            "id": 10,
            "jsonrpc": "2.0",
            "method": "subscribe",
            "params": {
                "any": {
                    "topic": "test"
                }
            }
        });
        let jsonrpc: JsonRpc = serde_json::from_value(jsonrpc).unwrap();
        let request = NexusRequest::try_from(jsonrpc).unwrap();
        assert_eq!(request.method, Methods::Subscribe);
        assert_eq!(
            request.params,
            Params::Any(Any {
                topic: "test".to_string(),
                content: None
            })
        );
    }

    #[test]
    fn test_nexus_request_subscribe_proposal() {
        let jsonrpc = json!({
            "id": 10,
            "jsonrpc": "2.0",
            "method": "subscribe",
            "params": {
                "pair": {
                    "input": "test",
                    "output": "best"
                }
            }
        });
        let jsonrpc: JsonRpc = serde_json::from_value(jsonrpc).unwrap();
        let request = NexusRequest::try_from(jsonrpc).unwrap();
        assert_eq!(request.method, Methods::Subscribe);
        assert_eq!(
            request.params,
            Params::ProposalPair(ProposalPair {
                input: "test".to_string(),
                output: "best".to_string()
            })
        );
    }

    #[test]
    fn test_nexus_request_subscribe_address() {
        let jsonrpc = json!({
            "id": 10,
            "jsonrpc": "2.0",
            "method": "subscribe",
            "params": {
                "address": {
                    "address": "test"
                }
            }
        });
        let jsonrpc: JsonRpc = serde_json::from_value(jsonrpc).unwrap();
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
        let jsonrpc = json!({
            "id": 10,
            "jsonrpc": "2.0",
            "method": "subscribe",
            "params": {
                "tx": {
                    "txid": "test"
                }
            }
        });
        let jsonrpc: JsonRpc = serde_json::from_value(jsonrpc).unwrap();
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
    fn test_nexus_request_subscribe_wallet() {
        let jsonrpc = json!({
            "id": 10,
            "jsonrpc": "2.0",
            "method": "subscribe",
            "params": {
                "wallet": {
                    "id": "walletid"
                }
            }
        });
        let jsonrpc: JsonRpc = serde_json::from_value(jsonrpc).unwrap();
        let request = NexusRequest::try_from(jsonrpc).unwrap();
        assert_eq!(request.method, Methods::Subscribe);
        assert_eq!(
            request.params,
            Params::Wallet(Wallet {
                id: "walletid".to_string()
            })
        );
    }

    #[test]
    fn test_nexus_request_publish_any() {
        let jsonrpc = json!({
            "id": 10,
            "jsonrpc": "2.0",
            "method": "publish",
            "params": {
                "any": {
                    "topic": "test",
                    "content": "content"
                }
            }
        });
        let jsonrpc: JsonRpc = serde_json::from_value(jsonrpc).unwrap();
        let request = NexusRequest::try_from(jsonrpc).unwrap();
        assert_eq!(request.method, Methods::Publish);
        assert_eq!(
            request.params,
            Params::Any(Any {
                topic: "test".to_string(),
                content: Some("content".to_string())
            })
        );
    }

    #[test]
    fn test_nexus_request_publish_proposal() {
        let proposal = LiquidexProposal::<Unvalidated>::from_str(proposal_str()).unwrap();
        let jsonrpc = json!({
            "id": 10,
            "jsonrpc": "2.0",
            "method": "publish",
            "params": {
                "proposal": proposal,
            }
        });

        let jsonrpc: JsonRpc = serde_json::from_value(jsonrpc).unwrap();
        let request = NexusRequest::try_from(jsonrpc).unwrap();
        assert_eq!(request.method, Methods::Publish);
        assert_eq!(
            request.params,
            Params::Proposal(Proposal { proposal: proposal })
        );
    }

    #[test]
    fn test_nexus_request_publish_pset() {
        let jsonrpc = json!({
            "id": 10,
            "jsonrpc": "2.0",
            "method": "publish",
            "params": {
                "pset": {
                    "wallet_id": "walletid",
                    "pset": "base64"
                }
            }
        });
        let jsonrpc: JsonRpc = serde_json::from_value(jsonrpc).unwrap();
        let request = NexusRequest::try_from(jsonrpc).unwrap();
        assert_eq!(request.method, Methods::Publish);
        assert_eq!(
            request.params,
            Params::Pset(Pset {
                wallet_id: "walletid".to_string(),
                pset: "base64".to_string()
            })
        );
    }
}
