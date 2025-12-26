use std::{
    collections::HashSet,
    marker::PhantomData,
    mem::MaybeUninit,
    sync::{Arc, atomic::AtomicBool},
};

use solana_sdk::{
    clock::Slot, pubkey::Pubkey, signature::Signature, transaction::TransactionError,
};

use crate::{err::CatscopeZerohopError, util::bytes_to_struct};

/// TODO: Document what slot status values mean.
///
/// # Fields
///
/// * `slot` - TODO: Document the slot number
/// * `status` - TODO: Document status codes (e.g., 0 = pending, 1 = confirmed, 2 = finalized, etc.)
#[derive(Clone, Debug)]
pub struct SlotWithStatus {
    /// TODO: Document the slot number
    pub slot: Slot,
    /// TODO: Document status code meanings
    pub status: SlotStatusU8,
}
pub type SlotStatusU8 = u8;
pub trait BlobInterface: BlobView + BlobWrite {}

/// TODO: Document when to use StructBlob and its lifecycle.
///
/// Store a struct backed by a blob.
///
/// # Type Parameters
///
/// * `T` - TODO: Document constraints on T (alignment, size, etc.)
///
/// # Example
///
/// ```ignore
/// use catscope_zerohop::store::NamedBlob;
///
/// // TODO: Add example of creating and using StructBlob
/// // let blob: StructBlob<MyStruct> = ...;
/// // let payload = blob.payload_mut()?;
/// ```
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
    /// TODO: Document when this returns None vs Some and mutable access rules.
    ///
    /// # Returns
    ///
    /// TODO: Document None/Some conditions
    #[allow(clippy::mut_from_ref)]
    pub fn payload_mut(&self) -> Option<&mut MaybeUninit<T>> {
        let slice = self.blob.slice_mut()?;
        assert_eq!(slice.len(), std::mem::size_of::<MaybeUninit<T>>());
        let ptr = slice.as_mut_ptr() as *mut _;
        let ans: &mut MaybeUninit<T> = unsafe { &mut *ptr };
        Some(ans)
    }

    /// TODO: Document when to use this for reading vector payloads.
    ///
    /// # Returns
    ///
    /// TODO: Document the slice returned
    pub fn vec_payload(&self) -> &[MaybeUninit<T>] {
        let slice = self.blob.slice();
        let n = std::mem::size_of::<MaybeUninit<T>>();
        assert_eq!(slice.len() % n, 0);
        let count = slice.len() / n;
        let ptr = slice.as_ptr() as *const _;
        unsafe { std::slice::from_raw_parts(ptr, count) }
    }

    /// TODO: Document mutable vector access and when this returns None.
    ///
    /// # Returns
    ///
    /// TODO: Document None/Some conditions
    #[allow(clippy::mut_from_ref)]
    pub fn vec_payload_mut(&self) -> Option<&mut [MaybeUninit<T>]> {
        let slice = self.blob.slice_mut()?;
        let n = std::mem::size_of::<MaybeUninit<T>>();
        assert_eq!(slice.len() % n, 0);
        let count = slice.len() / n;
        let ptr = slice.as_mut_ptr() as *mut _;
        Some(unsafe { std::slice::from_raw_parts_mut(ptr, count) })
    }
}

/// TODO: Document when to implement this trait and write access patterns.
#[allow(clippy::mut_from_ref)]
pub trait BlobWrite: Send + Sync {
    /// TODO: Document when this returns None and thread safety.
    ///
    /// # Returns
    ///
    /// TODO: Document None/Some conditions for mutable slice access
    fn slice_mut(&self) -> Option<&mut [u8]>;
}

/// TODO: Document when to implement this trait for read-only blob access.
pub trait BlobView: Send + Sync {
    /// TODO: Document the blob length in bytes
    fn len(&self) -> usize;

    /// TODO: Document empty check behavior
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// TODO: Document the slice lifetime and thread safety
    fn slice(&self) -> &[u8];
}

/// TODO: Document the Solana account structure and its components.
///
/// # Example
///
/// ```ignore
/// use catscope_zerohop::store::SolanaAccount;
///
/// fn process_account(account: &SolanaAccount) {
///     // TODO: Add example of accessing account data
///     // let header = account.header();
///     // let data = account.data();
///     // let edges = account.edge();
/// }
/// ```
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
    /// TODO: Document what the ticket represents and its uses.
    pub fn ticket(&self) -> Ticket {
        self.ticket
    }

    /// TODO: Document the account header structure.
    ///
    /// # Returns
    ///
    /// TODO: Document header contents and layout
    pub fn header(&self) -> &AccountHeader {
        let slice = self.blob.slice();
        let ah_len = std::mem::size_of::<AccountHeader>();
        assert!(slice.len() <= ah_len);
        bytes_to_struct(&slice[0..ah_len])
    }

    /// TODO: Document when account data is present vs absent.
    ///
    /// # Returns
    ///
    /// TODO: Document None (no data) vs Some (account data bytes)
    pub fn data(&self) -> Option<&[u8]> {
        let slice = self.blob.slice();
        let ah_len = std::mem::size_of::<AccountHeader>();
        if slice.len() == ah_len {
            None
        } else {
            Some(&slice[ah_len..])
        }
    }

    /// TODO: Document the edge structure and graph relationships.
    ///
    /// # Returns
    ///
    /// TODO: Document when edges exist and what they represent
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

