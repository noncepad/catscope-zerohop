use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use catscope_zerohop::{
    plugin::init_logger,
    quic::{QuicClientConfig, QuicRequestHandler, SolanaQuicClient},
    txfwd::server,
};
use log::error;
use signal_hook::{
    consts::signal::{SIGINT, SIGTERM},
    iterator::Signals,
};
use solana_sdk::{signature::Keypair, signer::EncodableKey};

fn main() {
    init_logger();
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_signal = shutdown.clone();

    let mut signals = Signals::new([SIGINT, SIGTERM]).expect("failed to register signals");
    std::thread::spawn(move || {
        let mut l_sig = signals.wait();
        let sig = l_sig.next().unwrap();
        match sig {
            SIGINT => error!("received SIGINT, shutting down..."),
            SIGTERM => error!("received SIGTERM, shutting down..."),
            _ => {}
        }
        shutdown_signal.store(true, Ordering::Relaxed);
    });

    let rpc_url = std::env::var("RPC_URL").expect("need RPC_URL");
    let keypair = Keypair::read_from_file(std::env::var("KEYPAIR").expect("need KEYPAIR")).unwrap();
    let config = QuicClientConfig {
        connection_timeout: Duration::from_millis(1_000),
        skip_preflight: true,
        connection_pool_size: 10,
    };
    let quic_client = match SolanaQuicClient::new(keypair, Some(config)) {
        Ok(x) => x,
        Err(e) => panic!(
            "quic client configuration failed: rpc url {}; error {}",
            rpc_url, e
        ),
    };

    let handler = match QuicRequestHandler::new(rpc_url.clone(), quic_client) {
        Ok(x) => x,
        Err(e) => panic!(
            "quic request handler failed: rpc url {}; error {}",
            rpc_url, e
        ),
    };
    let p = PathBuf::from(std::env::var("BUFFER_PATH").expect("need BUFFER_PATH"));
    let max_client: usize = match std::env::var("MAX_CLIENT") {
        Ok(x) => x.parse().expect("failed to parse MAX_CLIENT"),
        Err(_) => 6,
    };

    server(p.as_path(), handler, max_client, shutdown).unwrap();
}
