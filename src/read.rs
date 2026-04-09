use std::{
    collections::{HashMap, HashSet},
    hash::BuildHasherDefault,
    sync::{Arc, atomic::AtomicU32},
};

use crate::{
    err::CatscopeZerohopError,
    store::{
        AccountId, BlobView, CatscopeTransaction, CatscopeTransactionResult, Depth, RollingWindow,
        SlotWithStatus, SolanaAccount, TransactionResult, Weight,
    },
    usage::{Capacity, Usage},
};
use solana_sdk::{
    clock::Slot, hash::Hash, pubkey::Pubkey, signature::Signature, transaction::TransactionError,
};
use twox_hash::XxHash64;

/// ======================================================================
/// GRAPH CLIENT (ENTRY POINT)
/// ======================================================================
/// GraphClient defines the **streaming read interface** exposed to ZeroHop plugins.
///
/// Plugin lifecycle (read only):
/// 1. Call `connect()` once to open a read session
/// 2. Receive a channel group for this session
/// 3. Create subscriptions and listen for events
pub trait GraphClient: Send + Sync + Capacity {
    /// Open a read session.
    ///
    /// Returns the channels the plugin will listen on.
    /// If this fails, the plugin should treat it as fatal and stop.
    fn connect(
        &self,
        include_low_latency: bool,
    ) -> Result<Box<dyn CatscopeReadChannelGroup>, CatscopeZerohopError>;

    fn poller(&self) -> Box<dyn CatscopeBuffer>;

    /// Returns the most recent blockhash known to the runtime.
    ///
    /// Exposed so plugins can stay in sync with the chain head
    /// and align their logic with current block production.
    fn blockhash(&self) -> Option<Hash>;

    /// Streams slot updates.
    ///
    /// Returns an error if slot updates cannot be provided.
    /// Plugins should treat this as a loss of signal and assume pipeline is unhealthy
    fn slot(&self) -> Result<flume::Receiver<SlotWithStatus>, CatscopeZerohopError>;

    fn neighbor(
        &self,
        account_id: AccountId,
        direction: Direction,
        m_peer: &mut HashMap<AccountId, Weight, BuildHasherDefault<XxHash64>>,
    );
}

/// Direction of relationship traversal in the account graph.
#[derive(Clone, Copy)]
pub enum Direction {
    Incoming,
    Outgoing,
}

/// ======================================================================
///  ACCOUNT ID ↔ PUBKEY MAPPING
/// ======================================================================
///
/// PubkeyMap provides a translation layer between Solana pubkeys
/// and CatScope’s internal AccountId representation.
///
/// Internally, CatScope operates on compact AccountIds for faster graph
/// trasnversal and scalability. Plugins use this trait to:
/// - register accounts they care about (via pubkey → AccountId)
/// - translate internal identifiers back to Solana pubkeys
pub trait PubkeyMap: Send + Sync {
    /// Translate an internal AccountId back into a Solana Pubkey.
    ///
    /// Returns `Some(pubkey)` if the account is known to the runtime.
    /// Returns `None` if the AccountId is unknown or no longer tracked.
    fn account_id(&self, account_id: &AccountId) -> Option<Pubkey>;

    /// Translate a Solana Pubkey into an internal AccountId.
    ///
    /// If the pubkey is not yet known, this call may register it
    /// for tracking and return a newly assigned AccountId.
    fn pubkey(&self, pubkey: &Pubkey) -> AccountId;
}

/// ======================================================================
/// CLIENTS AND SUBSCRIPTIONS
/// ======================================================================
///
/// Each call to `connect()` creates a client.
/// Each client can create multiple subscriptions.
///
/// A subscription defines:
/// - where to start (root)
/// - how far to expand (depth)
/// - what signal to ignore (weight threshold)

/// Identifier for a plugin read connection. Created on `connect()` and owns all subscriptions.
pub type ClientId = u32;

/// Identifier for a single graph subscription. Lives under a client and is removed on cancel or disconnect.
pub type SubscriptionId = u32;

