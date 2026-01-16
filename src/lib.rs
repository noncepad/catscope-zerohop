/// Catscope Zerohop plugin runtime.
///
/// This crate defines the interfaces and runtime components used by
/// ZeroHop plugins to interact with CatScope at the validator level.
///
/// It exposes read and write APIs, shared state handling, and internal
/// utilities required for low-latency plugin execution.
///
pub mod buffer;
pub mod config;
pub mod err;
pub mod plugin;
pub mod queue;
pub mod quic;
pub mod read;
pub mod store;
pub mod txfwd;
pub(crate) mod util;
pub mod write;
