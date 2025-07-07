use std::str::FromStr;

use jsonrpc_lite::Id;
use jsonrpc_lite::JsonRpc;
use lwk_wollet::LiquidexProposal;
use lwk_wollet::Unvalidated;
use serde::{Deserialize, Serialize};
use std::fmt;

use crate::Topic;

#[derive(Debug)]
pub struct NexusRequest {
    pub id: u32,
    pub method: Method,
    pub params: Params,
}
#[derive(Debug)]
pub struct NexusResponse {
    id: Option<i64>,
    val: Result<serde_json::Value, Error>,
}

impl NexusRequest {
    pub fn new_address_subscribe(id: u32, address: &str) -> Self {
        Self {
            id,
            method: Method::Subscribe,
            params: Params::Address(address.to_string()),
        }
    }
    pub fn new_txid_subscribe(id: u32, txid: &str) -> Self {
        Self {
            id,
            method: Method::Subscribe,
            params: Params::Tx(Tx {
                txid: txid.to_string(),
            }),
        }
    }
    pub(crate) fn topic(&self) -> Result<Topic, Error> {
        topic_from_params(&self.params)
    }
}

impl NexusResponse {
    pub fn new(id: u32, val: serde_json::Value) -> Self {
        Self {
            id: Some(id as i64),
            val: Ok(val),
        }
    }
    pub fn new_subscribed(id: u32) -> Self {
        Self {
            id: Some(id as i64),
            val: Ok(serde_json::Value::String("subscribed".to_string())),
        }
    }
    pub fn new_pong(id: u32) -> Self {
        Self {
            id: Some(id as i64),
            val: Ok(serde_json::Value::String("pong".to_string())),
        }
    }
    pub fn error(id: u32, err: Error) -> Self {
        Self {
            id: Some(id as i64),
            val: Err(err),
        }
    }
    pub fn notification(val: serde_json::Value) -> Self {
        Self {
            id: Some(-1),
            val: Ok(val),
        }
    }
}

impl From<NexusResponse> for JsonRpc {
    fn from(response: NexusResponse) -> Self {
        JsonRpc::from(&response)
    }
}

impl From<&NexusResponse> for JsonRpc {
    fn from(response: &NexusResponse) -> Self {
        match (response.id, &response.val) {
            (Some(id), Ok(val)) => JsonRpc::success(id as i64, val),
            (Some(id), Err(err)) => JsonRpc::error(id as i64, err.into()),
            (None, Ok(val)) => {
                // Note JSON-RPC 2.0 does not properly support pub-sub notifications, because:
                // * notifications are supposed to not carry a result
                // * response are supposed to have an id
                // In absence of a proper way we use a success response with a negative id
                JsonRpc::success(-1, &val)
            }
            (None, Err(_)) => unreachable!(),
        }
    }
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Params {
    Any(Any),
    Address(String),
    Tx(Tx),
    Pair(Pair),
    Wallet(Wallet),
    Empty,
    Proposal(Proposal),
    Pset(Pset),
    Ping,
}

#[derive(Debug)]
pub enum Error {
    InvalidId(Option<Id>),
    InvalidMethod,
    InvalidParams,
    InvalidParamsForThisMethod,
    ArrayParamsAreInvalid,
    ParamsMustContainASingleRootElement,
}

impl From<Error> for jsonrpc_lite::JsonRpc {
    fn from(err: Error) -> Self {
        jsonrpc_lite::JsonRpc::error(0, err.into())
    }
}

impl From<&Error> for jsonrpc_lite::JsonRpc {
    fn from(err: &Error) -> Self {
        jsonrpc_lite::JsonRpc::error(0, err.into())
    }
}

impl From<&Error> for jsonrpc_lite::Error {
    fn from(err: &Error) -> Self {
        match err {
            Error::InvalidId(id) => jsonrpc_lite::Error {
                code: jsonrpc_lite::ErrorCode::InvalidRequest.code(),
                message: format!("Invalid id, must be present, not a string and positive"),
                data: Some(
                    id.as_ref()
                        .map(|id| serde_json::to_value(&id).unwrap())
                        .unwrap_or(serde_json::Value::Null),
                ),
            },
            Error::InvalidMethod => jsonrpc_lite::Error::method_not_found(),
            Error::InvalidParams
            | Error::InvalidParamsForThisMethod
            | Error::ArrayParamsAreInvalid
            | Error::ParamsMustContainASingleRootElement => jsonrpc_lite::Error::invalid_params(),
        }
    }
}

impl From<Error> for jsonrpc_lite::Error {
    fn from(err: Error) -> Self {
        jsonrpc_lite::Error::from(&err)
    }
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
        let id = parse_id(&jsonrpc)?;
        let method = parse_method(&jsonrpc)?;
        let params = parse_params(&jsonrpc)?;
        params.validate_for_method(&method)?;

