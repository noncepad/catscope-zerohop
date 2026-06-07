use agave_geyser_plugin_interface::geyser_plugin_interface::SlotStatus;
use solana_sdk::signature::SIGNATURE_BYTES;
use solana_sdk::{
    clock::Slot, instruction::InstructionError, pubkey::Pubkey, signature::Signature,
    transaction::TransactionError,
};
use std::time::{Duration, Instant};
/// Core data structures shared between the CatScope runtime and plugins.
use std::{marker::PhantomData, mem::MaybeUninit, sync::Arc};
use wincode::{SchemaRead, SchemaWrite};

use crate::{
    err::CatscopeZerohopError,
    util::{as_bytes, as_bytes_slice, bytes_to_struct},
};

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
        if !slice.len().is_multiple_of(t_len) {
            return Err(CatscopeZerohopError::UnalignedMemory);
        }
        Ok(NamedBlob {
            blob,
            _d: PhantomData,
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
    /// slice returns the header and body.
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

#[allow(clippy::non_canonical_partial_ord_impl)]
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
            ord
        } else {
            Some(std::cmp::Ordering::Equal)
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

    ///  the monotonically increasing account version number local to the validator
    pub write_version: u64,

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
        self.write_version = 0;
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
// ---------------------------------------------------------------------------
// Wire format
// ---------------------------------------------------------------------------
//
// Layout (all little-endian, naturally aligned):
//
//   CatscopeTransactionHeader          96 bytes  (fixed)
//   outer:              outer_len × 16 bytes  (CatscopeInstruction, align 8)
//   inner:              inner_len × 16 bytes  (CatscopeInstruction, align 8)
//   account:            account_len × 8 bytes  (AccountId = u64,    align 8)
//   account_chunk:  account_chunk_len × 8 bytes  (AccountId,        align 8)
//   account_chunk_last: ix_count × 2 bytes  (u16, ix_count = outer+inner, align 2)
//   data_chunk_last:    ix_count × 2 bytes  (u16,                   align 2)
//   l1_inner:           outer_len × 1 byte   (u8)
//   data_chunk:     data_chunk_len × 1 byte   (u8)
//
// The header is 96 bytes so all 8-byte-aligned sections start correctly
// provided the buffer itself is at least 8-byte aligned (true for any
// heap allocation or mmap).

#[repr(C)]
struct CatscopeTransactionHeader {
    signature: [u8; SIGNATURE_BYTES], // offset  0, 64 bytes
    index: u64,                       // offset 64
    outer_len: u32,                   // offset 72
    inner_len: u32,                   // offset 76
    account_len: u32,                 // offset 80
    account_chunk_len: u32,           // offset 84
    data_chunk_len: u32,              // offset 88
    _pad: u32,                        // offset 92 → total 96, align 8
}

impl CatscopeTransaction {
    /// Serialize into `buf` using the catscope wire format.
    /// No intermediate allocation: each field is appended in one `extend_from_slice`.
    pub fn write_to(&self, buf: &mut Vec<u8>) {
        let outer_len = self.outer.len() as u32;
        let inner_len = self.inner.len() as u32;
        let ix_count = (outer_len + inner_len) as usize;
        debug_assert_eq!(self.account_chunk_last.len(), ix_count);
        debug_assert_eq!(self.data_chunk_last.len(), ix_count);
        debug_assert_eq!(self.l1_inner.len(), outer_len as usize);

        let hdr = CatscopeTransactionHeader {
            signature: self.signature,
            index: self.index,
            outer_len,
            inner_len,
            account_len: self.account.len() as u32,
            account_chunk_len: self.account_chunk.len() as u32,
            data_chunk_len: self.data_chunk.len() as u32,
            _pad: 0,
        };
        buf.extend_from_slice(as_bytes(&hdr));
        buf.extend_from_slice(as_bytes_slice(&self.outer));
        buf.extend_from_slice(as_bytes_slice(&self.inner));
        buf.extend_from_slice(as_bytes_slice(&self.account));
        buf.extend_from_slice(as_bytes_slice(&self.account_chunk));
        buf.extend_from_slice(as_bytes_slice(&self.account_chunk_last));
        buf.extend_from_slice(as_bytes_slice(&self.data_chunk_last));
        buf.extend_from_slice(&self.l1_inner);
        buf.extend_from_slice(&self.data_chunk);
    }
}

// ---------------------------------------------------------------------------
// Zero-copy read view
// ---------------------------------------------------------------------------

/// Zero-copy view of a serialized [`CatscopeTransaction`].
///
/// Constructed via `TryFrom<&'a [u8]>` — no allocation, all fields are
/// slices into the original buffer.
pub struct CatscopeTransactionReadWrapper<'a> {
    pub signature: &'a [u8; SIGNATURE_BYTES],
    pub index: u64,
    pub account: &'a [AccountId],
    pub l1_inner: &'a [u8],
    pub data_chunk: &'a [u8],
    outer: &'a [CatscopeInstruction],
    inner: &'a [CatscopeInstruction],
    account_chunk: &'a [AccountId],
    account_chunk_last: &'a [u16],
    data_chunk_last: &'a [u16],
}

impl<'a> CatscopeTransactionReadWrapper<'a> {
    /// Iterate over outer (client-submitted) instructions.
    pub fn outer(&'a self) -> impl Iterator<Item = CatscopeInstructionViewRead<'a>> {
        (0..self.outer.len()).map(move |i| CatscopeInstructionViewRead {
            i: IxIndex::Outer(i),
            view: self,
        })
    }

    /// Iterate over inner (CPI) instructions.
    pub fn inner(&'a self) -> impl Iterator<Item = CatscopeInstructionViewRead<'a>> {
        (0..self.inner.len()).map(move |i| CatscopeInstructionViewRead {
            i: IxIndex::Inner(i),
            view: self,
        })
    }
}

/// Instruction accessor for a zero-copy [`CatscopeTransactionReadWrapper`].
pub struct CatscopeInstructionViewRead<'a> {
    i: IxIndex,
    view: &'a CatscopeTransactionReadWrapper<'a>,
}

impl<'a> CatscopeInstructionViewRead<'a> {
    pub fn program(&self) -> &AccountId {
        match self.i {
            IxIndex::Outer(i) => &self.view.outer[i].program,
            IxIndex::Inner(i) => &self.view.inner[i].program,
        }
    }

    pub fn data(&self) -> &[u8] {
        let ix = match self.i {
            IxIndex::Outer(i) => &self.view.outer[i],
            IxIndex::Inner(i) => &self.view.inner[i],
        };
        let chunk_i = ix.data_chunk_i as usize;
        let (start, finish) = if chunk_i == 0 {
            (0, self.view.data_chunk_last[chunk_i])
        } else {
            (
                self.view.data_chunk_last[chunk_i - 1],
                self.view.data_chunk_last[chunk_i],
            )
        };
        &self.view.data_chunk[(start as usize)..(finish as usize)]
    }

    pub fn account(&self) -> &[AccountId] {
        let ix = match self.i {
            IxIndex::Outer(i) => &self.view.outer[i],
            IxIndex::Inner(i) => &self.view.inner[i],
        };
        let chunk_i = ix.account_chunk_i as usize;
        let (start, finish) = if chunk_i == 0 {
            (0, self.view.account_chunk_last[chunk_i])
        } else {
            (
                self.view.account_chunk_last[chunk_i - 1],
                self.view.account_chunk_last[chunk_i],
            )
        };
        &self.view.account_chunk[(start as usize)..(finish as usize)]
    }
}

impl<'a> TryFrom<&'a [u8]> for CatscopeTransactionReadWrapper<'a> {
    type Error = CatscopeZerohopError;

    fn try_from(data: &'a [u8]) -> Result<Self, Self::Error> {
        use std::mem::{align_of, size_of};

        // --- helper: advance `offset` by `count` elements of type T,
        //     returning a typed slice aliasing `data` ---
        unsafe fn take_slice<'b, T>(
            data: &'b [u8],
            offset: &mut usize,
            count: usize,
        ) -> Result<&'b [T], CatscopeZerohopError> {
            let byte_len = count
                .checked_mul(size_of::<T>())
                .ok_or(CatscopeZerohopError::OutofRange)?;
            let start = *offset;
            let end = start
                .checked_add(byte_len)
                .ok_or(CatscopeZerohopError::OutofRange)?;
            if end > data.len() {
                return Err(CatscopeZerohopError::OutofRange);
            }
            let ptr = data[start..end].as_ptr() as *const T;
            if !(ptr as usize).is_multiple_of(align_of::<T>()) {
                return Err(CatscopeZerohopError::UnalignedMemory);
            }
            *offset = end;
            #[allow(unsafe_op_in_unsafe_fn)]
            Ok(std::slice::from_raw_parts(ptr, count))
        }

        let hdr_size = std::mem::size_of::<CatscopeTransactionHeader>();
        if data.len() < hdr_size {
            return Err(CatscopeZerohopError::OutofRange);
        }

        // SAFETY: header is repr(C) with all valid bit patterns; we checked
        // both the length and alignment of the buffer.
        let hdr = unsafe {
            let ptr = data.as_ptr() as *const CatscopeTransactionHeader;
            if !(ptr as usize).is_multiple_of(std::mem::align_of::<CatscopeTransactionHeader>()) {
                return Err(CatscopeZerohopError::UnalignedMemory);
            }
            &*ptr
        };

        let outer_len = hdr.outer_len as usize;
        let inner_len = hdr.inner_len as usize;
        let account_len = hdr.account_len as usize;
        let account_chunk_len = hdr.account_chunk_len as usize;
        let data_chunk_len = hdr.data_chunk_len as usize;
        let ix_count = outer_len
            .checked_add(inner_len)
            .ok_or(CatscopeZerohopError::OutofRange)?;

        let mut off = hdr_size;
        // SAFETY: each take_slice call checks bounds and alignment.
        unsafe {
            let outer = take_slice::<CatscopeInstruction>(data, &mut off, outer_len)?;
            let inner = take_slice::<CatscopeInstruction>(data, &mut off, inner_len)?;
            let account = take_slice::<AccountId>(data, &mut off, account_len)?;
            let account_chunk = take_slice::<AccountId>(data, &mut off, account_chunk_len)?;
            let account_chunk_last = take_slice::<u16>(data, &mut off, ix_count)?;
            let data_chunk_last = take_slice::<u16>(data, &mut off, ix_count)?;
            let l1_inner = take_slice::<u8>(data, &mut off, outer_len)?;
            let data_chunk = take_slice::<u8>(data, &mut off, data_chunk_len)?;

            // signature lives inside the header, which is borrowed from `data`
            let signature = &*(hdr.signature.as_ptr() as *const [u8; SIGNATURE_BYTES]);

            Ok(Self {
                signature,
                index: hdr.index,
                outer,
                inner,
                account,
                account_chunk,
                account_chunk_last,
                data_chunk_last,
                l1_inner,
                data_chunk,
            })
        }
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

impl CatscopeTransaction {
    /// Set the number of outer instructions and return a mutable slice.
    pub fn append_outer<'a, 'b: 'a>(&'b mut self) -> CatscopeInstructionWrite<'a> {
        let i = IxIndex::Outer(self.outer.len());
        self.outer.push(CatscopeInstruction::default());
        self.l1_inner.push(0);
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

    /// Append an inner instruction belonging to the outer instruction at `outer_idx`.
    pub fn append_inner<'a, 'b: 'a>(
        &'b mut self,
        outer_idx: usize,
    ) -> CatscopeInstructionWrite<'a> {
        self.l1_inner[outer_idx] += 1;
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
///   A single CatScope instrcution
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

