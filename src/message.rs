use std::error::Error as StdError;
use std::fmt;
use std::str::FromStr;

use lwk_wollet::{LiquidexProposal, Unvalidated, Validated};

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Message<'a> {
    pub(crate) type_: MessageType,
    pub(crate) random_id: Option<u64>,
    pub(crate) content: &'a str,
}

#[derive(Debug, PartialEq, Clone)]
pub enum MessageType {
    Publish,
    PublishProposal,
    Subscribe,
    Result,
    Error,
    Ping,
    Pong,
}

impl fmt::Display for MessageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MessageType::Publish => write!(f, "PUBLISH"),
            MessageType::PublishProposal => write!(f, "PUBLISH_PROPOSAL"),
            MessageType::Subscribe => write!(f, "SUBSCRIBE"),
            MessageType::Result => write!(f, "RESULT"),
            MessageType::Error => write!(f, "ERROR"),
            MessageType::Ping => write!(f, "PING"),
            MessageType::Pong => write!(f, "PONG"),
        }
    }
}

impl FromStr for MessageType {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "PUBLISH" => Ok(MessageType::Publish),
            "PUBLISH_PROPOSAL" => Ok(MessageType::PublishProposal),
            "SUBSCRIBE" => Ok(MessageType::Subscribe),
            "RESULT" => Ok(MessageType::Result),
            "ERROR" => Ok(MessageType::Error),
            "PING" => Ok(MessageType::Ping),
            "PONG" => Ok(MessageType::Pong),
            _ => Err(Error::InvalidMessage),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Error {
    InvalidMessage,
    MissingField,
    InvalidVersion,
    InvalidRandomId,
    InvalidLength,
    ContentLengthMismatch { expected: u64, actual: u64 },
    MissingTopic,
    InvalidTopic,
    MissingContent,
    InvalidProposal,
    ResponseMessageUsedAsRequest,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidMessage => write!(f, "Invalid message type"),
            Error::MissingField => write!(f, "Missing message field"),
            Error::InvalidVersion => write!(f, "Invalid message version"),
            Error::InvalidRandomId => write!(f, "Invalid random ID"),
            Error::InvalidLength => write!(f, "Invalid length"),
            Error::ContentLengthMismatch { expected, actual } => write!(
                f,
                "Content length mismatch: expected {}, got {}",
                expected, actual
            ),
            Error::MissingTopic => write!(f, "Missing topic"),
            Error::MissingContent => write!(f, "Missing content"),
            Error::InvalidProposal => write!(f, "Invalid proposal"),
            Error::InvalidTopic => write!(f, "Invalid topic"),
            Error::ResponseMessageUsedAsRequest => write!(f, "Response message used as request"),
        }
    }
}

impl<'a> fmt::Display for Message<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}||", self.type_)?; // type and version
        if let Some(random_id) = self.random_id {
            write!(f, "{}", random_id)?;
        }
        write!(f, "|")?;
        let length = self.content.len() as u64;
        if length > 0 {
            write!(f, "{}", length)?;
        }
        write!(f, "|")?;
        write!(f, "{}", self.content)
    }
}

impl StdError for Error {}

impl From<std::num::ParseIntError> for Error {
    fn from(_err: std::num::ParseIntError) -> Self {
        // We can't determine which field caused the error here,
        // so we'll rely on the parsing logic to use specific errors
        Error::MissingField
    }
}

// Custom parser instead of implementing FromStr directly to avoid lifetime issues
impl<'a> Message<'a> {
    pub fn new(type_: MessageType, random_id: Option<u64>, content: &'a str) -> Self {
        Message {
            type_,
            random_id,
            content,
        }
    }

