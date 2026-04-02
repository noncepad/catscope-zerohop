use crate::err::CatscopeZerohopError;
use iceoryx2::prelude::*;
use log::debug;
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Debug, Clone, Copy, ZeroCopySend)]
#[repr(C)]
pub struct SampleRequest {
    pub id: u8,
}

pub type TxFwdErrorCode = u8;

#[derive(Debug, Clone, Copy, ZeroCopySend)]
#[repr(C)]
pub struct SampleResponse {
    pub id: u8,
    pub success: bool,
}

#[derive(Debug, Clone, Copy, ZeroCopySend)]
#[repr(C)]
pub struct SampleResult {
    r: Result<SampleResponse, TxFwdErrorCode>,
}

#[derive(Debug, Clone, Copy, ZeroCopySend)]
#[repr(C)]
pub struct JobResult<S: Clone + Copy + ZeroCopySend> {
    pub r: Result<S, TxFwdErrorCode>,
}

pub trait RequestHandler<T: Sized, S: Sized> {
    fn on_request(&mut self, request: &T) -> Result<&S, CatscopeZerohopError>;
}

/// server listens on a validator as a separate process `catfwd`.
pub fn server<T, S, RH>(
    service_name: &Path,
    mut handler: RH,
    max_client: usize,
    shutdown: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error>>
where
    T: Clone + Copy + ZeroCopySend + std::fmt::Debug,
    S: Clone + Copy + ZeroCopySend + std::fmt::Debug,
    RH: RequestHandler<T, S>,
{
    let mut node_config = Config::default();
    node_config.global.node.cleanup_dead_nodes_on_creation = true;
    let node = NodeBuilder::new()
        .config(&node_config)
        .create::<ipc::Service>()?;

    let sn: ServiceName = ServiceName::new(service_name.to_str().unwrap())?;
    let service = node
        .service_builder(&sn)
        .request_response::<T, JobResult<S>>()
        .max_clients(max_client)
        .max_nodes(256)
        .open_or_create()?;

    let server = service.server_builder().create()?;

    debug!("server - 1");
    while !shutdown.load(Ordering::Relaxed) {
        while let Some(active_request) = server.receive()? {
            //let request_id = active_request.id;
            debug!("server - 2");
            let r: Result<S, TxFwdErrorCode> = match handler.on_request(active_request.payload()) {
                Ok(x) => Ok(*x),
                Err(e) => Err(e.into()),
            };
            debug!("server - 3 - r {:?}", r);
            active_request.send_copy(JobResult { r })?;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    debug!("server - 4");
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::plugin::init_logger;
    use crate::quic::{
        QuicRequestHandler, SolanaQuicClient, TransactionRequest, TransactionResponse,
    };

    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    fn test_service_name(suffix: &str) -> String {
        format!("test_example_{}", suffix)
    }

    fn run_server_for_n_requests(
        service_name: &str,
        request_count: usize,
        stop_signal: Arc<AtomicBool>,
    ) -> Result<Vec<SampleResponse>, Box<dyn std::error::Error + Send + Sync>> {
        let node = NodeBuilder::new().create::<ipc::Service>()?;

        let service = node
            .service_builder(&service_name.try_into()?)
            .request_response::<SampleRequest, SampleResponse>()
            .open_or_create()?;

        let server = service.server_builder().create()?;

        let mut responses_sent = Vec::new();

        while responses_sent.len() < request_count && !stop_signal.load(Ordering::Relaxed) {
            while let Some(active_request) = server.receive()? {
                let request_id = active_request.id;

                let response = SampleResponse {
                    id: request_id,
                    success: true,
                };

                active_request.send_copy(response)?;
                responses_sent.push(response);

                if responses_sent.len() >= request_count {
                    break;
                }
            }

            thread::sleep(Duration::from_millis(1));
        }

        Ok(responses_sent)
    }

    fn run_client_requests(
        service_name: &str,
        request_ids: &[u8],
    ) -> Result<Vec<SampleResponse>, Box<dyn std::error::Error + Send + Sync>> {
        let node = NodeBuilder::new().create::<ipc::Service>()?;

        let service = node
            .service_builder(&service_name.try_into()?)
            .request_response::<SampleRequest, SampleResponse>()
            .open_or_create()?;

        let client = service.client_builder().create()?;

        let mut responses = Vec::new();

        for &id in request_ids {
            let request = SampleRequest { id };
            let pending_response = client.send_copy(request)?;

            let start = std::time::Instant::now();
            loop {
                if let Some(response) = pending_response.receive()? {
                    responses.push(*response);
                    break;
                }
                if start.elapsed() > Duration::from_secs(5) {
                    return Err("Timeout waiting for response".into());
                }
                thread::sleep(Duration::from_millis(1));
            }
        }

        Ok(responses)
    }

    #[test]
    fn test_single_request_response() {
        let service_name = test_service_name("single");
        let stop_signal = Arc::new(AtomicBool::new(false));
        let stop_signal_clone = stop_signal.clone();
        let service_name_clone = service_name.clone();

        let server_handle = thread::spawn(move || {
            run_server_for_n_requests(&service_name_clone, 1, stop_signal_clone)
        });

        // Give server time to start
        thread::sleep(Duration::from_millis(50));

        let client_result = run_client_requests(&service_name, &[42]);

        stop_signal.store(true, Ordering::Relaxed);
        let server_result = server_handle.join().expect("Server thread panicked");

        let responses = client_result.expect("Client failed");
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].id, 42);
        assert!(responses[0].success);

        let server_responses = server_result.expect("Server failed");
        assert_eq!(server_responses.len(), 1);
        assert_eq!(server_responses[0].id, 42);
    }

    #[test]
    fn test_multiple_requests_sequential() {
        let service_name = test_service_name("multi_seq");
        let stop_signal = Arc::new(AtomicBool::new(false));
        let stop_signal_clone = stop_signal.clone();
        let service_name_clone = service_name.clone();

        let request_ids: Vec<u8> = (0..5).collect();
        let request_count = request_ids.len();

        let server_handle = thread::spawn(move || {
            run_server_for_n_requests(&service_name_clone, request_count, stop_signal_clone)
        });

        thread::sleep(Duration::from_millis(50));

        let client_result = run_client_requests(&service_name, &request_ids);

        stop_signal.store(true, Ordering::Relaxed);
        let server_result = server_handle.join().expect("Server thread panicked");

        let responses = client_result.expect("Client failed");
        assert_eq!(responses.len(), request_ids.len());

        for (i, response) in responses.iter().enumerate() {
            assert_eq!(response.id, i as u8);
            assert!(response.success);
        }

        let server_responses = server_result.expect("Server failed");
        assert_eq!(server_responses.len(), request_ids.len());
    }

    #[test]
    fn test_request_response_data_integrity() {
        let service_name = test_service_name("integrity");
        let stop_signal = Arc::new(AtomicBool::new(false));
        let stop_signal_clone = stop_signal.clone();
        let service_name_clone = service_name.clone();

        // Test with various ID values including edge cases
        let request_ids: Vec<u8> = vec![0, 1, 127, 128, 254, 255];
        let request_count = request_ids.len();

        let server_handle = thread::spawn(move || {
            run_server_for_n_requests(&service_name_clone, request_count, stop_signal_clone)
        });

        thread::sleep(Duration::from_millis(50));

        let client_result = run_client_requests(&service_name, &request_ids);

        stop_signal.store(true, Ordering::Relaxed);
        let _ = server_handle.join().expect("Server thread panicked");

        let responses = client_result.expect("Client failed");
        assert_eq!(responses.len(), request_ids.len());

        // Verify each response matches its request ID
        for (expected_id, response) in request_ids.iter().zip(responses.iter()) {
            assert_eq!(response.id, *expected_id, "Response ID mismatch");
            assert!(response.success, "Response should indicate success");
        }
    }

    #[test]
    fn test_concurrent_clients() {
        let service_name = test_service_name("concurrent");
        let stop_signal = Arc::new(AtomicBool::new(false));
        let stop_signal_clone = stop_signal.clone();
        let service_name_clone = service_name.clone();

        let total_requests = 6; // 2 clients * 3 requests each

        let server_handle = thread::spawn(move || {
            run_server_for_n_requests(&service_name_clone, total_requests, stop_signal_clone)
        });

        thread::sleep(Duration::from_millis(50));

        let service_name_1 = service_name.clone();
        let service_name_2 = service_name.clone();

        let client1_handle =
            thread::spawn(move || run_client_requests(&service_name_1, &[10, 11, 12]));

        let client2_handle =
            thread::spawn(move || run_client_requests(&service_name_2, &[20, 21, 22]));

        let client1_result = client1_handle.join().expect("Client 1 panicked");
        let client2_result = client2_handle.join().expect("Client 2 panicked");

        stop_signal.store(true, Ordering::Relaxed);
        let _ = server_handle.join().expect("Server thread panicked");

        let responses1 = client1_result.expect("Client 1 failed");
        let responses2 = client2_result.expect("Client 2 failed");

        assert_eq!(responses1.len(), 3);
        assert_eq!(responses2.len(), 3);

        // Verify client 1 got its responses
        for (i, response) in responses1.iter().enumerate() {
            assert_eq!(response.id, 10 + i as u8);
            assert!(response.success);
        }

        // Verify client 2 got its responses
        for (i, response) in responses2.iter().enumerate() {
            assert_eq!(response.id, 20 + i as u8);
            assert!(response.success);
        }
    }

    /// Test that shared memory can be properly torn down and reconstituted without corruption.
    /// This verifies that iceoryx2 correctly cleans up files in /dev/shm and /tmp,
    /// preventing Service state corruption when services are recreated.
    #[test]
    fn test_teardown_and_reconstitute_shared_memory() {
        let service_name = test_service_name("teardown_reconstitute");

        // Number of teardown/reconstitute cycles to perform
        const CYCLES: usize = 3;
        // Number of requests per cycle
        const REQUESTS_PER_CYCLE: usize = 3;

        for cycle in 0..CYCLES {
            let stop_signal = Arc::new(AtomicBool::new(false));
            let stop_signal_clone = stop_signal.clone();
            let service_name_clone = service_name.clone();

            // Create server in its own scope so it gets dropped completely
            let server_handle = thread::spawn(move || {
                run_server_for_n_requests(
                    &service_name_clone,
                    REQUESTS_PER_CYCLE,
                    stop_signal_clone,
                )
            });

            // Give server time to start and create shared memory resources
            thread::sleep(Duration::from_millis(50));

            // Create client and send requests
            let request_ids: Vec<u8> = (0..REQUESTS_PER_CYCLE)
                .map(|i| (cycle * REQUESTS_PER_CYCLE + i) as u8)
                .collect();

            let client_result = run_client_requests(&service_name, &request_ids);

            // Signal server to stop and wait for it
            stop_signal.store(true, Ordering::Relaxed);
            let server_result = server_handle.join().expect("Server thread panicked");

            // Verify this cycle worked correctly
            let responses = client_result.expect(&format!("Client failed on cycle {}", cycle));
            assert_eq!(
                responses.len(),
                REQUESTS_PER_CYCLE,
                "Wrong number of responses on cycle {}",
                cycle
            );

            for (i, response) in responses.iter().enumerate() {
                let expected_id = (cycle * REQUESTS_PER_CYCLE + i) as u8;
                assert_eq!(
                    response.id, expected_id,
                    "Response ID mismatch on cycle {}, request {}",
                    cycle, i
                );
                assert!(
                    response.success,
                    "Response should indicate success on cycle {}, request {}",
                    cycle, i
                );
            }

            let server_responses =
                server_result.expect(&format!("Server failed on cycle {}", cycle));
            assert_eq!(
                server_responses.len(),
                REQUESTS_PER_CYCLE,
                "Server processed wrong number of requests on cycle {}",
                cycle
            );

            // Allow time for cleanup before next cycle
            // This is where corruption would manifest if shared memory isn't properly cleaned up
            thread::sleep(Duration::from_millis(100));
        }

        // Final verification: create one more service instance and verify it works
        // This catches any accumulated corruption from the previous cycles
        let stop_signal = Arc::new(AtomicBool::new(false));
        let stop_signal_clone = stop_signal.clone();
        let service_name_clone = service_name.clone();

        let server_handle = thread::spawn(move || {
            run_server_for_n_requests(&service_name_clone, 1, stop_signal_clone)
        });

        thread::sleep(Duration::from_millis(50));

        let final_result = run_client_requests(&service_name, &[255]);

        stop_signal.store(true, Ordering::Relaxed);
        let _ = server_handle.join().expect("Final server thread panicked");

        let final_responses =
            final_result.expect("Final client request failed after multiple teardown cycles");
        assert_eq!(final_responses.len(), 1, "Final request should succeed");
        assert_eq!(final_responses[0].id, 255, "Final response ID should match");
        assert!(
            final_responses[0].success,
            "Final response should indicate success"
        );
    }
}
