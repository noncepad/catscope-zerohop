use std::{
    collections::HashMap,
    sync::{Arc, atomic::AtomicU32},
};

use solana_sdk::{
    clock::Slot, hash::Hash, pubkey::Pubkey, signature::Signature, transaction::TransactionError,
};

use crate::{
    err::CatscopeZerohopError,
    store::{
        AccountId, BlobView, CatscopeTransaction, CatscopeTransactionResult, Depth, SlotWithStatus,
        SolanaAccount, TransactionResult, Weight,
    },
};

/// TODO: Document the purpose of GraphClient and its role in the system.
///
/// # Example
///
/// ```ignore
/// use catscope_zerohop::read::GraphClient;
///
/// struct MyGraphClient {
///     // TODO: Add your fields
/// }
///
/// impl GraphClient for MyGraphClient {
///     fn connect(&self) -> Result<CatscopeReadChannelGroup, CatscopeZerohopError> {
///         // TODO: Implement connection logic
///     }
///
///     fn blockhash(&self) -> Option<Hash> {
///         // TODO: Implement blockhash retrieval
///     }
///
///     fn slot(&self) -> Result<flume::Receiver<SlotWithStatus>, CatscopeZerohopError> {
///         // TODO: Implement slot stream
///     }
/// }
/// ```
pub trait GraphClient: Send + Sync {
    /// TODO: Document what connecting does and what the channel group is for.
    ///
    /// # Errors
    ///
    /// TODO: Document when this returns an error
    fn connect(&self) -> Result<Box<dyn CatscopeReadChannelGroup>, CatscopeZerohopError>;

    /// TODO: Document when this returns None vs Some and what the blockhash is used for.
    fn blockhash(&self) -> Option<Hash>;

    /// TODO: Document the slot stream and what consumers should do with it.
    ///
    /// # Errors
    ///
    /// TODO: Document error conditions
    fn slot(&self) -> Result<flume::Receiver<SlotWithStatus>, CatscopeZerohopError>;
    fn neighbor(
        &self,
        account_id: AccountId,
        direction: Direction,
        list: &mut HashMap<AccountId, Weight>,
    );
}

#[derive(Clone, Copy)]
pub enum Direction {
    Incoming,
    Outgoing,
}
/// TODO: Document the purpose of this trait and the mapping system.
///
/// Translate between AccountId and Pubkey.
///
/// # Example
///
/// ```ignore
/// use catscope_zerohop::read::PubkeyMap;
/// use catscope_zerohop::store::AccountId;
/// use solana_sdk::pubkey::Pubkey;
///
/// struct MyPubkeyMap {
///     // TODO: Add your storage mechanism
/// }
///
/// impl PubkeyMap for MyPubkeyMap {
///     fn account_id_exists(&self, account_id: &AccountId) -> Option<Pubkey> {
///         // TODO: Implement lookup
///         None
///     }
///
///     fn pubkey(&self, pubkey: &Pubkey) -> AccountId {
///         // TODO: Implement mapping/creation
///         0
///     }
/// }
/// ```
pub trait PubkeyMap: Send + Sync {
    /// TODO: Document what it means if this returns Some vs None.
    ///
    /// # Returns
    ///
    /// TODO: Explain the return value meaning
    fn account_id(&self, account_id: &AccountId) -> Option<Pubkey>;

    /// TODO: Document whether this creates a new mapping if it doesn't exist.
    ///
    /// # Arguments
    ///
    /// * `pubkey` - TODO: Document the pubkey being looked up or created
    fn pubkey(&self, pubkey: &Pubkey) -> AccountId;
}

/// TODO: Document the purpose of viewing accounts and caching behavior.
///
/// # Example
///
/// ```ignore
/// use catscope_zerohop::read::ViewAccount;
/// use catscope_zerohop::store::{AccountId, SolanaAccount};
///
/// struct MyAccountView {
///     // TODO: Add your storage
/// }
///
/// impl ViewAccount for MyAccountView {
///     fn get(&self, account_id: &AccountId) -> Option<SolanaAccount> {
///         // TODO: Implement account retrieval
///         None
///     }
/// }
/// ```
pub trait ViewAccount: Send + Sync {
    /// TODO: Document what "finalized" means and when this returns None.
    ///
    /// View the latest finalized version of an Account.
    ///
    /// # Returns
    ///
    /// TODO: Document when None is returned vs Some
    fn get(&self, account_id: &AccountId) -> Option<SolanaAccount>;
}

/// TODO: Document what a subscription ID represents and its lifecycle
pub type SubscriptionId = u32;

/// TODO: Document what a client ID represents and how clients are managed
pub type ClientId = u32;

/// TODO: Document the client message protocol and when each variant is used.
///
/// # Variants
///
/// * `Disconnect` - TODO: Document disconnect behavior
/// * `Subscribe` - TODO: Document subscription setup
/// * `Cancel` - TODO: Document subscription cancellation
pub enum ClientMessage {
    Connect(ClientId),
    /// TODO: Document what happens when a client disconnects
    Disconnect(ClientId),
    /// TODO: Document subscription parameters
    Subscribe(ClientId, SubscriptionId, SubscriptionDetail),
    /// TODO: Document cancellation behavior
    Cancel(ClientId, SubscriptionId),
}

/// TODO: Document what a subscription detail configures.
///
/// # Fields
///
/// * `root` - TODO: Explain what the root account means
/// * `filter_weight` - TODO: Explain the weight filtering system
/// * `depth` - TODO: Explain depth limits
pub struct SubscriptionDetail {
    /// TODO: Document the root account selection
    pub root: AccountId,
    /// TODO: Document how weight filtering works
    pub filter_weight: Weight,
    /// TODO: Document depth limits and traversal
    pub depth: Depth,
}

pub trait CatscopeReadChannelGroup: Send + Sync {
    fn id(&self) -> ClientId;
    /// TODO: Document what events are received on this channel
    fn event_rx(&self) -> flume::Receiver<Event>;
    /// TODO: Document slot status updates
    fn slot_status_rx(&self) -> flume::Receiver<SlotWithStatus>;
    fn cancel(&self, subscription_id: SubscriptionId);
    fn subscribe(
        &self,
        root_id: AccountId,
        filter_weight: Weight,
        depth: Depth,
    ) -> Result<SubscriptionId, CatscopeZerohopError>;
}

/// TODO: Document the event types received from subscriptions.
///
/// # Variants
///
/// * `Ack` - TODO: Document acknowledgment events
/// * `Commit` - TODO: Document commit events
pub enum Event {
    /// TODO: Document what this acknowledgment means
    Ack(SubscriptionId),
    /// TODO: Document commit data structure
    Commit(Box<dyn Commit>),
    /// Transaction resutl
    TransactionResult(Arc<dyn CatscopeTransactionResult>),
}

/// TODO: Document the commit trait and iteration pattern.
///
/// # Example
///
/// ```ignore
/// use catscope_zerohop::read::Commit;
///
/// fn process_commit(commit: &dyn Commit) {
///     // TODO: Add example of iterating through commit
///     // while let Some(account) = commit.next() {
///     //     // Process account
///     // }
/// }
/// ```
pub trait Commit: Send + Sync {
    /// TODO: Document which slot this commit is for
    fn slot(&self) -> Slot;

    /// TODO: Document the iteration pattern and when this returns None
    ///
    /// # Returns
    ///
    /// TODO: Document what None means vs Some
    fn next(&mut self) -> Option<SolanaAccount>;
}