    pub fn parse(s: &'a str) -> Result<Self, Error> {
        let mut parts = s.splitn(5, '|');

        let type_str = parts.next().ok_or(Error::MissingField)?;
        let type_ = type_str.parse::<MessageType>()?;

        let version_str = parts.next().ok_or(Error::MissingField)?;
        if !version_str.is_empty() {
            return Err(Error::InvalidVersion);
        }

        let random_id_str = parts.next().ok_or(Error::MissingField)?;
        let random_id = if random_id_str.is_empty() {
            None
        } else {
            Some(
                random_id_str
                    .parse::<u64>()
                    .map_err(|_| Error::InvalidRandomId)?,
            )
        };

        let length_str = parts.next().ok_or(Error::MissingField)?;
        let length = if length_str.is_empty() {
            0
        } else {
            length_str
                .parse::<u64>()
                .map_err(|_| Error::InvalidLength)?
        };

        let content = parts.next().ok_or(Error::MissingField)?;

        // Special handling for trailing newlines in content
        let (actual_content, actual_length) = if content.ends_with('\n') {
            let trimmed = content.trim_end_matches('\n');
            (trimmed, trimmed.len() as u64)
        } else {
            (content, content.len() as u64)
        };

        if actual_length != length {
            return Err(Error::ContentLengthMismatch {
                expected: length,
                actual: actual_length,
            });
        }

        Ok(Message {
            type_,
            random_id,
            content: actual_content,
        })
    }

    pub fn content(&self) -> &str {
        self.content
    }

    pub fn topic_content(&self) -> Result<(&str, &str), Error> {
        let mut parts = self.content.splitn(2, '|');
        let topic = parts.next().ok_or(Error::MissingTopic)?;
        if topic.is_empty() {
            return Err(Error::MissingTopic);
        }
        let content = parts.next().ok_or(Error::MissingContent)?;

        Ok((topic, content))
    }

    pub fn proposal(&self) -> Result<LiquidexProposal<Unvalidated>, Error> {
        Ok(LiquidexProposal::from_str(self.content).map_err(|_| Error::InvalidProposal)?)
    }
}

