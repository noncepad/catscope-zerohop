use super::proto::{
    ApiDescription, CapacityRequest, CapacityStatus, EndpointRequest, EndpointResponse,
    InformationRequest, StatusRequest, Usage,
};
use tonic::Status;

/// Implement this trait to provide business logic for all relay.proto services.
///
/// Unary RPCs return a `Result` directly. Server-streaming RPCs receive a
/// `flume::Sender`; send `Ok(item)` for each response item or `Err(status)`
/// to terminate the stream with an error. Drop the sender (or stop sending)
/// to close the stream normally.
///
/// None of the methods are async. `flume` channels are usable from both sync
/// and async contexts, so you can drive the sender from a background thread,
/// a sync loop, or wherever is convenient.
pub trait RelayHandler: Send + Sync + 'static {
    // ── Capacity service ──────────────────────────────────────────────────

    fn capacity_on_status(
        &self,
        req: CapacityRequest,
        tx: flume::Sender<Result<CapacityStatus, Status>>,
    );

    // ── Meter service ─────────────────────────────────────────────────────

    fn meter_get_status(&self, req: StatusRequest) -> Result<Usage, Status>;

    fn meter_on_status(&self, req: StatusRequest, tx: flume::Sender<Result<Usage, Status>>);

    // ── Endpoint service ──────────────────────────────────────────────────

    fn endpoint_get_clear_net_address(
        &self,
        req: EndpointRequest,
    ) -> Result<EndpointResponse, Status>;

    // ── Information service ───────────────────────────────────────────────

    fn information_get(&self, req: InformationRequest) -> Result<ApiDescription, Status>;
}
