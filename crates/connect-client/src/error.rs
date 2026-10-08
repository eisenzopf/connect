//! Typed transport, uncertainty, authentication and application failures.
use connect_protocol::Error;
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("Client configuration or trust material is invalid")]
    Configuration,
    #[error("Native transport disconnected or failed TLS verification")]
    Transport,
    #[error("Request deadline expired; acceptance is unknown; preserve the logical request ID")]
    Timeout,
    #[error("Operation cancelled; acceptance may be unknown; preserve the logical request ID")]
    Cancelled,
    #[error("Server rejected authentication or the required profile")]
    Authentication,
    #[error("Server response violates the negotiated contract")]
    Protocol,
    #[error(transparent)]
    Application(#[from] Error),
}
pub type Result<T> = std::result::Result<T, ClientError>;
