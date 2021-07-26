use thiserror::Error;

#[derive(Error, Debug)]
pub enum KasaError {
    #[error("Empty auth response: code={code} message={message}")]
    EmptyAuthResponse { code: i32, message: String },
    #[error("Serialization error: debug={debug}")]
    Serialization {
        source: anyhow::Error,
        debug: String,
    },
    #[error("Passthrough param initialization error")]
    PassthroughParams { source: anyhow::Error },
    #[error("Empty passthrough response")]
    EmptyPassthroughResponse {},
    #[error("Empty emeter response")]
    EmptyEmeterResponse {},
}
