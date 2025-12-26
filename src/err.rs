use std::sync::Arc;

/// TODO: Describe the error types that can occur in Catscope Zerohop operations.
///
/// # Example
///
/// ```
/// use catscope_zerohop::err::CatscopeZerohopError;
///
/// // TODO: Show how to handle errors
/// // fn example() -> Result<(), CatscopeZerohopError> {
/// //     // Your code here
/// //     Ok(())
/// // }
/// ```
pub enum CatscopeZerohopError {
    /// TODO: Document when this error variant is used
    Unknown(String),
    NotImplemented,
    UnalignedMemory,
    TransactionFailure(Box<dyn std::error::Error + Send + Sync>),
    OutofRange,
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
        }
    }
}

impl std::fmt::Display for CatscopeZerohopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(arg0) => f.debug_tuple("Unknown").field(arg0).finish(),
            Self::NotImplemented => f.debug_tuple("Not implemented").finish(),
            Self::UnalignedMemory => f.debug_tuple("UnalignedMemory").finish(),
            Self::OutofRange => f.debug_tuple("OutofRange").finish(),
            Self::TransactionFailure(e) => write!(f, "TransactionFailure {}", e),
        }
    }
}
