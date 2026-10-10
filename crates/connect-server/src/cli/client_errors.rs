//! Shared native helper and one-shot CLI failure classification.
use connect_client::ClientError;
use connect_protocol::Error;

pub(super) fn client_error(error: ClientError) -> Error {
    match error {
        ClientError::Application(error) => error,
        ClientError::Configuration => Error::new(
            "configuration",
            "Invalid client configuration or trust material",
        ),
        ClientError::Authentication => {
            Error::new("unauthorized", "Authentication or profile was rejected")
        }
        ClientError::Protocol => {
            Error::new("protocol", "Response violates the negotiated contract")
        }
        ClientError::Transport
        | ClientError::MediaEnded
        | ClientError::Timeout
        | ClientError::Cancelled => Error::new(
            "transport-uncertain",
            "Native exchange failed; preserve the logical request ID",
        ),
    }
}
