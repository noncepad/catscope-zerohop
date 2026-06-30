use crate::store::{AccountHeader, WEIGHT_IS_OUTGOING, Weight};
pub use solana_sdk::pubkey::Pubkey as SolanaPubkey;
use solana_sdk::{clock::Slot, pubkey::Pubkey};

/// An edge in the account relationship graph, keyed by public key.
///
/// `#[repr(C, align(8))]` so the layout is stable across cdylib plugin boundaries.
#[repr(C, align(8))]
#[derive(Debug, Clone, Default)]
pub struct FilterEdge {
    pub slot: Slot,
    pub to: Pubkey,
    pub from: Pubkey,
    pub weight: Weight,
}

impl FilterEdge {
    pub fn set_outgoing(&mut self, id: &Pubkey) {
        if self.from.eq(id) {
            self.weight |= WEIGHT_IS_OUTGOING;
        }
    }
}

/// Native Rust edge generator — replaces WasmEdgeGeneratorFilter.
///
/// Implement this trait in a `cdylib` crate and export `create_edge_generator()`
/// (see below).  The host loads the shared library with `libloading` and calls
/// that symbol to obtain a `Box<dyn NativeEdgeGenerator>`.
///
/// # ABI contract
/// Both the plugin and the host **must** be compiled with the same Rust toolchain
/// version so that the vtable layout of `dyn NativeEdgeGenerator` matches.
pub trait NativeEdgeGenerator: Send + Sync {
    /// All Solana program IDs that this generator cares about.
    ///
    /// The host uses this list to decide which accounts to route through
    /// `edge()` — only accounts whose `owner` is in this list are passed.
    fn program_id_list(&self) -> &[Pubkey];

    /// Compute outgoing/incoming edges for one account.
    ///
    /// * `header` – account metadata (pubkey, owner, lamports, slot, …)
    /// * `data`   – raw account data bytes (may be empty)
    ///
    /// Returns a `Vec<FilterEdge>`.  Edges with `from == to` are silently
    /// dropped by the host.
    fn edge(&self, header: &AccountHeader, data: &[u8], filter_edge: &mut Vec<FilterEdge>);
}

/// Symbol name that the plugin cdylib must export.
pub const CREATE_EDGE_GENERATOR_SYMBOL: &[u8] = b"create_edge_generator\0";

/// Signature of the constructor symbol exported by plugin cdylibs.
///
/// ```ignore
/// // In your plugin crate:
/// #[no_mangle]
/// pub extern "C" fn create_edge_generator() -> *mut dyn NativeEdgeGenerator {
///     Box::into_raw(Box::new(MyGenerator::new()))
/// }
/// ```
#[allow(improper_ctypes_definitions)]
pub type CreateEdgeGeneratorFn = unsafe extern "C" fn() -> *mut dyn NativeEdgeGenerator;
