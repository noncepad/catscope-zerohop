use std::sync::{Arc, Once};

use env_logger::Env;

use crate::{
    err::CatscopeZerohopError,
    read::{GraphClient, PubkeyMap, ViewAccount},
    write::{Scheduler, TransactionProcessor},
};

/// ======================================================================
/// ZEROHOP PLUGIN INTERFACE
/// ======================================================================
///
/// This file defines the boundary between the ZeroHop runtime and user plugins.
///
/// A plugin is loaded by the ZeroHop host, given read/write handles into the
/// validator-local runtime, and expected to manage its own lifecycle.

/// Main interface that every ZeroHop plugin must implement.
///
/// The host owns plugin creation and destruction. Plugins are notified
/// synchronously when they are loaded and unloaded.
pub trait ZerohopInterface: Send + Sync {
    /// Called once when the plugin is loaded by the ZeroHop host.
    ///
    /// This is where the plugin should:
    /// - store the reader/writer handles
    /// - parse configuration
    /// - start any background tasks
    ///
    /// `catscope_reader` provides read-only access to accounts and graphs.
    /// `o_catscope_writer` provides optional transaction submission capability.
    fn on_load(
        &mut self,
        catscope_reader: Arc<dyn CatscopeReader>,
        o_catscope_writer: Option<Arc<dyn CatscopeWriter>>,
        configuration_json_data: &str,
    ) -> Result<(), CatscopeZerohopError>;

    /// Called when the plugin is being unloaded.
    ///
    /// The plugin should stop background work and release resources.
    fn unload(&mut self) -> Result<(), CatscopeZerohopError>;
}

/// ======================================================================
/// READER INTERFACE
/// ======================================================================
///
/// Read-only view into validator-local state.
///
/// This is the primary way plugins observe:
/// - account state
/// - graph relationships
/// - pubkey ↔ account-id mappings
pub trait CatscopeReader: GraphClient + PubkeyMap + ViewAccount {}

/// ======================================================================
/// WRITER INTERFACE
/// ======================================================================
///
/// Write-side interface for submitting transactions.
///
pub trait CatscopeWriter: TransactionProcessor + Scheduler {}

/// ======================================================================
/// LOGGER INITIALIZATION
/// ======================================================================
///
/// Initializes a shared logger for plugins and runtime.
///
/// Safe to call multiple times.
static INIT: Once = Once::new();

pub fn init_logger() {
    INIT.call_once(|| {
        env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    });
}
