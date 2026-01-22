/// Core data structures shared between the CatScope runtime and plugins.
use std::{
    collections::HashSet,
    marker::PhantomData,
    mem::MaybeUninit,
    sync::{Arc, atomic::AtomicBool},
};

use agave_geyser_plugin_interface::geyser_plugin_interface::SlotStatus;
use solana_sdk::{
    clock::Slot, pubkey::Pubkey, signature::Signature, transaction::TransactionError,
};

use crate::{err::CatscopeZerohopError, util::bytes_to_struct};

/// ======================================================================
/// LIFECYCLE & IDENTITY
/// ======================================================================
/// The time and status of a block.
///
/// * `slot` - the chain clock value
/// * `status` - 0 = pending, 10 = failed, 12 = finalized
#[derive(Clone, Debug)]
pub struct SlotWithStatus {
    pub slot: Slot,
    pub status: SlotStatus,
}

/// Monotonically increasing `u64` version number assigned to every account version.
pub type Ticket = u64;

/// Catscope assigns a 1 to 1 mapping of `u64` to `Pubkey`
pub type AccountId = u64;

/// ======================================================================
/// SHARED MEMORY & ZERO-COPY PRIMITIVES
/// ======================================================================

/// Combined read/write interface for a shared memory blob.
pub trait BlobInterface: BlobView + BlobWrite {}

/// NamedBlob wraps a Blob.
///
/// * `T` - a 64 byte aligned, sized struct
pub struct NamedBlob<T: Sized> {
    blob: Arc<dyn BlobInterface>,
    _d: PhantomData<T>,
}
impl<T: Sized> NamedBlob<T> {
    pub fn new(blob: Arc<dyn BlobInterface>) -> Result<Self, CatscopeZerohopError> {
        let slice = blob.slice();
        let t_len = std::mem::size_of::<T>();
        if slice.len() % t_len != 0 {
            return Err(CatscopeZerohopError::UnalignedMemory);
        }
        Ok(NamedBlob {
            blob,
            _d: PhantomData::default(),
        })
    }

    /// Cast a byte slice as an array of T.
    ///
    /// Returns an array of T.
    pub fn vec_payload(&self) -> &[MaybeUninit<T>] {
        let slice = self.blob.slice();
        let n = std::mem::size_of::<MaybeUninit<T>>();
        assert_eq!(slice.len() % n, 0);
        let count = slice.len() / n;
        let ptr = slice.as_ptr() as *const _;
        unsafe { std::slice::from_raw_parts(ptr, count) }
    }
}

/// Write to a blob.
#[allow(clippy::mut_from_ref)]
pub trait BlobWrite: Send + Sync {
    /// Get a mutable slice to write into.
    ///
    /// Returns None if the slice has already been written to.
    fn slice_mut(&self) -> Option<&mut [u8]>;
}

/// Read-only view into a shared memory buffer.
/// View the underlying byte slice.
pub trait BlobView: Send + Sync {
    fn len(&self) -> usize;
    /// Get the length of the slice.

    /// Is the slice empty
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Access the underlying byte slice.
    fn slice(&self) -> &[u8];
}

/// ======================================================================
/// SOLANA ACCOUNT SNAPSHOTS
/// ======================================================================

/// Finalized snapshot of a Solana account.
#[derive(Clone)]
pub struct SolanaAccount {
    blob: Arc<dyn BlobView>,
    o_edge: Option<Arc<dyn BlobInterface>>,
    ticket: Ticket,
}

impl SolanaAccount {
    pub fn new(
        blob: Arc<dyn BlobInterface>,
        o_edge: Option<Arc<dyn BlobInterface>>,
        write_version: Ticket,
    ) -> Self {
        Self {
            blob,
            o_edge,
            ticket: write_version,
        }
    }
    pub fn slice(&self) -> &[u8] {
        self.blob.slice()
    }
    /// Return the write version number.
    pub fn ticket(&self) -> Ticket {
        self.ticket
    }

    /// Fixed metadata for the account (pubkey, owner, lamports, slot).
    pub fn header(&self) -> &AccountHeader {
        let slice = self.blob.slice();
        let ah_len = std::mem::size_of::<AccountHeader>();
        assert!(
            ah_len <= slice.len(),
            "bad slice len: {} vs {}",
            ah_len,
            slice.len()
        );
        bytes_to_struct(&slice[0..ah_len])
    }