pub fn proposal_topic(proposal: &LiquidexProposal<Validated>) -> Result<String, Error> {
    let topic = format!("{}|{}", proposal.input().asset, proposal.output().asset);
    Ok(topic)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proposal_str() -> &'static str {
        include_str!("../test_data/proposal.json")
    }

    #[test]
    fn test_from_str() {
        let message = Message::parse("PUBLISH||12345|25|topic|{\"message\":\"hello\"}").unwrap();
        assert_eq!(message.type_, MessageType::Publish);
        assert_eq!(message.random_id, Some(12345));
        assert_eq!(message.content, "topic|{\"message\":\"hello\"}");

        // Generic publish
        let message = Message::parse("PUBLISH||1|25|topic|{\"message\":\"hello\"}").unwrap();
        assert_eq!(message.type_, MessageType::Publish);
        assert_eq!(message.random_id, Some(1));
        assert_eq!(message.content, "topic|{\"message\":\"hello\"}");

        // Publish proposal (with placeholder content)
        let message = Message::parse("PUBLISH_PROPOSAL||1|9|$PROPOSAL").unwrap();
        assert_eq!(message.type_, MessageType::PublishProposal);
        assert_eq!(message.random_id, Some(1));
        assert_eq!(message.content, "$PROPOSAL");

        // Response - RESULT
        let message = Message::parse("RESULT||1|22|{\"response\":\"success\"}").unwrap();
        assert_eq!(message.type_, MessageType::Result);
        assert_eq!(message.random_id, Some(1));
        assert_eq!(message.content, "{\"response\":\"success\"}");

        // Response - ACK
        let message = Message::parse("RESULT|||0|").unwrap();
        assert_eq!(message.type_, MessageType::Result);
        assert_eq!(message.random_id, None);
        assert_eq!(message.content, "");

        // Ping
        let message = Message::parse("PING|||0|").unwrap();
        assert_eq!(message.type_, MessageType::Ping);
        assert_eq!(message.random_id, None);
        assert_eq!(message.content, "");

        let message = Message::parse("PING||||").unwrap();
        assert_eq!(message.type_, MessageType::Ping);
        assert_eq!(message.random_id, None);
        assert_eq!(message.content, "");

        // Pong
        let message = Message::parse("PONG|||0|").unwrap();
        assert_eq!(message.type_, MessageType::Pong);
        assert_eq!(message.random_id, None);
        assert_eq!(message.content, "");

        // Error
        let message = Message::parse("ERROR||1|12|InvalidTopic").unwrap();
        assert_eq!(message.type_, MessageType::Error);
        assert_eq!(message.random_id, Some(1));
        assert_eq!(message.content, "InvalidTopic");

        // Subscribe
        let message = Message::parse("SUBSCRIBE||1|8|mytopic1").unwrap();
        assert_eq!(message.type_, MessageType::Subscribe);
        assert_eq!(message.random_id, Some(1));
        assert_eq!(message.content, "mytopic1");
    }

    #[test]
    fn test_error_invalid_message_type() {
        let result = Message::parse("UNKNOWN||12345|0|");
        assert!(matches!(result, Err(Error::InvalidMessage)));
    }

    #[test]
    fn test_error_missing_field() {
        let result = Message::parse("PUBLISH||12345");
        assert_eq!(result, Err(Error::MissingField));
    }

    #[test]
    fn test_error_invalid_version() {
        let result = Message::parse("PUBLISH|invalid|12345|0|");
        assert!(matches!(result, Err(Error::InvalidVersion)));
        let result = Message::parse("PUBLISH|1|12345|0|");
        assert!(matches!(result, Err(Error::InvalidVersion)));
    }

    #[test]
    fn test_error_invalid_random_id() {
        let result = Message::parse("PUBLISH||invalid|0|");
        assert!(matches!(result, Err(Error::InvalidRandomId)));
    }

    #[test]
    fn test_error_invalid_length() {
        let result = Message::parse("PUBLISH||12345|invalid|");
        assert!(matches!(result, Err(Error::InvalidLength)));
    }

    #[test]
    fn test_error_content_length_mismatch() {
        // Test with content shorter than expected length
        let result = Message::parse("PUBLISH||12345|10|content");
        assert!(
            matches!(result, Err(Error::ContentLengthMismatch { expected, actual })
            if expected == 10 && actual == 7)
        );

        // Test with content longer than expected length (not due to trailing newlines)
        let result = Message::parse("PUBLISH||12345|7|content-too-long");
        assert!(
            matches!(result, Err(Error::ContentLengthMismatch { expected, actual })
            if expected == 7 && actual == 16)
        );

        // Test that trailing newlines are properly handled
        let result = Message::parse("PUBLISH||12345|7|content\n");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().content, "content");
    }

    #[test]
    fn test_topic_content() {
        // Test valid topic and content
        let message = Message::parse("PUBLISH||12345|25|topic|{\"message\":\"hello\"}").unwrap();
        let (topic, content) = message.topic_content().unwrap();
        assert_eq!(topic, "topic");
        assert_eq!(content, "{\"message\":\"hello\"}");

        // Test with empty content
        let message = Message::parse("PUBLISH||12345|6|topic|").unwrap();
        let (topic, content) = message.topic_content().unwrap();
        assert_eq!(topic, "topic");
        assert_eq!(content, "");

        // Test with missing content
        let message = Message::parse("PUBLISH||12345|5|topic").unwrap();
        assert_eq!(message.topic_content(), Err(Error::MissingContent));

        // Test with missing topic
        let message = Message::parse("PUBLISH||12345|0|").unwrap();
        assert_eq!(message.topic_content(), Err(Error::MissingTopic));
    }

    #[test]
    fn test_proposal() {
        // Test valid proposal
        let proposal_json = proposal_str();
        let message = format!("PUBLISH||12345|{}|{}", proposal_json.len(), proposal_json);
        let message = Message::parse(&message).unwrap();
        let proposal = message.proposal().unwrap();
        assert!(matches!(proposal, LiquidexProposal::<Unvalidated> { .. }));

        // Test invalid proposal
        let message = Message::parse("PUBLISH||12345|12|invalid_json").unwrap();
        assert_eq!(message.proposal(), Err(Error::InvalidProposal));
    }

    #[test]
    fn test_proposal_topic() {
        let proposal_json = proposal_str();
        let message = format!("PUBLISH||12345|{}|{}", proposal_json.len(), proposal_json);
        let message = Message::parse(&message).unwrap();
        let proposal = message.proposal().unwrap();
        let validated = proposal.insecure_validate().unwrap();
        let topic = proposal_topic(&validated).unwrap();
        assert_eq!(topic, "6921c799f7b53585025ae8205e376bfd2a7c0571f781649fb360acece252a6a7|f13806d2ab6ef8ba56fc4680c1689feb21d7596700af1871aef8c2d15d4bfd28");
    }
}
