use crate::read::CatscopeReadChannelGroup;

/// Track subscriptions via `connect()` in order to efficiently
/// filter low latency updates.
pub struct LocalState {
    connection: Box<dyn CatscopeReadChannelGroup>,
}