    /// Raw account data, if present.
    pub fn data(&self) -> Option<&[u8]> {
        let slice = self.blob.slice();
        let ah_len = std::mem::size_of::<AccountHeader>();
        if slice.len() == ah_len {
            None
        } else {
            Some(&slice[ah_len..])
        }
    }

    /// Graph edges associated with this account.
    pub fn edge(&self) -> Option<&[AccountEdge]> {
        let blob = self.o_edge.as_ref()?;
        let slice = blob.slice();
        let e_len = std::mem::size_of::<AccountEdge>();
        assert!(!slice.is_empty() && slice.len() % e_len == 0);
        let ptr = slice.as_ptr() as *const AccountEdge;
        let l_edge = unsafe { std::slice::from_raw_parts(ptr, slice.len() / e_len) };
        Some(l_edge)
    }
}

impl std::fmt::Debug for SolanaAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SolanaAccount").finish()
    }
}

/// ======================================================================
/// ACCOUNT GRAPH MODEL
/// ======================================================================

/// This is weight on a graph edge.
pub type Weight = u32;

/// Depth is how far from subscription root account an account being traversed is.
pub type Depth = u8;

/// Directed relationship between two accounts.
#[repr(C, align(8))]
#[derive(Debug, Default, Clone, Eq)]
pub struct AccountEdge {
    // source account
    pub from: AccountId,

    // destination account
    pub to: AccountId,

    // edge weight
    pub weight: Weight,

    // deprecated
    pub slot: Slot,
}

impl PartialEq for AccountEdge {
    fn eq(&self, other: &Self) -> bool {
        self.from == other.from && self.to == other.to && self.weight == other.weight
    }
}

impl PartialOrd for AccountEdge {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match self.from.partial_cmp(&other.from) {
            Some(core::cmp::Ordering::Equal) => {}
            ord => {
                if ord.is_some() {
                    return ord;
                }
            }
        }
        match self.to.partial_cmp(&other.to) {
            Some(core::cmp::Ordering::Equal) => {}
            ord => {
                if ord.is_some() {
                    return ord;
                }
            }
        }
        let ord = self.weight.partial_cmp(&other.weight);
        if ord.is_some() {
            return ord;
        } else {
            return Some(std::cmp::Ordering::Equal);
        }
    }
}
impl Ord for AccountEdge {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.from.cmp(&other.from) {
            std::cmp::Ordering::Equal => {}
            std::cmp::Ordering::Greater => return std::cmp::Ordering::Greater,
            std::cmp::Ordering::Less => return std::cmp::Ordering::Less,
        };
        match self.to.cmp(&other.to) {
            std::cmp::Ordering::Equal => {}
            std::cmp::Ordering::Greater => return std::cmp::Ordering::Greater,
            std::cmp::Ordering::Less => return std::cmp::Ordering::Less,
        };
        match self.weight.cmp(&other.weight) {
            std::cmp::Ordering::Equal => {}
            std::cmp::Ordering::Greater => return std::cmp::Ordering::Greater,
            std::cmp::Ordering::Less => return std::cmp::Ordering::Less,
        };
        std::cmp::Ordering::Equal
    }
}

/// This is the header for a Solana account.
#[repr(C, align(8))]
#[derive(Default, Copy, Debug, Clone, PartialEq, Eq)]
pub struct AccountHeader {
    /// account public key - (the ecdsa 32B public key)
    pub pubkey: Pubkey,

    /// lamports in the account
    pub lamports: u64,

    /// account ID (the `u64` assigned to the `pubkey` by Catscope)
    pub account_id: AccountId,

    /// the program that owns this account. If executable, the program that loads this account.
    pub owner: Pubkey,

    ///  the epoch at which this account will next owe rent
    pub rent_epoch: u64,

    // the slot at which the transaction was registered
    pub slot: u64,

    // the size of the account data
    pub data_size: u32,

    //  This account's data contains a loaded program (and is now read-only)
    pub executable: bool,
}
impl AccountHeader {
    pub fn reset(&mut self) {
        self.pubkey = Pubkey::default();
        self.owner = self.pubkey;
        self.lamports = 0;
        self.account_id = 0;
        self.rent_epoch = 0;
        self.slot = 0;
        self.data_size = 0;
        self.executable = false;
    }
}
impl PartialOrd for AccountHeader {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for AccountHeader {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.slot.cmp(&other.slot)
    }
}

