use std::sync::{Arc, Once};

use env_logger::Env;

use crate::{
    err::CatscopeZerohopError,
    read::{GraphClient, PubkeyMap, ViewAccount},
    write::{Scheduler, TransactionProcessor},
};

/// TODO: Document the plugin interface and lifecycle.
///
/// This is the main interface that plugins must implement to integrate with Catscope Zerohop.
///
/// # Example
///
/// ```ignore
/// use catscope_zerohop::plugin::{ZerohopInterface, CatscopeReader, CatscopeWriter};
/// use catscope_zerohop::err::CatscopeZerohopError;
/// use std::sync::Arc;
///
/// struct MyPlugin {
///     // TODO: Add your plugin state
/// }
///
/// impl ZerohopInterface for MyPlugin {
///     fn on_load(
///         &mut self,
///         catscope_reader: Arc<dyn CatscopeReader>,
///         o_catscope_writer: Arc<dyn CatscopeWriter>,
///         configuration: &str,
///     ) -> Result<(), CatscopeZerohopError> {
///         // TODO: Initialize plugin with reader and writer
///         Ok(())
///     }
///
///     fn shutdown(&self) {
///         // TODO: Implement graceful shutdown
///     }
///
///     fn on_unload(&mut self) -> Result<(), CatscopeZerohopError> {
///         // TODO: Cleanup resources
///         Ok(())
///     }
/// }
/// ```
pub trait ZerohopInterface: Send + Sync {
    /// TODO: Document the plugin loading process and initialization.
    ///
    /// Once loaded, the plugin shall receive a reader and writer.
    ///
    /// # Arguments
    ///
    /// * `catscope_reader` - TODO: Document reader capabilities
    /// * `o_catscope_writer` - TODO: Document writer capabilities (optional?)
    ///
    /// # Errors
    ///
    /// TODO: Document when this returns an error and what happens
    fn on_load(
        &mut self,
        catscope_reader: Arc<dyn CatscopeReader>,
        o_catscope_writer: Option<Arc<dyn CatscopeWriter>>,
        configuration_json_data: &str,
    ) -> Result<(), CatscopeZerohopError>;

    /// TODO: Document the unload process and cleanup requirements.
    ///
    /// Called by the host when the validator is about to be unloaded.
    ///
    /// # Errors
    ///
    /// TODO: Document error conditions
    fn unload(&mut self) -> Result<(), CatscopeZerohopError>;
}

/// TODO: Document the CatscopeReader trait and its combined capabilities.
///
/// Combines GraphClient, PubkeyMap, and ViewAccount functionality.
///
/// # Example
///
/// ```ignore
/// use catscope_zerohop::plugin::CatscopeReader;
///
/// fn use_reader(reader: &dyn CatscopeReader) {
///     // TODO: Add example of using reader capabilities
///     // let channels = reader.connect()?;
///     // let pubkey = reader.account_id_exists(&account_id)?;
///     // let account = reader.get(&account_id)?;
/// }
/// ```
pub trait CatscopeReader: GraphClient + PubkeyMap + ViewAccount {}

/// TODO: Document the CatscopeWriter trait and its combined capabilities.
///
/// Combines TransactionProcessor and Scheduler functionality.
///
/// # Example
///
/// ```ignore
/// use catscope_zerohop::plugin::CatscopeWriter;
///
/// fn use_writer(writer: &dyn CatscopeWriter) {
///     // TODO: Add example of using writer capabilities
///     // writer.send(client_id, &transaction_data)?;
///     // writer.batch(client_id, &bundle)?;
/// }
/// ```
pub trait CatscopeWriter: TransactionProcessor + Scheduler {}

static INIT: Once = Once::new();
pub fn init_logger() {
    INIT.call_once(|| {
        env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    });
}
