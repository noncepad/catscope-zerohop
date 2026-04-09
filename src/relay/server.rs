use super::proto::{
    ApiDescription, CapacityRequest, CapacityStatus, EndpointRequest, EndpointResponse,
    InformationRequest, StatusRequest, Usage,
    capacity_server::{Capacity, CapacityServer},
    endpoint_server::{Endpoint, EndpointServer},
    information_server::{Information, InformationServer},
    meter_server::{Meter, MeterServer},
};
use crate::read::GraphClient;
use crate::usage::Capacity as UsageCapacity;
use crate::write::TransactionProcessor;
use crate::{
    err::CatscopeZerohopError,
    plugin::{CatscopeReader, CatscopeWriter},
};
use log::debug;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tokio_stream::Stream;
use tonic::transport::server::Router;
use tonic::{Request, Response, Status, transport::Server};

type BoxStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send + 'static>>;

// ── Per-service wrapper structs ───────────────────────────────────────────────

#[derive(Clone)]
pub struct CapacityImpl {
    us: Arc<dyn UsageCapacity>,
}
impl CapacityImpl {
    pub async fn from_write(writer: &Arc<dyn CatscopeWriter>) -> Self {
        Self::new(writer.clone()).await
    }
    pub async fn from_read(reader: &Arc<dyn CatscopeReader>) -> Self {
        Self::new(reader.clone()).await
    }
    pub async fn new(us: Arc<dyn UsageCapacity>) -> Self {
        Self { us }
    }
}

// ── Capacity ──────────────────────────────────────────────────────────────────

#[tonic::async_trait]
impl Capacity for CapacityImpl {
    type OnStatusStream = BoxStream<CapacityStatus>;

    async fn on_status(
        &self,
        _req: Request<CapacityRequest>,
    ) -> Result<Response<Self::OnStatusStream>, Status> {
        let (sx, rx) = tokio::sync::mpsc::channel(64);
        let (usage_sx, usage_rx) = flume::bounded(64);
        self.us.on_status(usage_sx);
        let sx2 = sx.clone();
        let jh = tokio::task::spawn(async move {
            loop {
                let msg = match usage_rx.recv_async().await {
                    Ok(x) => x,
                    Err(e) => {
                        debug!("OnStatusStream recv failed: {e}");
                        return Err(CatscopeZerohopError::OutofRange);
                    }
                };

                match msg {
                    crate::usage::Usage::UtilizationRatio(ratio) => {
                        let cs = CapacityStatus {
                            utilization_ratio: ratio,
                        };
                        match sx2.send(Ok(cs)).await {
                            Ok(_) => {}
                            Err(e) => return Err(CatscopeZerohopError::Unknown(e.to_string())),
                        };
                    }
                };
            }
        });
        tokio::task::spawn(async move {
            let result: Result<(), CatscopeZerohopError> = match jh.await {
                Ok(x) => x,
                Err(e) => {
                    debug!("join failure: {e}");
                    let _ignore = sx.send(Err(Status::internal("internal failure")));
                    return;
                }
            };
            match result {
                Ok(_) => {}
                Err(e) => {
                    debug!("join failure: {e}");
                    let _ignore = sx.send(Err(Status::internal("internal failure")));
                }
            };
        });
        Ok(Response::new(Box::pin(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        )))
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Serve all relay.proto services on `addr` using `handler` as the backend.
///
/// This is an async function intended to be awaited inside a Tokio runtime.
pub async fn capacity_serve(capacity: CapacityImpl, router: Router) -> Router {
    router.add_service(CapacityServer::new(capacity.clone()))
}