impl AccountEdge {
    pub fn account(&self) -> AccountId {
        if edge_is_outgoing(&self.weight) {
            self.from
        } else {
            self.to
        }
    }
}

#[inline(always)]
pub fn edge_is_outgoing(weight: &Weight) -> bool {
    0 < *weight & WEIGHT_IS_OUTGOING
}

/// Edge direction flag: indicates an outgoing edge
pub const WEIGHT_IS_OUTGOING: Weight = 1 << 0;
/// Edge type flag: slot-based relationship
pub const WEIGHT_SLOT: Weight = 1 << 1;
/// Edge type flag: client mapping (maps accounts to programs for client sync)
pub const WEIGHT_CLIENT: Weight = 1 << 2;
/// Edge type flag: upload the destination node to a client
pub const WEIGHT_UPLOAD: Weight = 1 << 3;
/// Maximum weight value for non-account relationships
pub const MAX_NON_ACCOUNT_WEIGHT: Weight = WEIGHT_UPLOAD;

/// Bitmask for all non-account weight flags
pub const WEIGHT_NON_ACCOUNT: Weight =
    WEIGHT_SLOT | WEIGHT_CLIENT | WEIGHT_UPLOAD | WEIGHT_IS_OUTGOING;
/// Bitmask for account-based weight values
pub const WEIGHT_ACCOUNT: Weight = !WEIGHT_NON_ACCOUNT;

pub const MAX_WEIGHT_NONACCOUNT_EXPONENT: u8 = 3;

pub const WEIGHT_PROGRAM: Weight = 1 << 4;
pub const WEIGHT_SPLTOKEN_OWNER: Weight = 1 << 5;
pub const WEIGHT_SPLTOKEN_MINT: Weight = 1 << 6;
pub const WEIGHT_DIRECT: Weight = 1 << 7;
pub const WEIGHT_SYMLINK: Weight = 1 << 8;

// the memory requirements grow exponentionally (doubles) for every integer increment of this
// variable.
pub const MAX_WEIGHT_ACCOUNT_EXPONENT: u8 = 9;
pub const MAX_WEIGHT: Weight = 1 << MAX_WEIGHT_ACCOUNT_EXPONENT;

/// Load the latest finalized version of a Solana account.
/// ```
pub trait ViewAccount {
    /// Returns a Solana account if it exists.
    fn load_account(&self, account_id: &AccountId) -> Option<SolanaAccount>;
}

/// Show a transaction result as recorded by the Geyser interface.
#[repr(C, align(8))]
pub struct TransactionResultHeader {
    /// The first signature in the transaction
    pub signature: Signature,
    /// The slot where transaction was processed
    pub slot: Slot,
    count: u32,
    /// The success/failure flag
    pub success: bool,
}

/// ======================================================================
/// TRANSACTION RESULTS
/// ======================================================================

pub struct TransactionResult {
    /// Decoded transaction and instruction data
    pub transaction: CatscopeTransaction,

    /// slot in which the transaction was processed
    pub slot: Slot,

    /// success or Solana runtime error
    pub result: Result<(), TransactionError>,
}

/// Zero-copy representation of a Solana transaction.
#[repr(C, align(64))]
pub struct CatscopeTransaction {
    /// Transaction signature
    pub signature: Signature,

    /// Index assigned by the runtime.
    pub index: u64,

    /// Number of outer instructions.
    outer_len: u16,
    /// Top-level instructions.
    outer: [CatscopeInstruction; 30],

    /// Number of inner-instruction groups.
    l1_inner_len: u8,
    /// Per-outer-instruction inner instruction counts.
    l1_inner: [u8; 30],

    /// Total number of inner instructions.
    inner_len: u16,
    /// Flattened inner instruction list.
    inner: [CatscopeInstruction; 60],

    /// Number of unique accounts touched.
    account_len: u16,
    /// Sorted list of account IDs touched by this transaction.
    account: [AccountId; 256],
}