/// TODO: Document what a ticket represents in the system
pub type Ticket = u64;

/// TODO: Document the account ID system and uniqueness guarantees
pub type AccountId = u64;

/// TODO: Document the account header layout and memory representation.
///
/// Use this to store accounts.
///
/// # Layout
///
/// TODO: Document the C layout requirements and alignment
///
/// # Fields
///
/// * `pubkey` - TODO: Document the account's public key
/// * `lamports` - Lamports in the account (TODO: add more details)
/// * `account_id` - TODO: Document the internal account ID
/// * `owner` - The program that owns this account. If executable, the program that loads this account. (TODO: add more details)
/// * `rent_epoch` - The epoch at which this account will next owe rent (TODO: add more details)
/// * `slot` - TODO: Document which slot this account state is from
/// * `data_size` - TODO: Document the account data size
/// * `executable` - This account's data contains a loaded program (and is now read-only) (TODO: add more details)
#[repr(C, align(8))]
#[derive(Default, Copy, Debug, Clone, PartialEq, Eq)]
pub struct AccountHeader {
    /// TODO: Document the account's public key
    pub pubkey: Pubkey,
    /// lamports in the account
    pub lamports: u64,
    /// TODO: Document the account ID
    pub account_id: AccountId,
    /// the program that owns this account. If executable, the program that loads this account.
    pub owner: Pubkey,
    /// the epoch at which this account will next owe rent
    pub rent_epoch: u64,
    /// TODO: Document the slot number
    pub slot: u64,
    /// this account's data contains a loaded program (and is now read-only)
    pub data_size: u32,
    /// TODO: Document executable flag meaning
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

/// TODO: Document the weight system and filtering
pub type Weight = u32;

/// TODO: Document depth in graph traversal
pub type Depth = u8;

/// TODO: Document the account edge structure in the graph.
///
/// # Layout
///
/// TODO: Document C layout requirements
///
/// # Fields
///
/// * `from` - TODO: Document source account
/// * `to` - TODO: Document destination account
/// * `weight` - TODO: Document edge weight meaning
/// * `slot` - TODO: Document when this edge was created
#[repr(C, align(8))]
#[derive(Debug, Clone, Eq)]
pub struct AccountEdge {
    /// TODO: Document the source account
    pub from: AccountId,
    /// TODO: Document the destination account
    pub to: AccountId,
    /// TODO: Document the edge weight
    pub weight: u32,
    /// TODO: Document the slot
    pub slot: Slot,
}
impl PartialEq for AccountEdge {
    fn eq(&self, other: &Self) -> bool {
        self.from == other.from && self.to == other.to && self.weight == other.weight
    }
}
impl PartialOrd for AccountEdge {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for AccountEdge {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.from.cmp(&other.from) {
            core::cmp::Ordering::Equal => {}
            ord => return ord,
        }
        match self.to.cmp(&other.to) {
            core::cmp::Ordering::Equal => {}
            ord => return ord,
        }
        match self.weight.cmp(&other.weight) {
            core::cmp::Ordering::Equal => {}
            ord => return ord,
        }
        core::cmp::Ordering::Equal
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

/// TODO: Document the ViewAccount trait and when to implement it.
///
/// # Example
///
/// ```ignore
/// use catscope_zerohop::store::{ViewAccount, AccountId, SolanaAccount};
///
/// struct MyAccountStore {
///     // TODO: Add your storage
/// }
///
/// impl ViewAccount for MyAccountStore {
///     fn load_account(&self, account_id: &AccountId) -> Option<SolanaAccount> {
///         // TODO: Implement account loading
///         None
///     }
/// }
/// ```
pub trait ViewAccount {
    /// TODO: Document account loading behavior and when this returns None.
    ///
    /// # Returns
    ///
    /// TODO: Document None/Some conditions
    fn load_account(&self, account_id: &AccountId) -> Option<SolanaAccount>;
}

/// TODO: Document the transaction result header structure.
///
/// # Layout
///
/// TODO: Document C layout requirements
#[repr(C, align(8))]
pub struct TransactionResultHeader {
    /// TODO: Document the transaction signature
    pub signature: Signature,
    /// TODO: Document the slot where transaction was processed
    pub slot: Slot,
    /// TODO: Document what count represents
    count: u32,
    /// TODO: Document success/failure flag
    pub success: bool,
}

/// TODO: Document the transaction result structure.
///
/// # Fields
///
/// * `transaction` - TODO: Document the transaction data
/// * `slot` - TODO: Document the processing slot
/// * `result` - TODO: Document success vs error result
pub struct TransactionResult {
    /// TODO: Document the transaction
    pub transaction: CatscopeTransaction,
    /// TODO: Document the slot
    pub slot: Slot,
    /// TODO: Document the result
    pub result: Result<(), TransactionError>,
}

/// TODO: Document the Catscope transaction structure.
///
/// # Fields
///
/// * `signature` - TODO: Document the transaction signature
/// * `index` - TODO: Document what the index represents
/// * `outer` - TODO: Document outer instructions (max 30)
/// * `inner` - TODO: Document inner instructions (max 30)
///
/// # Example
///
/// ```ignore
/// use catscope_zerohop::store::CatscopeTransaction;
///
/// fn process_tx(tx: &CatscopeTransaction) {
///     // TODO: Add example of processing transaction
///     // for instruction in tx.outer() {
///     //     // Process outer instruction
///     // }
///     // for instruction in tx.inner() {
///     //     // Process inner instruction
///     // }
/// }
/// ```
#[repr(C, align(64))]
pub struct CatscopeTransaction {
    /// TODO: Document the transaction signature
    pub signature: Signature,
    /// TODO: Document the transaction index
    pub index: u64,
    outer_len: u16,
    outer: [CatscopeInstruction; 30],
    l1_inner_len: u8,
    l1_inner: [u8; 30],
    inner_len: u16,
    inner: [CatscopeInstruction; 60],
    account_len: u16,
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

