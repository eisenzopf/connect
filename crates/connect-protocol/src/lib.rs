//! Transport-independent Connect application contract carried by UCTP envelopes.

use serde::{Deserialize, Serialize};

pub const PROFILE: &str = "connect-control/1";
pub const REQUIRED_SCOPE: &str = "connect:control";
pub const REQUEST_TYPE: &str = "connect.request";
pub const RESULT_TYPE: &str = "connect.result";
/// Leave framing headroom for result metadata/cursors below UCTP's 1 MiB cap.
pub const MAX_PAGE_BYTES: usize = 512 * 1024;

/// The transport envelope ID correlates one exchange. `request_id` identifies a
/// logical mutation across retries, transports, and server restarts.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Request {
    pub profile: String,
    #[serde(flatten)]
    pub operation: Operation,
}

impl Request {
    pub fn new(operation: Operation) -> Self {
        Self {
            profile: PROFILE.into(),
            operation,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Resolve {
        address: String,
    },
    Create {
        request_id: String,
        participants: Vec<String>,
    },
    Send {
        request_id: String,
        conversation_id: String,
        body: String,
    },
    History {
        conversation_id: String,
        after: Option<String>,
        limit: u32,
    },
    Inbox {
        cursor: Option<String>,
        limit: u32,
        wait_ms: u32,
    },
    RequestStatus {
        request_id: String,
    },
    Close {
        request_id: String,
        conversation_id: String,
    },
    Health {},
}

impl Operation {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Resolve { .. } => "resolve",
            Self::Create { .. } => "create",
            Self::Send { .. } => "send",
            Self::History { .. } => "history",
            Self::Inbox { .. } => "inbox",
            Self::RequestStatus { .. } => "request_status",
            Self::Close { .. } => "close",
            Self::Health {} => "health",
        }
    }
    pub fn request_id(&self) -> Option<&str> {
        match self {
            Self::Create { request_id, .. }
            | Self::Send { request_id, .. }
            | Self::Close { request_id, .. } => Some(request_id),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Identity {
    pub id: String,
    pub alias: String,
    /// Durable declared capability, independent of whether a client is online.
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Conversation {
    pub id: String,
    pub participants: Vec<String>,
    pub closed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub sender: String,
    pub body: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    pub id: String,
    pub kind: String,
    pub conversation_id: String,
    pub message: Option<Message>,
    pub cursor: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Outcome {
    Identity(Identity),
    Conversation(Conversation),
    Accepted {
        request_id: String,
        message_id: String,
        conversation_id: String,
    },
    History {
        messages: Vec<Message>,
        next: Option<String>,
    },
    Inbox {
        events: Vec<Event>,
        cursor: Option<String>,
    },
    RequestStatus {
        state: String,
        outcome: Option<Box<Outcome>>,
    },
    Health {
        installation_id: String,
        schema: u32,
        profile: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, thiserror::Error, PartialEq, Eq)]
#[error("{code}: {message}")]
pub struct Error {
    pub code: String,
    pub message: String,
}

impl Error {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
    pub fn unauthorized() -> Self {
        Self::new("unauthorized", "Credential is invalid, expired, or revoked")
    }
    pub fn forbidden() -> Self {
        Self::new("forbidden", "Operation or resource is not permitted")
    }
    pub fn storage() -> Self {
        Self::new("storage-unavailable", "Durable storage is unavailable")
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub profile: String,
    pub result: Result<Outcome>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub message_bytes: usize,
    pub page_size: u32,
    pub journal_events: u64,
    #[serde(
        default = "default_receipt_limit",
        skip_serializing_if = "is_default_receipt_limit"
    )]
    pub operation_receipts: u64,
    pub message_storage_bytes: u64,
    pub participants: usize,
    pub wait_ms: u32,
    pub request_timeout_ms: u64,
    pub connections: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            message_bytes: 16_384,
            page_size: 100,
            journal_events: 100_000,
            operation_receipts: default_receipt_limit(),
            message_storage_bytes: 64 * 1024 * 1024,
            participants: 8,
            wait_ms: 5000,
            request_timeout_ms: 10_000,
            connections: 64,
        }
    }
}
fn default_receipt_limit() -> u64 {
    100_000
}
fn is_default_receipt_limit(value: &u64) -> bool {
    *value == default_receipt_limit()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_payload_roundtrips_and_rejects_spoofing() {
        let request = Request::new(Operation::Send {
            request_id: "logical-1".into(),
            conversation_id: "conversation-1".into(),
            body: "hello".into(),
        });
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(request, serde_json::from_value(value.clone()).unwrap());
        let mut spoofed = value;
        spoofed["sender"] = serde_json::json!("other-actor");
        assert!(serde_json::from_value::<Request>(spoofed).is_err());
    }

    #[test]
    fn operations_reject_claimed_sender_and_unknown_fields() {
        assert!(serde_json::from_str::<Operation>(r#"{"op":"send","request_id":"r","conversation_id":"c","body":"hello","sender":"other"}"#).is_err());
        for operation in [
            Operation::Health {},
            Operation::Resolve {
                address: "connect:reviews@local".into(),
            },
            Operation::Inbox {
                cursor: None,
                limit: 5,
                wait_ms: 0,
            },
        ] {
            let json = serde_json::to_string(&operation).unwrap();
            assert_eq!(operation, serde_json::from_str(&json).unwrap());
        }
    }
}