// ---------------------------------------------------------------------------
// Result<Slot, TransactionError> — 16-byte encoding
// ---------------------------------------------------------------------------
//
//  [0]     result_tag:  0 = Ok,  1 = Err
//  [1]     tx_disc:     TransactionError variant index (0-based, enum declaration order)
//  [2]     ie_disc:     InstructionError variant index (only when tx_disc == 8)
//  [3]     aux_u8:      • InstructionError(u8, _)               → instruction index
//                       • DuplicateInstruction(u8)              → instruction index
//                       • InsufficientFundsForRent{account_index}
//                       • ProgramExecutionTemporarilyRestricted{account_index}
//  [4..8]  custom_u32:  InstructionError::Custom(u32) payload, little-endian
//  [8..16] slot:        Ok(Slot) value, little-endian

/// Encode `Result<Slot, TransactionError>` into exactly 16 bytes.
pub fn result_to_bytes(result: &Result<Slot, TransactionError>) -> [u8; 16] {
    let mut buf = [0u8; 16];
    match result {
        Ok(slot) => {
            buf[0] = 0;
            buf[8..16].copy_from_slice(&slot.to_le_bytes());
        }
        Err(e) => {
            buf[0] = 1;
            let (tx_disc, aux, ie_disc, custom) = encode_tx_error(e);
            buf[1] = tx_disc;
            buf[2] = ie_disc;
            buf[3] = aux;
            buf[4..8].copy_from_slice(&custom.to_le_bytes());
        }
    }
    buf
}