        Ok(NexusRequest { id, method, params })
    }
}

impl fmt::Display for NexusRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let method = self.method.to_string();
        let params = serde_json::to_value(&self.params).unwrap();
        let j = JsonRpc::request_with_params(self.id as i64, &method, params);
        write!(f, "{}", serde_json::to_string(&j).unwrap())
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
pub struct Pair {
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

pub fn topic_from_params(params: &Params) -> Result<Topic, Error> {
    match params {
        Params::Any(any) => Ok(Topic::Unvalidated(any.topic.clone())),
        Params::Address(address) => Ok(Topic::Validated(address.clone())),
        Params::Tx(tx) => Ok(Topic::Validated(tx.txid.clone())),
        Params::Pair(proposal) => Ok(Topic::Validated(proposal.input.clone())),
        Params::Pset(pset) => Ok(Topic::Validated(pset.wallet_id.clone())),
        _ => Err(Error::InvalidParams),
    }
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
                            let address: String = serde_json::from_value(value).unwrap();
                            Ok(Params::Address(address))
                        }
                        "tx" => {
                            let tx: Tx = serde_json::from_value(value).unwrap();
                            Ok(Params::Tx(tx))
                        }
                        "pair" => {
                            let proposal: Pair = serde_json::from_value(value).unwrap();
                            Ok(Params::Pair(proposal))
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

pub fn parse_method(jsonrpc: &JsonRpc) -> Result<Method, Error> {
    match jsonrpc.get_method() {
        Some(s) => Method::from_str(s).map_err(|_| Error::InvalidMethod),
        _ => Err(Error::InvalidMethod),
    }
}

impl Params {
    pub fn validate_for_method(&self, method: &Method) -> Result<(), Error> {
        println!("validate_for_method: {:?}, {:?}", self, method);
        match (self, method) {
            (Params::Any(any), e) => match e {
                Method::Subscribe => {
                    if any.content.is_some() {
                        Err(Error::InvalidParams)
                    } else {
                        Ok(())
                    }
                }
                Method::Publish => {
                    if any.content.is_none() {
                        Err(Error::InvalidParams)
                    } else {
                        Ok(())
                    }
                }
                _ => Err(Error::InvalidParams),
            },
            (Params::Address(_), Method::Subscribe) => Ok(()),
            (Params::Tx(_), Method::Subscribe) => Ok(()),
            (Params::Wallet(_), Method::Subscribe) => Ok(()),
            (Params::Pair(_), Method::Subscribe) => Ok(()),
            (Params::Proposal(_), Method::Publish) => Ok(()),
            (Params::Pset(_), Method::Publish) => Ok(()),
            (Params::Ping, Method::Publish) => Ok(()),
            _ => Err(Error::InvalidParamsForThisMethod),
        }
    }
}

pub fn parse_id(jsonrpc: &JsonRpc) -> Result<u32, Error> {
    let id = jsonrpc.get_id();
    if let Some(Id::Num(n)) = id {
        if n > 0 && n < (u32::MAX as i64) {
            return Ok(n as u32);
        }
    }
    Err(Error::InvalidId(id))
}

#[derive(Debug, PartialEq, Clone)]
pub enum Method {
    Publish,
    Subscribe,
    Unsubscribe,
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Method::Publish => write!(f, "publish"),
            Method::Subscribe => write!(f, "subscribe"),
            Method::Unsubscribe => write!(f, "unsubscribe"),
        }
    }
}

