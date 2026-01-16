use crate::txfwd::TxFwdErrorCode;

/// Error types that can occur in Catscope Zerohop operations.
pub enum CatscopeZerohopError {
    Unknown(String),
    NotImplemented,

    /// Invalid or unsupported memory alignment.
    UnalignedMemory,

    /// A submitted transaction failed.
    TransactionFailure(Box<dyn std::error::Error + Send + Sync>),

    /// Value or index was out of bounds.
    OutofRange,

    /// Configuration or initialization error.
    ConfigError(String),

    /// Network or connection error.
    ConnectionError(String),
}

impl From<CatscopeZerohopError> for TxFwdErrorCode {
    fn from(e: CatscopeZerohopError) -> Self {
        match e {
            CatscopeZerohopError::Unknown(_) => 1,
            CatscopeZerohopError::NotImplemented => 2,
            CatscopeZerohopError::UnalignedMemory => 3,
            CatscopeZerohopError::TransactionFailure(_error) => 4,
            CatscopeZerohopError::OutofRange => 5,
            CatscopeZerohopError::ConfigError(_) => 6,
            CatscopeZerohopError::ConnectionError(_) => 7,
        }
    }
}

impl std::error::Error for CatscopeZerohopError {}
impl std::fmt::Debug for CatscopeZerohopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(arg0) => f.debug_tuple("Unknown").field(arg0).finish(),
            Self::NotImplemented => f.debug_tuple("Not implemented").finish(),
            Self::UnalignedMemory => f.debug_tuple("UnalignedMemory").finish(),
            Self::OutofRange => f.debug_tuple("OutofRange").finish(),
            Self::TransactionFailure(e) => write!(f, "TransactionFailure {}", e),
            Self::ConfigError(msg) => f.debug_tuple("ConfigError").field(msg).finish(),
            Self::ConnectionError(msg) => f.debug_tuple("ConnectionError").field(msg).finish(),
        }
    }
}

// Display mirrors Debug output
impl std::fmt::Display for CatscopeZerohopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(arg0) => f.debug_tuple("Unknown").field(arg0).finish(),
            Self::NotImplemented => f.debug_tuple("Not implemented").finish(),
            Self::UnalignedMemory => f.debug_tuple("UnalignedMemory").finish(),
            Self::OutofRange => f.debug_tuple("OutofRange").finish(),
            Self::TransactionFailure(e) => write!(f, "TransactionFailure {}", e),
            Self::ConfigError(msg) => f.debug_tuple("ConfigError").field(msg).finish(),
            Self::ConnectionError(msg) => f.debug_tuple("ConnectionError").field(msg).finish(),
        }
    }
}
impl From<Box<dyn std::error::Error>> for CatscopeZerohopError {
    fn from(e: Box<dyn std::error::Error>) -> Self {
        CatscopeZerohopError::Unknown(e.to_string())
    }
}
impl From<iceoryx2::service::service_name::ServiceNameError> for CatscopeZerohopError {
    fn from(e: iceoryx2::service::service_name::ServiceNameError) -> Self {
        CatscopeZerohopError::Unknown(e.to_string())
    }
}
impl From<iceoryx2::node::NodeCreationFailure> for CatscopeZerohopError {
    fn from(e: iceoryx2::node::NodeCreationFailure) -> Self {
        CatscopeZerohopError::Unknown(e.to_string())
    }
}
impl From<iceoryx2::service::builder::request_response::RequestResponseOpenOrCreateError>
    for CatscopeZerohopError
{
    fn from(
        e: iceoryx2::service::builder::request_response::RequestResponseOpenOrCreateError,
    ) -> Self {
        CatscopeZerohopError::Unknown(e.to_string())
    }
}
impl From<iceoryx2::service::port_factory::client::ClientCreateError> for CatscopeZerohopError {
    fn from(e: iceoryx2::service::port_factory::client::ClientCreateError) -> Self {
        CatscopeZerohopError::Unknown(e.to_string())
    }
}