/// Decode `Result<Slot, TransactionError>` from 16 bytes produced by [`result_to_bytes`].
pub fn result_from_bytes(
    buf: &[u8; 16],
) -> Result<Result<Slot, TransactionError>, CatscopeZerohopError> {
    match buf[0] {
        0 => {
            let slot = u64::from_le_bytes(buf[8..16].try_into().unwrap());
            Ok(Ok(slot))
        }
        1 => {
            let tx_disc = buf[1];
            let ie_disc = buf[2];
            let aux = buf[3];
            let custom = u32::from_le_bytes(buf[4..8].try_into().unwrap());
            Ok(Err(decode_tx_error(tx_disc, aux, ie_disc, custom)?))
        }
        _ => Err(CatscopeZerohopError::OutofRange),
    }
}

fn encode_tx_error(e: &TransactionError) -> (u8, u8, u8, u32) {
    // returns (tx_disc, aux_u8, ie_disc, custom_u32)
    match e {
        TransactionError::AccountInUse => (0, 0, 0, 0),
        TransactionError::AccountLoadedTwice => (1, 0, 0, 0),
        TransactionError::AccountNotFound => (2, 0, 0, 0),
        TransactionError::ProgramAccountNotFound => (3, 0, 0, 0),
        TransactionError::InsufficientFundsForFee => (4, 0, 0, 0),
        TransactionError::InvalidAccountForFee => (5, 0, 0, 0),
        TransactionError::AlreadyProcessed => (6, 0, 0, 0),
        TransactionError::BlockhashNotFound => (7, 0, 0, 0),
        TransactionError::InstructionError(ix, ie) => {
            let (ie_disc, custom) = encode_instruction_error(ie);
            (8, *ix, ie_disc, custom)
        }
        TransactionError::CallChainTooDeep => (9, 0, 0, 0),
        TransactionError::MissingSignatureForFee => (10, 0, 0, 0),
        TransactionError::InvalidAccountIndex => (11, 0, 0, 0),
        TransactionError::SignatureFailure => (12, 0, 0, 0),
        TransactionError::InvalidProgramForExecution => (13, 0, 0, 0),
        TransactionError::SanitizeFailure => (14, 0, 0, 0),
        TransactionError::ClusterMaintenance => (15, 0, 0, 0),
        TransactionError::AccountBorrowOutstanding => (16, 0, 0, 0),
        TransactionError::WouldExceedMaxBlockCostLimit => (17, 0, 0, 0),
        TransactionError::UnsupportedVersion => (18, 0, 0, 0),
        TransactionError::InvalidWritableAccount => (19, 0, 0, 0),
        TransactionError::WouldExceedMaxAccountCostLimit => (20, 0, 0, 0),
        TransactionError::WouldExceedAccountDataBlockLimit => (21, 0, 0, 0),
        TransactionError::TooManyAccountLocks => (22, 0, 0, 0),
        TransactionError::AddressLookupTableNotFound => (23, 0, 0, 0),
        TransactionError::InvalidAddressLookupTableOwner => (24, 0, 0, 0),
        TransactionError::InvalidAddressLookupTableData => (25, 0, 0, 0),
        TransactionError::InvalidAddressLookupTableIndex => (26, 0, 0, 0),
        TransactionError::InvalidRentPayingAccount => (27, 0, 0, 0),
        TransactionError::WouldExceedMaxVoteCostLimit => (28, 0, 0, 0),
        TransactionError::WouldExceedAccountDataTotalLimit => (29, 0, 0, 0),
        TransactionError::DuplicateInstruction(ix) => (30, *ix, 0, 0),
        TransactionError::InsufficientFundsForRent { account_index } => (31, *account_index, 0, 0),
        TransactionError::MaxLoadedAccountsDataSizeExceeded => (32, 0, 0, 0),
        TransactionError::InvalidLoadedAccountsDataSizeLimit => (33, 0, 0, 0),
        TransactionError::ResanitizationNeeded => (34, 0, 0, 0),
        TransactionError::ProgramExecutionTemporarilyRestricted { account_index } => {
            (35, *account_index, 0, 0)
        }
        TransactionError::UnbalancedTransaction => (36, 0, 0, 0),
        TransactionError::ProgramCacheHitMaxLimit => (37, 0, 0, 0),
        TransactionError::CommitCancelled => (38, 0, 0, 0),
    }
}