impl FromStr for Method {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "publish" => Ok(Method::Publish),
            "subscribe" => Ok(Method::Subscribe),
            "unsubscribe" => Ok(Method::Unsubscribe),
            _ => Err(Error::InvalidMethod),
        }
    }
}

impl fmt::Display for NexusResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let jsonrpc = JsonRpc::from(self);
        write!(f, "{}", serde_json::to_string(&jsonrpc).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

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
        assert_eq!(request.method, Method::Publish);
        assert_eq!(request.params, Params::Ping);
    }

    #[test]
    fn test_nexus_response_ping() {
        let jsonrpc = json!({
            "id": 10,
            "jsonrpc": "2.0",
            "result": "pong"
        });
        let jsonrpc: JsonRpc = serde_json::from_value(jsonrpc).unwrap();
        let request = NexusResponse::new(10, Value::String("pong".to_string()));
        assert_eq!(JsonRpc::from(request), jsonrpc);
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
        assert_eq!(request.method, Method::Subscribe);
        assert_eq!(
            request.params,
            Params::Any(Any {
                topic: "test".to_string(),
                content: None
            })
        );
    }

    fn test_nexus_response_subscribe() {
        let jsonrpc = json!({
            "id": 10,
            "jsonrpc": "2.0",
            "result": "subscribed"
        });
        let jsonrpc: JsonRpc = serde_json::from_value(jsonrpc).unwrap();
        let request = NexusResponse::new(10, Value::String("subscribed".to_string()));
        assert_eq!(JsonRpc::from(request), jsonrpc);

        let address = json!({
            "address": "test"
        });
        let jsonrpc = json!({
            "id": -1,
            "jsonrpc": "2.0",
            "result": address
        });
        let jsonrpc: JsonRpc = serde_json::from_value(jsonrpc).unwrap();
        let request = NexusResponse::notification(address);
        assert_eq!(JsonRpc::from(request), jsonrpc);
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
        assert_eq!(request.method, Method::Subscribe);
        assert_eq!(
            request.params,
            Params::Pair(Pair {
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
                "address": "test"
            }
        });
        let jsonrpc: JsonRpc = serde_json::from_value(jsonrpc).unwrap();
        let request = NexusRequest::try_from(jsonrpc).unwrap();
        assert_eq!(request.method, Method::Subscribe);
        assert_eq!(request.params, Params::Address("test".to_string()));
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
        assert_eq!(request.method, Method::Subscribe);
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
        assert_eq!(request.method, Method::Subscribe);
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
        assert_eq!(request.method, Method::Publish);
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
        assert_eq!(request.method, Method::Publish);
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
        assert_eq!(request.method, Method::Publish);
        assert_eq!(
            request.params,
            Params::Pset(Pset {
                wallet_id: "walletid".to_string(),
                pset: "base64".to_string()
            })
        );
    }

    #[test]
    fn test_message_type_roundtrip() {
        // Test all variants of MessageType for roundtrip conversion
        let types = vec![Method::Publish, Method::Subscribe, Method::Unsubscribe];

        for msg_type in types {
            // Convert MessageType to string
            let type_str = msg_type.to_string();
            // Parse string back to MessageType
            let parsed_type = Method::from_str(&type_str).unwrap();
            // Verify roundtrip conversion
            assert_eq!(msg_type, parsed_type);
        }
    }
}