impl Default for CatscopeTransaction {
    fn default() -> Self {
        Self {
            signature: Default::default(),
            index: Default::default(),
            outer_len: Default::default(),
            outer: Default::default(),
            l1_inner_len: Default::default(),
            l1_inner: Default::default(),
            inner_len: Default::default(),
            inner: [CatscopeInstruction::default(); 60],
            account_len: 0,
            account: [0; 256],
        }
    }
}

const CAP: usize = 1024;
impl CatscopeTransaction {
    /// Reset all instructions and account tracking.
    pub fn clear(&mut self) {
        let mut n = self.outer.len();
        for i in 0..n {
            self.outer[i].clear();
        }
        self.outer_len = 0;
        n = self.inner.len();
        for i in 0..n {
            self.inner[i].clear();
        }
        self.inner_len = 0;
        n = self.account.len();
        for i in 0..n {
            self.account[i] = 0;
        }
        self.account_len = 0;
    }

    /// Take accounts touched by this transaction.
    /// TODO: improve the efficiency
    pub fn set_account(&mut self) {
        let mut size = 0;
        for i in 0..self.outer_len {
            let outer = &self.outer[i as usize];
            for j in 0..outer.account_len {
                let account = outer.account[j as usize];
                let old_size = size;
                if size == 0 {
                    self.account[size] = account;
                    size += 1;
                } else if account < self.account[0] {
                    // prepend
                    size += 1;
                    let array = &mut self.account[0..size];
                    array.copy_within(0..old_size, 1);
                    array[0] = account;
                } else if self.account[size] < account {
                    // append
                    size += 1;
                    let array = &mut self.account[0..size];
                    array[old_size] = account;
                } else {
                    match &self.account[0..size].binary_search(&account) {
                        Ok(_) => {
                            // there is a duplicate
                        }
                        Err(pos) => {
                            // internal insert
                            let old_size = size;
                            size += 1;
                            let array = &mut self.account[0..size];
                            assert!(
                                *pos < array.len(),
                                "out of range: {} vs {}",
                                *pos,
                                array.len()
                            );
                            array.copy_within(*pos..old_size, *pos + 1);
                            array[*pos] = account;
                        }
                    };
                }
            }
        }
        self.account_len = size as _;
    }

    /// Set the number of outer instructions and return a mutable slice.
    pub fn set_outer(&mut self, size: usize) -> &mut [CatscopeInstruction] {
        self.outer_len = size as u16;
        &mut self.outer[0..size]
    }

    /// Append a group of inner instructions.
    pub fn append_inner(&mut self, size: usize) -> &mut [CatscopeInstruction] {
        let i = self.l1_inner_len as usize;
        let mut start = 0;
        for k in 0..i {
            start += self.l1_inner[k] as usize;
        }
        assert!(size <= u8::MAX as usize);
        self.l1_inner[i] = size as u8;
        self.l1_inner_len += 1;
        self.inner_len = size as u16;
        &mut self.inner[start..(start + size)]
    }

    /// Accounts touched by this transaction.
    pub fn account(&self) -> &[AccountId] {
        &self.account[0..(self.account_len as usize)]
    }

    /// Return the top-level instructions of the transaction.
    /// These are the instructions submitted by the client and define the execution flow.
    pub fn outer(&self) -> &[CatscopeInstruction] {
        &self.outer[0..(self.outer_len as usize)]
    }

    /// Return the inner (CPI-generated) instructions of the transaction.
    pub fn inner(&self) -> &[CatscopeInstruction] {
        &self.inner[0..(self.inner_len as usize)]
    }
}
impl std::fmt::Debug for CatscopeTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatscopeTransaction")
            .field("signature", &self.signature)
            .field("index", &self.index)
            .field("outer_len", &self.outer_len)
            .field("l1_inner_len", &self.l1_inner_len)
            .field("inner_len", &self.inner_len)
            .field("account_len", &self.account_len)
            .field("account", &self.account)
            .finish()
    }
}

/// TODO: Document the Catscope instruction structure.
///
/// # Fields
///
/// * `program` - TODO: Document the program account ID
/// * `data` - TODO: Document instruction data (max 2KB)
/// * `account` - TODO: Document account list (max 256 accounts)
///
/// # Limits
///
/// * MAX_DATA: 2048 bytes
/// * MAX_ACCOUNT: 256 accounts

