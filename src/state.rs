use crate::read::CatscopeReadChannelGroup;

/// Track subscriptions via `connect()` in order to efficiently
/// filter low latency updates.
pub struct LocalState {
    #[allow(dead_code)]
    connection: Box<dyn CatscopeReadChannelGroup>,
}