    pub fn set_outer(&mut self, size: usize) -> &mut [CatscopeInstruction] {
        self.outer_len = size as u16;
        &mut self.outer[0..size]
    }
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

    pub fn account(&self) -> &[AccountId] {
        &self.account[0..(self.account_len as usize)]
    }

    /// TODO: Document outer instructions and their purpose.
    ///
    /// # Returns
    ///
    /// TODO: Document the slice of outer instructions
    pub fn outer(&self) -> &[CatscopeInstruction] {
        &self.outer[0..(self.outer_len as usize)]
    }

    /// TODO: Document inner instructions and their purpose.
    ///
    /// # Returns
    ///
    /// TODO: Document the slice of inner instructions
    pub fn inner(&self) -> &[CatscopeInstruction] {
        &self.inner[0..(self.inner_len as usize)]
    }
}
impl std::fmt::Debug for CatscopeTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatscopeTransaction").finish()
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
///
/// # Example
///
/// ```ignore
/// use catscope_zerohop::store::CatscopeInstruction;
///
/// fn process_instruction(instruction: &CatscopeInstruction) {
///     // TODO: Add example
///     // let program_id = instruction.program();
///     // let data = instruction.data();
///     // let accounts = instruction.account();
/// }
/// ```
#[derive(Clone, Copy)]
pub struct CatscopeInstruction {
    program: AccountId,
    data_len: u16,
    data: [u8; MAX_DATA],
    account_len: u16,
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
    #[inline]
    pub fn set_program(&mut self, program_id: AccountId) {
        self.program = program_id;
    }
    #[inline]
    pub fn set_account(&mut self, size: usize) -> &mut [AccountId] {
        assert!(self.account.len() <= size);
        self.account_len = size as u16;
        &mut self.account[0..size]
    }
    #[inline]
    pub fn set_data(&mut self, data: &[u8]) {
        assert!(data.len() <= self.data.len());
        self.data_len = data.len() as u16;
        let subbuf = &mut self.data[0..data.len()];
        subbuf.copy_from_slice(data);
    }
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

    /// TODO: Document the program account.
    ///
    /// # Returns
    ///
    /// TODO: Document the program account ID
    #[inline]
    pub fn program(&self) -> AccountId {
        self.program
    }

    /// TODO: Document the instruction data.
    ///
    /// # Returns
    ///
    /// TODO: Document the data slice (up to 2KB)
    #[inline]
    pub fn data(&self) -> &[u8] {
        &self.data[0..(self.data_len as usize)]
    }

    /// TODO: Document the account list.
    ///
    /// # Returns
    ///
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
}

const MAX_DATA: usize = 2 * 1024;
const MAX_ACCOUNT: usize = 256;

mod tests {
    use crate::store::CatscopeTransaction;

    #[test]
    fn test_tx() {
        let _tx = CatscopeTransaction::default();
        assert_eq!(std::mem::size_of::<CatscopeTransaction>(), 0);
    }
}
