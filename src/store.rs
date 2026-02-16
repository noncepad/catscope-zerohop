use agave_geyser_plugin_interface::geyser_plugin_interface::SlotStatus;
use solana_sdk::signature::SIGNATURE_BYTES;
use solana_sdk::{
    clock::Slot, pubkey::Pubkey, signature::Signature, transaction::TransactionError,
};
use std::alloc::{Layout, alloc, alloc_zeroed};
/// Core data structures shared between the CatScope runtime and plugins.
use std::{
    collections::HashSet,
    marker::PhantomData,
    mem::MaybeUninit,
    sync::{Arc, atomic::AtomicBool},
};
use wincode::{SchemaRead, SchemaWrite};

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
#[derive(Debug, Default, Clone, Copy, Eq)]
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
#[derive(SchemaWrite, SchemaRead)]
#[repr(C, align(64))]
pub struct CatscopeTransaction {
    /// Transaction signature
    pub signature: [u8; SIGNATURE_BYTES],

    /// Index assigned by the runtime.
    pub index: u64,

    data_chunk_last: Vec<u16>,
    data_chunk: Vec<u8>,

    account_chunk_last: Vec<u16>,
    account_chunk: Vec<AccountId>,

    /// Top-level instructions.
    outer: Vec<CatscopeInstruction>,

    /// Per-outer-instruction inner instruction counts.
    l1_inner: Vec<u8>,

    /// Flattened inner instruction list.
    inner: Vec<CatscopeInstruction>,

    /// Sorted list of account IDs touched by this transaction.
    account: Vec<AccountId>,
}
pub struct CatscopeTransactionReadWrapper<'a> {
    tx: &'a CatscopeTransaction,
}
impl<'a> TryFrom<&[u8]> for CatscopeTransactionReadWrapper<'a> {
    type Error = CatscopeZerohopError;

    fn try_from(data: &[u8]) -> Result<Self, Self::Error> {
        todo!()
    }
}
impl Default for CatscopeTransaction {
    fn default() -> Self {
        Self {
            signature: [0u8; SIGNATURE_BYTES],
            index: 0,
            outer: Vec::with_capacity(2),
            l1_inner: Vec::with_capacity(6),
            inner: Vec::with_capacity(2),
            account: Vec::with_capacity(6),
            account_chunk_last: Vec::with_capacity(6),
            account_chunk: Vec::with_capacity(6),
            data_chunk_last: Vec::with_capacity(6),
            data_chunk: Vec::with_capacity(6),
        }
    }
}

const OUTER_MAX: usize = 512;
const INNER_MAX: usize = 512;
const ACCOUNT_MAX: usize = 512;
const DATA_MAX: usize = 4 * 1024;

const CAP: usize = 1024;
impl CatscopeTransaction {
    /// Set the number of outer instructions and return a mutable slice.
    pub fn append_outer<'a, 'b: 'a>(&'b mut self) -> CatscopeInstructionWrite<'a> {
        let i = IxIndex::Outer(self.outer.len());
        self.outer.push(CatscopeInstruction::default());
        let account_chunk_i = self.account_chunk_last.len();
        self.account_chunk_last.push(0);
        let data_chunk_i = self.data_chunk_last.len();
        self.data_chunk_last.push(0);
        CatscopeInstructionWrite {
            i,
            account_chunk_i,
            data_chunk_i,
            account_set: false,
            data_set: false,
            tx: self,
        }
    }

    /// Append a group of inner instructions.
    pub fn append_inner<'a, 'b: 'a>(&'b mut self) -> CatscopeInstructionWrite<'a> {
        let i = IxIndex::Inner(self.inner.len());
        self.inner.push(CatscopeInstruction::default());
        let account_chunk_i = self.account_chunk_last.len();
        self.account_chunk_last.push(0);
        let data_chunk_i = self.data_chunk_last.len();
        self.data_chunk_last.push(0);

        CatscopeInstructionWrite {
            i,
            account_chunk_i,
            data_chunk_i,
            account_set: false,
            data_set: false,
            tx: self,
        }
    }

    /// Accounts touched by this transaction.
    pub fn account(&self) -> &[AccountId] {
        &self.account
    }

    /// Return the top-level instructions of the transaction.
    /// These are the instructions submitted by the client and define the execution flow.
    pub fn outer<'a, 'b: 'a>(&'b self) -> CatscopeInstructionReadOuterIterator<'a> {
        CatscopeInstructionReadOuterIterator { i: 0, tx: self }
    }

    /// Return the inner (CPI-generated) instructions of the transaction.
    pub fn inner<'a, 'b: 'a>(&'b self) -> CatscopeInstructionReadInnerIterator<'a> {
        CatscopeInstructionReadInnerIterator { i: 0, tx: self }
    }
}
impl std::fmt::Debug for CatscopeTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatscopeTransaction")
            .field("signature", &self.signature)
            .field("index", &self.index)
            .field("account", &self.account)
            .finish()
    }
}
pub struct CatscopeInstructionReadInnerIterator<'a> {
    i: usize,
    tx: &'a CatscopeTransaction,
}

