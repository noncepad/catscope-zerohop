use super::proto::{
    CapacityRequest, CapacityStatus,
    capacity_server::{Capacity, CapacityServer},
};
use crate::usage::Capacity as UsageCapacity;
use crate::{
    err::CatscopeZerohopError,
    plugin::{CatscopeReader, CatscopeWriter},
};
use log::{debug, warn};
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::Stream;
use tonic::transport::server::Router;
use tonic::{Request, Response, Status};

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
        warn!("on_status - 1");
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

                warn!("on_status - 2 - msg");
                match msg {
                    crate::usage::Usage::UtilizationRatio(usage) => {
                        // need to inverse to get idle capacity
                        let idle = if usage < 0.0 {
                            1.0
                        } else if 1.0 < usage {
                            0.0
                        } else {
                            1.0 - usage
                        };
                        let cs = CapacityStatus {
                            utilization_ratio: idle,
                        };
                        warn!("on_status - 3 - idle {idle}");
                        match sx2.send(Ok(cs)).await {
                            Ok(_) => {}
                            Err(e) => return Err(CatscopeZerohopError::Unknown(e.to_string())),
                        };
                    }
                };
            }
        });
        tokio::task::spawn(async move {
            warn!("on_status - 4");
            let result: Result<(), CatscopeZerohopError> = match jh.await {
                Ok(x) => x,
                Err(e) => {
                    debug!("join failure: {e}");
                    let _ignore = sx.send(Err(Status::internal("internal failure")));
                    return;
                }
            };
            warn!("on_status - 5");
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