fn encode_instruction_error(ie: &InstructionError) -> (u8, u32) {
    // returns (ie_disc, custom_u32)
    match ie {
        InstructionError::GenericError => (0, 0),
        InstructionError::InvalidArgument => (1, 0),
        InstructionError::InvalidInstructionData => (2, 0),
        InstructionError::InvalidAccountData => (3, 0),
        InstructionError::AccountDataTooSmall => (4, 0),
        InstructionError::InsufficientFunds => (5, 0),
        InstructionError::IncorrectProgramId => (6, 0),
        InstructionError::MissingRequiredSignature => (7, 0),
        InstructionError::AccountAlreadyInitialized => (8, 0),
        InstructionError::UninitializedAccount => (9, 0),
        InstructionError::UnbalancedInstruction => (10, 0),
        InstructionError::ModifiedProgramId => (11, 0),
        InstructionError::ExternalAccountLamportSpend => (12, 0),
        InstructionError::ExternalAccountDataModified => (13, 0),
        InstructionError::ReadonlyLamportChange => (14, 0),
        InstructionError::ReadonlyDataModified => (15, 0),
        InstructionError::DuplicateAccountIndex => (16, 0),
        InstructionError::ExecutableModified => (17, 0),
        InstructionError::RentEpochModified => (18, 0),
        #[allow(deprecated)]
        InstructionError::NotEnoughAccountKeys => (19, 0),
        InstructionError::AccountDataSizeChanged => (20, 0),
        InstructionError::AccountNotExecutable => (21, 0),
        InstructionError::AccountBorrowFailed => (22, 0),
        InstructionError::AccountBorrowOutstanding => (23, 0),
        InstructionError::DuplicateAccountOutOfSync => (24, 0),
        InstructionError::Custom(code) => (25, *code),
        InstructionError::InvalidError => (26, 0),
        InstructionError::ExecutableDataModified => (27, 0),
        InstructionError::ExecutableLamportChange => (28, 0),
        InstructionError::ExecutableAccountNotRentExempt => (29, 0),
        InstructionError::UnsupportedProgramId => (30, 0),
        InstructionError::CallDepth => (31, 0),
        InstructionError::MissingAccount => (32, 0),
        InstructionError::ReentrancyNotAllowed => (33, 0),
        InstructionError::MaxSeedLengthExceeded => (34, 0),
        InstructionError::InvalidSeeds => (35, 0),
        InstructionError::InvalidRealloc => (36, 0),
        InstructionError::ComputationalBudgetExceeded => (37, 0),
        InstructionError::PrivilegeEscalation => (38, 0),
        InstructionError::ProgramEnvironmentSetupFailure => (39, 0),
        InstructionError::ProgramFailedToComplete => (40, 0),
        InstructionError::ProgramFailedToCompile => (41, 0),
        InstructionError::Immutable => (42, 0),
        InstructionError::IncorrectAuthority => (43, 0),
        InstructionError::BorshIoError => (44, 0),
        InstructionError::AccountNotRentExempt => (45, 0),
        InstructionError::InvalidAccountOwner => (46, 0),
        InstructionError::ArithmeticOverflow => (47, 0),
        InstructionError::UnsupportedSysvar => (48, 0),
        InstructionError::IllegalOwner => (49, 0),
        InstructionError::MaxAccountsDataAllocationsExceeded => (50, 0),
        InstructionError::MaxAccountsExceeded => (51, 0),
        InstructionError::MaxInstructionTraceLengthExceeded => (52, 0),
        InstructionError::BuiltinProgramsMustConsumeComputeUnits => (53, 0),
    }
}