impl<'a> Iterator for CatscopeInstructionReadInnerIterator<'a> {
    type Item = CatscopeInstructionRead<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.tx.inner.len() <= self.i {
            None
        } else {
            let r = CatscopeInstructionRead {
                i: IxIndex::Inner(self.i),
                tx: self.tx,
            };
            self.i += 1;
            Some(r)
        }
    }
}

pub struct CatscopeInstructionReadOuterIterator<'a> {
    i: usize,
    tx: &'a CatscopeTransaction,
}

impl<'a> Iterator for CatscopeInstructionReadOuterIterator<'a> {
    type Item = CatscopeInstructionRead<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.tx.outer.len() <= self.i {
            None
        } else {
            let r = CatscopeInstructionRead {
                i: IxIndex::Outer(self.i),
                tx: self.tx,
            };
            self.i += 1;
            Some(r)
        }
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
/// * ACCOUNT_MAX: 256 accounts

/// A single CatScope instrcution
///
/// This is used to record what was executed during transaction processing,
/// both for instructions explicitly submitted by the client and instructions
/// invoked indirectly during execution.
#[derive(Clone, Copy, Default, SchemaWrite, SchemaRead)]
#[repr(C, align(8))]
struct CatscopeInstruction {
    /// AccountId of the program that executed this instruction.
    program: AccountId,
    data_chunk_i: u16,
    account_chunk_i: u16,
}

pub struct CatscopeInstructionRead<'a> {
    i: IxIndex,
    tx: &'a CatscopeTransaction,
}

impl<'a> CatscopeInstructionRead<'a> {
    /// Returns the program that executed this instruction.
    #[inline]
    pub fn program(&self) -> &AccountId {
        match self.i {
            IxIndex::Outer(i) => &self.tx.outer[i].program,
            IxIndex::Inner(i) => &self.tx.inner[i].program,
        }
    }

    /// Return the raw instruction data used during execution.
    pub fn data(&self) -> &[u8] {
        let ix = match self.i {
            IxIndex::Outer(i) => &self.tx.outer[i],
            IxIndex::Inner(i) => &self.tx.inner[i],
        };
        let chunk_i = ix.data_chunk_i as usize;
        let (start, finish) = if chunk_i == 0 {
            (0, self.tx.data_chunk_last[chunk_i])
        } else {
            (
                self.tx.data_chunk_last[chunk_i - 1],
                self.tx.data_chunk_last[chunk_i],
            )
        };
        &self.tx.data_chunk[(start as usize)..(finish as usize)]
    }

    /// Returns the accounts referenced by this instruction.
    pub fn account(&self) -> &[AccountId] {
        let ix = match self.i {
            IxIndex::Outer(i) => &self.tx.outer[i],
            IxIndex::Inner(i) => &self.tx.inner[i],
        };
        let chunk_i = ix.account_chunk_i as usize;
        let (start, finish) = if chunk_i == 0 {
            (0, self.tx.account_chunk_last[chunk_i])
        } else {
            (
                self.tx.account_chunk_last[chunk_i - 1],
                self.tx.account_chunk_last[chunk_i],
            )
        };
        &self.tx.account_chunk[(start as usize)..(finish as usize)]
    }
}

enum IxIndex {
    Outer(usize),
    Inner(usize),
}