/// A single CatScope instrcution
///
/// This is used to record what was executed during transaction processing,
/// both for instructions explicitly submitted by the client and instructions
/// invoked indirectly during execution.
#[derive(Clone, Copy)]
pub struct CatscopeInstruction {
    /// AccountId of the program that executed this instruction.
    program: AccountId,

    /// Length of the instruction data actually used.
    data_len: u16,

    /// Raw instruction data payload (truncated to MAX_DATA).
    data: [u8; MAX_DATA],

    /// Number of accounts referenced by this instruction.
    account_len: u16,

    /// AccountIds touched by this instruction (ordered as executed).
    account: [AccountId; MAX_ACCOUNT],
}

impl Default for CatscopeInstruction {
    fn default() -> Self {
        Self {
            program: Default::default(),
            data_len: Default::default(),
            data: [0u8; MAX_DATA],
            account_len: Default::default(),
            account: [0; MAX_ACCOUNT],
        }
    }
}
impl CatscopeInstruction {
    /// Set the program that executed this instruction.
    #[inline]
    pub fn set_program(&mut self, program_id: AccountId) {
        self.program = program_id;
    }

    /// Allocate space for `size` account references and return them for filling.
    #[inline]
    pub fn set_account(&mut self, size: usize) -> &mut [AccountId] {
        assert!(
            size < self.account.len(),
            "bad account length_: {} {} {}",
            self.account_len,
            self.account.len(),
            size
        );
        self.account_len = size as u16;
        &mut self.account[0..size]
    }

    /// Copy raw instruction data into the fixed buffer.
    #[inline]
    pub fn set_data(&mut self, data: &[u8]) {
        assert!(
            data.len() <= self.data.len(),
            "bad data: {} {}",
            data.len(),
            self.data.len()
        );
        self.data_len = data.len() as u16;
        let subbuf = &mut self.data[0..data.len()];
        subbuf.copy_from_slice(data);
    }

    /// Clear all fields so the instruction can be reused.
    #[inline]
    pub fn clear(&mut self) {
        self.program = 0;
        let buf = &mut self.data[0..];
        unsafe {
            std::ptr::write_bytes(buf.as_mut_ptr(), 0, self.data_len as usize);
        }
        self.data_len = 0;
        for i in 0..self.account_len {
            self.account[i as usize] = 0;
        }
        self.account_len = 0;
    }

    /// Returns the program that executed this instruction.
    #[inline]
    pub fn program(&self) -> AccountId {
        self.program
    }

    /// Return the raw instruction data used during execution.
    /// TODO: Document the data slice (up to 2KB)
    #[inline]
    pub fn data(&self) -> &[u8] {
        &self.data[0..(self.data_len as usize)]
    }

    /// Returns the accounts referenced by this instruction.
    /// TODO: Document the account ID slice (up to 256 accounts)
    #[inline]
    pub fn account(&self) -> &[AccountId] {
        &self.account[0..(self.account_len as usize)]
    }
}
impl std::fmt::Debug for CatscopeInstruction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatscopeInstructionFull").finish()
    }
}

pub trait CatscopeTransactionResult: Send + Sync {
    fn result(&self) -> Result<Slot, TransactionError>;
    fn tx(&self) -> &CatscopeTransaction;
    fn slot(&self) -> Slot;
}

const MAX_DATA: usize = 2 * 1024;
const MAX_ACCOUNT: usize = 256;

pub fn convert_slot_status_from(status: SlotStatus) -> u8 {
    match status {
        SlotStatus::Processed => 1,
        SlotStatus::Rooted => 2,
        SlotStatus::Confirmed => 3,
        SlotStatus::FirstShredReceived => 4,
        SlotStatus::Completed => 5,
        SlotStatus::CreatedBank => 6,
        SlotStatus::Dead(_) => 7,
    }
}

pub fn convert_slot_status_to(statusu8: u8) -> Result<SlotStatus, CatscopeZerohopError> {
    let x = match statusu8 {
        1 => SlotStatus::Processed,
        2 => SlotStatus::Rooted,
        3 => SlotStatus::Confirmed,
        4 => SlotStatus::FirstShredReceived,
        5 => SlotStatus::Completed,
        6 => SlotStatus::CreatedBank,
        7 => SlotStatus::Dead(String::from("blank")),
        _ => return Err(CatscopeZerohopError::OutofRange),
    };
    Ok(x)
}