fn decode_tx_error(
    tx_disc: u8,
    aux: u8,
    ie_disc: u8,
    custom: u32,
) -> Result<TransactionError, CatscopeZerohopError> {
    let e = match tx_disc {
        0 => TransactionError::AccountInUse,
        1 => TransactionError::AccountLoadedTwice,
        2 => TransactionError::AccountNotFound,
        3 => TransactionError::ProgramAccountNotFound,
        4 => TransactionError::InsufficientFundsForFee,
        5 => TransactionError::InvalidAccountForFee,
        6 => TransactionError::AlreadyProcessed,
        7 => TransactionError::BlockhashNotFound,
        8 => TransactionError::InstructionError(aux, decode_instruction_error(ie_disc, custom)?),
        9 => TransactionError::CallChainTooDeep,
        10 => TransactionError::MissingSignatureForFee,
        11 => TransactionError::InvalidAccountIndex,
        12 => TransactionError::SignatureFailure,
        13 => TransactionError::InvalidProgramForExecution,
        14 => TransactionError::SanitizeFailure,
        15 => TransactionError::ClusterMaintenance,
        16 => TransactionError::AccountBorrowOutstanding,
        17 => TransactionError::WouldExceedMaxBlockCostLimit,
        18 => TransactionError::UnsupportedVersion,
        19 => TransactionError::InvalidWritableAccount,
        20 => TransactionError::WouldExceedMaxAccountCostLimit,
        21 => TransactionError::WouldExceedAccountDataBlockLimit,
        22 => TransactionError::TooManyAccountLocks,
        23 => TransactionError::AddressLookupTableNotFound,
        24 => TransactionError::InvalidAddressLookupTableOwner,
        25 => TransactionError::InvalidAddressLookupTableData,
        26 => TransactionError::InvalidAddressLookupTableIndex,
        27 => TransactionError::InvalidRentPayingAccount,
        28 => TransactionError::WouldExceedMaxVoteCostLimit,
        29 => TransactionError::WouldExceedAccountDataTotalLimit,
        30 => TransactionError::DuplicateInstruction(aux),
        31 => TransactionError::InsufficientFundsForRent { account_index: aux },
        32 => TransactionError::MaxLoadedAccountsDataSizeExceeded,
        33 => TransactionError::InvalidLoadedAccountsDataSizeLimit,
        34 => TransactionError::ResanitizationNeeded,
        35 => TransactionError::ProgramExecutionTemporarilyRestricted { account_index: aux },
        36 => TransactionError::UnbalancedTransaction,
        37 => TransactionError::ProgramCacheHitMaxLimit,
        38 => TransactionError::CommitCancelled,
        _ => return Err(CatscopeZerohopError::OutofRange),
    };
    Ok(e)
}