pub struct CatscopeInstructionWrite<'a> {
    i: IxIndex,
    data_chunk_i: usize,
    account_chunk_i: usize,
    account_set: bool,
    data_set: bool,
    tx: &'a mut CatscopeTransaction,
}
impl<'a> CatscopeInstructionWrite<'a> {
    /// Set the program that executed this instruction.
    #[inline]
    pub fn program(&mut self, program_id: AccountId) {
        match self.i {
            IxIndex::Outer(i) => {
                self.tx.outer[i].program = program_id;
            }
            IxIndex::Inner(i) => {
                self.tx.inner[i].program = program_id;
            }
        };
    }

    /// Allocate space for `size` account references and return them for filling.
    #[inline]
    pub fn account(&mut self, size: usize) -> &mut [AccountId] {
        assert!(!self.account_set);
        self.account_set = true;
        let start = self.tx.account_chunk.len();
        let finish = start + size;
        self.tx
            .account_chunk
            .resize(self.tx.account_chunk.len() + size, 0);
        self.tx.account_chunk_last[self.account_chunk_i] = finish as u16;
        &mut self.tx.account_chunk[start..finish]
    }

    /// Allocate space for `size` account references and return them for filling.
    #[inline]
    pub fn data(&mut self, size: usize) -> &mut [u8] {
        assert!(!self.data_set);
        self.data_set = true;
        let start = self.tx.data_chunk.len();
        let finish = start + size;
        self.tx
            .data_chunk
            .resize(self.tx.data_chunk.len() + size, 0);
        self.tx.data_chunk_last[self.data_chunk_i] = finish as u16;
        &mut self.tx.data_chunk[start..finish]
    }
}
impl<'a> Drop for CatscopeInstructionWrite<'a> {
    fn drop(&mut self) {
        assert!(self.account_set);
        assert!(self.data_set);
        let ix = match self.i {
            IxIndex::Outer(i) => &mut self.tx.outer[i],
            IxIndex::Inner(i) => &mut self.tx.inner[i],
        };
        let chunk_i = ix.account_chunk_i as usize;
        let (start, finish) = if chunk_i == 0 {
            (0, self.tx.account_chunk_last[chunk_i])
        } else {
            (
                self.tx.account_chunk_last[chunk_i - 1],
                self.tx.account_chunk_last[chunk_i],
            )
        };
        assert!((start == 0 && finish == 0) || 0 < finish);
        for i in (start as usize)..(finish as usize) {
            let account_id = self.tx.account_chunk[i];
            match self.tx.account.binary_search(&account_id) {
                Ok(_) => {
                    // duplicate
                }
                Err(i) => self.tx.account.insert(i, account_id),
            };
        }
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

mod tests {
    use solana_sdk::pubkey::Pubkey;

    use crate::store::{AccountId, CatscopeTransaction};

    #[test]
    fn test_append() {
        let program_id = 2183483;
        let check_l_account_id: [AccountId; 2] = [32, 85];
        let ix_data: [u8; 4] = [0, 1, 13, 0];
        let mut catx = CatscopeTransaction::default();
        {
            let mut outer = catx.append_outer();
            outer.program(program_id);
            let l_a = outer.account(2);
            l_a.copy_from_slice(&check_l_account_id);
            let d = outer.data(ix_data.len());
            d.copy_from_slice(&ix_data);
        }
        {
            for ix in catx.outer() {
                assert_eq!(*ix.program(), program_id);
                let l_a_id = ix.account();
                assert_eq!(l_a_id.len(), check_l_account_id.len());
                for j in 0..check_l_account_id.len() {
                    assert_eq!(l_a_id[j], check_l_account_id[j], "bad j {j}");
                }
            }
        }
    }

    #[test]
    fn test_append_inner() {
        let program_id = 2183483;
        let check_l_account_id: [AccountId; 2] = [32, 85];
        let ix_data: [u8; 4] = [0, 1, 13, 0];
        let mut catx = CatscopeTransaction::default();
        {
            let mut inner = catx.append_inner();
            inner.program(program_id);
            let l_a = inner.account(2);
            l_a.copy_from_slice(&check_l_account_id);
            let d = inner.data(ix_data.len());
            d.copy_from_slice(&ix_data);
        }
        {
            for ix in catx.inner() {
                assert_eq!(*ix.program(), program_id);
                let l_a_id = ix.account();
                assert_eq!(l_a_id.len(), check_l_account_id.len());
                for j in 0..check_l_account_id.len() {
                    assert_eq!(l_a_id[j], check_l_account_id[j], "bad j {j}");
                }
            }
        }
    }
}
