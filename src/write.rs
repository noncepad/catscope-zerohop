use crate::{err::CatscopeZerohopError, read::ClientId};

/// TODO: Describe the purpose of this trait and when to implement it.
///
/// Send transactions out.
/// Receive the results from CatscopeReader.
///
/// # Example
///
/// ```ignore
/// use catscope_zerohop::write::TransactionProcessor;
/// use catscope_zerohop::err::CatscopeZerohopError;
/// use catscope_zerohop::read::ClientId;
///
/// struct MyProcessor {
///     // TODO: Add your fields
/// }
///
/// impl TransactionProcessor for MyProcessor {
///     fn send(&self, client_id: ClientId, data: &[u8]) -> Result<(), CatscopeZerohopError> {
///         // TODO: Implement transaction sending logic
///         Ok(())
///     }
/// }
/// ```
pub trait TransactionProcessor: Send + Sync {
    /// TODO: Document the expected behavior and error conditions.
    ///
    /// Send out a single transaction. Do not wait for a result from the validator.
    /// Returns success if the validator receives the transaction to process.
    ///
    /// # Arguments
    ///
    /// * `client_id` - TODO: Explain what the client_id represents
    /// * `data` - TODO: Explain the format and contents of the transaction data
    ///
    /// # Errors
    ///
    /// TODO: Document specific error conditions
    fn send(&self, client_id: ClientId, data: &[u8]) -> Result<(), CatscopeZerohopError>;

    /// TODO: Document when batching is beneficial vs individual sends.
    ///
    /// Send multiple transactions simultaneously. The default implementation just uses `send`.
    ///
    /// # Arguments
    ///
    /// * `client_id` - TODO: Explain the client context for this batch
    /// * `bundle` - TODO: Explain constraints on bundle size or contents
    ///
    /// # Errors
    ///
    /// TODO: Document error handling for partial batch failures
    fn batch(&self, client_id: ClientId, bundle: &[&[u8]]) -> Result<(), CatscopeZerohopError> {
        let n = bundle.len();
        for i in 0..n {
            let single = bundle[i];
            self.send(client_id, single)?;
        }
        Ok(())
    }
}

/// TODO: Document the scheduling functionality this trait will provide.
///
/// Placeholder for when the scheduler becomes available.
///
/// # Example
///
/// ```ignore
/// // TODO: Add example when scheduler functionality is implemented
/// ```
pub trait Scheduler: Send + Sync {}