/// Internal control messages for managing read clients and subscriptions.
pub enum ClientMessage {
    /// Register a new read client.
    Connect(ClientId),
    /// Disconnect a client and remove all of its subscriptions.
    Disconnect(ClientId),
    /// Create a new subscription for a client.
    Subscribe(ClientId, SubscriptionId, SubscriptionDetail),
    /// Cancel an existing subscription.
    Cancel(ClientId, SubscriptionId),
}

/// Parameters for a graph subscription.
pub struct SubscriptionDetail {
    /// Account to start graph traversal from (root node = 1).
    pub root: AccountId,

    /// Edge weight filter applied during traversal.
    pub filter_weight: Weight,

    /// How many edges deep the traversal may go.
    pub depth: Depth,
}

/// ======================================================================
/// Account and Transaction Buffer
/// ======================================================================
///
/// Poll the buffer to see if there are any updates.
///
/// Used to create subscriptions and receive updates from the runtime.
pub trait CatscopeBuffer: Send + Sync {
    /// poll returns references to non-finalized accounts and transactions.
    fn poll<'a, 'b: 'a>(
        &'b mut self,
        hs_account_id: &'b HashSet<AccountId, BuildHasherDefault<XxHash64>>,
        o_hs_signature: Option<&'b HashSet<Signature, BuildHasherDefault<XxHash64>>>,
    ) -> Option<StateWindow<'a>>;
}

/// ======================================================================
/// READ CHANNEL GROUP
/// ======================================================================
///
/// Handle for a plugin’s active read session.
///
/// Used to create subscriptions and receive updates from the runtime.
pub trait CatscopeReadChannelGroup: Send + Sync {
    /// Identifier for this read client.
    fn id(&self) -> ClientId;

    /// Main even stream. Receives events from active subscriptions.
    fn event_rx(&self) -> flume::Receiver<Event>;

    /// Request cancellation of a subscription.
    fn cancel(&self, subscription_id: SubscriptionId);

    /// Request a new subscription.
    fn subscribe(
        &self,
        root_id: AccountId,
        filter_weight: Weight,
        depth: Depth,
    ) -> Result<SubscriptionId, CatscopeZerohopError>;
}

// ======================================================================
/// EVENTS
/// ======================================================================
///
/// TODO: Document the event types received from subscriptions.
///
/// # Variants
///
/// * `Ack` - TODO: Document acknowledgment events
/// * `Commit` - TODO: Document commit events
pub enum Event {
    /// notification that a subscription has landed
    Ack(SubscriptionId),
    /// finalized account state
    Commit(Box<dyn Commit>),
    /// get the unconfirmed account and transactions as they arrive over the gossip protocol
    LowLatency(Vec<SolanaAccount>, Vec<Arc<dyn CatscopeTransactionResult>>),
}

/// ======================================================================
/// COMMITS
/// ======================================================================
///
/// A batch of finalized account updates for a single slot.
///
/// A Commit is delivered after the runtime determines which accounts
/// changed in a slot. Plugins iterate through the accounts in the
/// commit and update their internal state accordingly.
pub trait Commit: Send + Sync {
    /// Slot in which these updates were finalized.
    fn slot(&self) -> Slot;

    /// Returns the next account that changed in this slot.
    /// Returns `None` when all updates for the slot have been consumed.
    fn next(&mut self) -> Option<SolanaAccount>;
}

pub struct StateWindow<'a> {
    pub hs_intersect: &'a HashSet<AccountId, BuildHasherDefault<XxHash64>>,
    pub m_tx:
        &'a HashMap<Signature, Arc<dyn CatscopeTransactionResult>, BuildHasherDefault<XxHash64>>,
    pub l_account: &'a Vec<SolanaAccount>,
    pub l_tx: &'a Vec<Signature>,
}

impl<'a> std::fmt::Debug for StateWindow<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateWindow")
            .field("hs_intersect", &self.hs_intersect)
            .field("l_account", &self.l_account)
            .field("l_tx", &self.l_tx)
            .finish()
    }
}
