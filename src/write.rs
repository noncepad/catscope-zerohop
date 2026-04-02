use solana_sdk::{clock::Slot, signature::Signature, transaction::TransactionError};

use crate::{err::CatscopeZerohopError, read::ClientId};

/// Interface for submitting transactions to the runtime.
pub trait TransactionProcessor: Send + Sync {
    /// Send out a single transaction. Do not wait for a result from the validator.
    /// Returns success if the validator receives the transaction to process.
    ///
    /// * `client_id` - Identifies the session submitting the transaction.
    /// * `data` - raw transaction bytes
    ///
    /// Returns an error if the transaction cannot be accepted
    fn send(&self, client_id: ClientId, data: &[u8]) -> Result<(), CatscopeZerohopError>;

    /// Send multiple transactions simultaneously. The default implementation just uses `send`.
    ///
    /// * `bundle` - A list of raw transaction byte slices
    ///
    /// Returns the first error encountered while sending the batch.
    fn batch(&self, client_id: ClientId, bundle: &[&[u8]]) -> Result<(), CatscopeZerohopError> {
        let n = bundle.len();
        for i in 0..n {
            let single = bundle[i];
            self.send(client_id, single)?;
        }
        Ok(())
    }

    fn tx_result(
        &self,
        signature: Signature,
        response_sx: flume::Sender<Result<Slot, TransactionError>>,
    );
}

/// Placeholder for when the scheduler becomes available.
///
/// # Example
///
/// ```ignore
/// // TODO: Add example when scheduler functionality is implemented
/// ```
pub trait Scheduler: Send + Sync {}