fn decode_instruction_error(
    ie_disc: u8,
    custom: u32,
) -> Result<InstructionError, CatscopeZerohopError> {
    let ie = match ie_disc {
        0 => InstructionError::GenericError,
        1 => InstructionError::InvalidArgument,
        2 => InstructionError::InvalidInstructionData,
        3 => InstructionError::InvalidAccountData,
        4 => InstructionError::AccountDataTooSmall,
        5 => InstructionError::InsufficientFunds,
        6 => InstructionError::IncorrectProgramId,
        7 => InstructionError::MissingRequiredSignature,
        8 => InstructionError::AccountAlreadyInitialized,
        9 => InstructionError::UninitializedAccount,
        10 => InstructionError::UnbalancedInstruction,
        11 => InstructionError::ModifiedProgramId,
        12 => InstructionError::ExternalAccountLamportSpend,
        13 => InstructionError::ExternalAccountDataModified,
        14 => InstructionError::ReadonlyLamportChange,
        15 => InstructionError::ReadonlyDataModified,
        16 => InstructionError::DuplicateAccountIndex,
        17 => InstructionError::ExecutableModified,
        18 => InstructionError::RentEpochModified,
        #[allow(deprecated)]
        19 => InstructionError::NotEnoughAccountKeys,
        20 => InstructionError::AccountDataSizeChanged,
        21 => InstructionError::AccountNotExecutable,
        22 => InstructionError::AccountBorrowFailed,
        23 => InstructionError::AccountBorrowOutstanding,
        24 => InstructionError::DuplicateAccountOutOfSync,
        25 => InstructionError::Custom(custom),
        26 => InstructionError::InvalidError,
        27 => InstructionError::ExecutableDataModified,
        28 => InstructionError::ExecutableLamportChange,
        29 => InstructionError::ExecutableAccountNotRentExempt,
        30 => InstructionError::UnsupportedProgramId,
        31 => InstructionError::CallDepth,
        32 => InstructionError::MissingAccount,
        33 => InstructionError::ReentrancyNotAllowed,
        34 => InstructionError::MaxSeedLengthExceeded,
        35 => InstructionError::InvalidSeeds,
        36 => InstructionError::InvalidRealloc,
        37 => InstructionError::ComputationalBudgetExceeded,
        38 => InstructionError::PrivilegeEscalation,
        39 => InstructionError::ProgramEnvironmentSetupFailure,
        40 => InstructionError::ProgramFailedToComplete,
        41 => InstructionError::ProgramFailedToCompile,
        42 => InstructionError::Immutable,
        43 => InstructionError::IncorrectAuthority,
        44 => InstructionError::BorshIoError,
        45 => InstructionError::AccountNotRentExempt,
        46 => InstructionError::InvalidAccountOwner,
        47 => InstructionError::ArithmeticOverflow,
        48 => InstructionError::UnsupportedSysvar,
        49 => InstructionError::IllegalOwner,
        50 => InstructionError::MaxAccountsDataAllocationsExceeded,
        51 => InstructionError::MaxAccountsExceeded,
        52 => InstructionError::MaxInstructionTraceLengthExceeded,
        53 => InstructionError::BuiltinProgramsMustConsumeComputeUnits,
        _ => return Err(CatscopeZerohopError::OutofRange),
    };
    Ok(ie)
}

pub struct StillShotMeta {
    pub last_time: Instant,
    pub duration: Duration,
}

/// Access low latency updates.
pub trait RollingWindow: Send + Sync {
    fn meta(&self) -> &StillShotMeta;
    /// write an account to a byte slice (Header + account body).
    fn account(&self, buffer: &mut [u8]) -> usize;
    /// write a transaction to a byte slice.
    fn transaction(&self, buffer: &mut [u8]) -> usize;
}

mod tests {

    #[allow(unused_imports)]
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
            catx.append_outer(); // inner instructions require an outer instruction
        }
        {
            let mut inner = catx.append_inner(0);
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
