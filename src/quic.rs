//! Solana QUIC client for sending transactions to leader validators.
//!
//! This module provides a QUIC-based client for submitting transactions
//! directly to Solana validator TPU (Transaction Processing Unit) endpoints.
//! The client uses a Solana keypair for QUIC TLS authentication, which is
//! required by Solana's staked connection protocol.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use iceoryx2::prelude::ZeroCopySend;
use log::{debug, error, warn};
use solana_client::rpc_client::RpcClient;
use solana_connection_cache::client_connection::ClientConnection;
use solana_connection_cache::connection_cache::NewConnectionConfig;
use solana_quic_client::{QuicConfig, QuicConnectionCache, QuicConnectionManager};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::transaction::VersionedTransaction;
use solana_streamer::streamer::StakedNodes;

use crate::err::CatscopeZerohopError;
use crate::txfwd::RequestHandler;

/// Configuration for the QUIC client.
#[derive(Debug, Clone)]
pub struct QuicClientConfig {
    /// Connection timeout duration.
    pub connection_timeout: Duration,
    /// Skip transaction preflight checks when sending.
    pub skip_preflight: bool,
    /// Number of connections in the pool.
    pub connection_pool_size: usize,
}

impl Default for QuicClientConfig {
    fn default() -> Self {
        Self {
            connection_timeout: Duration::from_secs(10),
            skip_preflight: true,
            connection_pool_size: 4,
        }
    }
}

/// A QUIC client for sending transactions to Solana validators.
///
/// This client connects to validator TPU endpoints using QUIC protocol
/// with TLS authentication via the provided keypair.
pub struct SolanaQuicClient {
    /// The keypair used for QUIC TLS authentication (staking identity).
    keypair: Arc<Keypair>,
    /// Client configuration.
    config: QuicClientConfig,
    /// QUIC connection cache.
    connection_cache: QuicConnectionCache,
}

impl SolanaQuicClient {
    /// Creates a new QUIC client with the given keypair.
    ///
    /// The keypair is used for QUIC TLS authentication. In Solana's staked
    /// connection protocol, validators prioritize connections from staked
    /// identities.
    ///
    /// # Arguments
    /// * `keypair` - The Solana keypair for authentication
    /// * `config` - Optional configuration (uses defaults if None)
    pub fn new(
        keypair: Keypair,
        config: Option<QuicClientConfig>,
    ) -> Result<Self, CatscopeZerohopError> {
        let config = config.unwrap_or_default();
        let keypair = Arc::new(keypair);

        // Create QUIC configuration with the keypair's certificate
        let mut quic_config = QuicConfig::new().map_err(|e| {
            CatscopeZerohopError::ConfigError(format!("Failed to create QUIC config: {e}"))
        })?;

        // Update the certificate with the provided keypair
        quic_config.update_client_certificate(&keypair, IpAddr::V4(Ipv4Addr::UNSPECIFIED));

        // Create connection manager and cache
        let connection_manager = QuicConnectionManager::new_with_connection_config(quic_config);
        let connection_cache = solana_connection_cache::connection_cache::ConnectionCache::new(
            "catscope-quic-client",
            connection_manager,
            config.connection_pool_size,
        )
        .map_err(|e| {
            CatscopeZerohopError::ConfigError(format!("Failed to create connection cache: {e}"))
        })?;

        Ok(Self {
            keypair,
            config,
            connection_cache,
        })
    }

    /// Creates a new QUIC client with staked node information.
    ///
    /// This allows the client to be recognized by validators that prioritize
    /// staked connections.
    ///
    /// # Arguments
    /// * `keypair` - The Solana keypair for authentication
    /// * `staked_nodes` - Staked node information for priority connections
    /// * `config` - Optional configuration (uses defaults if None)
    pub fn new_with_staked_nodes(
        keypair: Keypair,
        staked_nodes: Arc<RwLock<StakedNodes>>,
        config: Option<QuicClientConfig>,
    ) -> Result<Self, CatscopeZerohopError> {
        let config = config.unwrap_or_default();
        let keypair = Arc::new(keypair);

        // Create QUIC configuration with staked nodes
        let mut quic_config = QuicConfig::new().map_err(|e| {
            CatscopeZerohopError::ConfigError(format!("Failed to create QUIC config: {e}"))
        })?;

        quic_config.update_client_certificate(&keypair, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        quic_config.set_staked_nodes(&staked_nodes, &keypair.pubkey());

        let connection_manager = QuicConnectionManager::new_with_connection_config(quic_config);
        let connection_cache = solana_connection_cache::connection_cache::ConnectionCache::new(
            "catscope-quic-client-staked",
            connection_manager,
            config.connection_pool_size,
        )
        .map_err(|e| {
            CatscopeZerohopError::ConfigError(format!("Failed to create connection cache: {e}"))
        })?;

        Ok(Self {
            keypair,
            config,
            connection_cache,
        })
    }

    /// Creates a new QUIC client from a keypair file path.
    ///
    /// # Arguments
    /// * `keypair_path` - Path to the keypair JSON file
    /// * `config` - Optional configuration
    pub fn from_keypair_file(
        keypair_path: &str,
        config: Option<QuicClientConfig>,
    ) -> Result<Self, CatscopeZerohopError> {
        let keypair = solana_sdk::signature::read_keypair_file(keypair_path).map_err(|e| {
            CatscopeZerohopError::ConfigError(format!("Failed to read keypair: {e}"))
        })?;
        Self::new(keypair, config)
    }

    /// Returns a reference to the client's keypair.
    pub fn keypair(&self) -> &Keypair {
        &self.keypair
    }

    /// Returns a reference to the client's configuration.
    pub fn config(&self) -> &QuicClientConfig {
        &self.config
    }

    /// Returns the public key of the client's keypair.
    pub fn pubkey(&self) -> solana_sdk::pubkey::Pubkey {
        self.keypair.pubkey()
    }

    /// Sends a single transaction to the specified TPU address via QUIC.
    ///
    /// # Arguments
    /// * `tpu_addr` - The TPU QUIC address of the leader validator
    /// * `transaction` - The transaction to send
    ///
    /// # Returns
    /// Ok(()) on successful send, or an error if the send failed.
    pub fn send_transaction(
        &self,
        tpu_addr: SocketAddr,
        transaction: &VersionedTransaction,
    ) -> Result<(), CatscopeZerohopError> {
        let wire_tx = bincode::serialize(transaction)
            .map_err(|e| CatscopeZerohopError::ConfigError(format!("Serialization error: {e}")))?;

        self.send_wire_transaction(tpu_addr, &wire_tx)
    }

    /// Sends a pre-serialized transaction to the specified TPU address.
    ///
    /// This is more efficient when you already have the serialized transaction
    /// bytes, avoiding redundant serialization.
    ///
    /// # Arguments
    /// * `tpu_addr` - The TPU QUIC address of the leader validator
    /// * `wire_transaction` - The serialized transaction bytes
    pub fn send_wire_transaction(
        &self,
        tpu_addr: SocketAddr,
        wire_transaction: &[u8],
    ) -> Result<(), CatscopeZerohopError> {
        debug!("Sending transaction to TPU at {}", tpu_addr);

        let connection = self.connection_cache.get_connection(&tpu_addr);

        connection.send_data(wire_transaction).map_err(|e| {
            error!("Failed to send transaction to {}: {}", tpu_addr, e);
            CatscopeZerohopError::ConnectionError(format!("QUIC send error: {}", e))
        })?;

        debug!("Transaction sent successfully to {}", tpu_addr);
        Ok(())
    }

    /// Sends multiple transactions in a batch to the specified TPU address.
    ///
    /// This is more efficient than sending transactions individually as it
    /// reuses the same QUIC connection for all transactions.
    ///
    /// # Arguments
    /// * `tpu_addr` - The TPU QUIC address of the leader validator
    /// * `transactions` - The transactions to send
    pub fn send_transactions_batch(
        &self,
        tpu_addr: SocketAddr,
        transactions: &[VersionedTransaction],
    ) -> Result<(), CatscopeZerohopError> {
        let wire_txs: Result<Vec<Vec<u8>>, _> = transactions
            .iter()
            .map(|tx| bincode::serialize(tx))
            .collect();

        let wire_txs = wire_txs.map_err(|e| {
            CatscopeZerohopError::ConfigError(format!("Serialization error: {}", e))
        })?;

        self.send_wire_transactions_batch(tpu_addr, &wire_txs)
    }

    /// Sends multiple pre-serialized transactions in a batch.
    ///
    /// # Arguments
    /// * `tpu_addr` - The TPU QUIC address of the leader validator
    /// * `wire_transactions` - The serialized transaction bytes
    pub fn send_wire_transactions_batch(
        &self,
        tpu_addr: SocketAddr,
        wire_transactions: &[Vec<u8>],
    ) -> Result<(), CatscopeZerohopError> {
        debug!(
            "Sending batch of {} transactions to TPU at {}",
            wire_transactions.len(),
            tpu_addr
        );

        let connection = self.connection_cache.get_connection(&tpu_addr);

        connection.send_data_batch(wire_transactions).map_err(|e| {
            error!("Failed to send transaction batch to {}: {}", tpu_addr, e);
            CatscopeZerohopError::ConnectionError(format!("QUIC batch send error: {}", e))
        })?;

        debug!(
            "Batch of {} transactions sent successfully to {}",
            wire_transactions.len(),
            tpu_addr
        );
        Ok(())
    }
}

/// Helper to get the TPU QUIC port from a regular TPU port.
///
/// Solana validators expose TPU on port N and TPU QUIC on port N+6.
pub fn tpu_quic_port(tpu_port: u16) -> u16 {
    tpu_port.saturating_add(6)
}

/// Converts a TPU address to its QUIC counterpart.
///
/// # Arguments
/// * `tpu_addr` - The regular TPU address
///
/// # Returns
/// The TPU QUIC address (same IP, port + 6)
pub fn to_tpu_quic_addr(tpu_addr: SocketAddr) -> SocketAddr {
    SocketAddr::new(tpu_addr.ip(), tpu_quic_port(tpu_addr.port()))
}

/// Information about a leader and their TPU address.
#[derive(Debug, Clone)]
pub struct LeaderTpuInfo {
    /// The leader's public key.
    pub pubkey: Pubkey,
    /// The slot this leader is assigned to.
    pub slot: u64,
    /// The TPU QUIC socket address for sending transactions.
    pub tpu_quic_addr: SocketAddr,
}

/// Gets the TPU QUIC addresses for upcoming leaders from RPC.
///
/// This function queries the RPC for the leader schedule and cluster node
/// information to determine where to send transactions via QUIC.
///
/// # Arguments
/// * `rpc_client` - The Solana RPC client
/// * `num_leaders` - Number of upcoming leader slots to fetch (default: 4)
///
/// # Returns
/// A vector of `LeaderTpuInfo` containing the leader pubkey, slot, and TPU QUIC address.
/// Leaders without a known TPU address are excluded from the results.
pub fn get_leader_tpu_addresses(
    rpc_client: &RpcClient,
    num_leaders: Option<u64>,
) -> Result<Vec<LeaderTpuInfo>, CatscopeZerohopError> {
    let num_leaders = num_leaders.unwrap_or(4);

    // Get current slot
    let current_slot = rpc_client.get_slot().map_err(|e| {
        CatscopeZerohopError::ConnectionError(format!("Failed to get current slot: {e}"))
    })?;

    //debug!("Current slot: {}", current_slot);

    // Get upcoming slot leaders
    let leaders = rpc_client
        .get_slot_leaders(current_slot, num_leaders)
        .map_err(|e| {
            CatscopeZerohopError::ConnectionError(format!("Failed to get slot leaders: {e}",))
        })?;

    //debug!("Got {} upcoming leaders", leaders.len());

    // Get cluster nodes to map pubkeys to TPU addresses
    let cluster_nodes = rpc_client.get_cluster_nodes().map_err(|e| {
        CatscopeZerohopError::ConnectionError(format!("Failed to get cluster nodes: {e}"))
    })?;

    // Build a map of pubkey -> TPU QUIC address
    let mut tpu_map: HashMap<Pubkey, SocketAddr> = HashMap::new();
    for node in cluster_nodes {
        if let Some(tpu_quic) = node.tpu_quic {
            if let Ok(pubkey) = node.pubkey.parse::<Pubkey>() {
                tpu_map.insert(pubkey, tpu_quic);
            }
        } else if let Some(tpu) = node.tpu {
            // Fall back to computing QUIC port from TPU port
            if let Ok(pubkey) = node.pubkey.parse::<Pubkey>() {
                tpu_map.insert(pubkey, to_tpu_quic_addr(tpu));
            }
        }
    }

    debug!("Built TPU map with {} entries", tpu_map.len());

    // Map leaders to their TPU addresses
    let mut result = Vec::with_capacity(leaders.len());
    for (i, leader) in leaders.iter().enumerate() {
        let slot = current_slot + i as u64;
        if let Some(&tpu_quic_addr) = tpu_map.get(leader) {
            result.push(LeaderTpuInfo {
                pubkey: *leader,
                slot,
                tpu_quic_addr,
            });
        } else {
            warn!("No TPU address found for leader {leader} at slot {slot}",);
        }
    }

    Ok(result)
}

/// Gets the TPU QUIC address for the current leader from RPC.
///
/// This is a convenience function that returns just the socket address
/// for the current slot's leader.
///
/// # Arguments
/// * `rpc_client` - The Solana RPC client
///
/// # Returns
/// The TPU QUIC socket address for the current leader, or an error if not found.
pub fn get_current_leader_tpu_address(
    rpc_client: &RpcClient,
) -> Result<SocketAddr, CatscopeZerohopError> {
    let leaders = get_leader_tpu_addresses(rpc_client, Some(1))?;
    leaders
        .into_iter()
        .next()
        .map(|info| info.tpu_quic_addr)
        .ok_or_else(|| {
            CatscopeZerohopError::ConnectionError(
                "No TPU address found for current leader".to_string(),
            )
        })
}

/// How long to keep retrying a transaction batch.
/// Solana blockhashes are valid for ~150 slots (~60 s); 30 s is a safe window.
const PENDING_TX_TTL: Duration = Duration::from_secs(30);

struct PendingBatch {
    txs: Vec<Vec<u8>>,
    sent_at: Instant,
}

/// Submit transactions to validators.
pub struct QuicRequestHandler {
    response: TransactionResponse,
    quic: SolanaQuicClient,
    l_tx: Vec<Vec<u8>>,
    /// Leader list kept fresh by the background refresh thread.
    l_leader: Arc<Mutex<Vec<LeaderTpuInfo>>>,
    /// Batches waiting to be retried by the background thread.
    pending: Arc<Mutex<VecDeque<PendingBatch>>>,
    refresh_stop: Arc<AtomicBool>,
    refresh_thread: Option<std::thread::JoinHandle<()>>,
}

impl QuicRequestHandler {
    /// * `rpc_url` — HTTP RPC endpoint used for leader-schedule queries.
    /// * `quic_client` — authenticated QUIC sender.
    ///
    /// Performs an initial leader fetch synchronously, then spawns a background
    /// thread that refreshes the list every ~400 ms (one Solana slot) regardless
    /// of whether `on_request` is being called.
    pub fn new(
        rpc_url: String,
        quic_client: SolanaQuicClient,
    ) -> Result<Self, CatscopeZerohopError> {
        // Best-effort initial fetch — if the RPC is unreliable at startup,
        // start with an empty list and let the background thread fill it in.
        let rpc = RpcClient::new(rpc_url.clone());
        let initial = match get_leader_tpu_addresses(&rpc, Some(8)) {
            Ok(leaders) => leaders,
            Err(e) => {
                warn!("initial leader fetch failed, background thread will retry: {e}");
                Vec::new()
            }
        };
        let l_leader = Arc::new(Mutex::new(initial));

        // Second QUIC client for the background retry thread.
        // It shares the same keypair identity but has its own connection cache,
        // so retries never contend with on_request sends.
        let retry_quic = SolanaQuicClient::new(
            quic_client.keypair().insecure_clone(),
            Some(quic_client.config().clone()),
        )?;

        let pending = Arc::new(Mutex::new(VecDeque::<PendingBatch>::new()));

        let stop = Arc::new(AtomicBool::new(false));
        let stop_bg = stop.clone();
        let l_leader_bg = l_leader.clone();
        let pending_bg = pending.clone();

        let handle = std::thread::spawn(move || {
            let rpc = RpcClient::new(rpc_url);
            // How long to wait between refresh attempts: shorter on failure.
            let mut sleep_ms: u64 = 400;
            loop {
                // Sleep in 50 ms increments so shutdown is noticed quickly.
                let steps = (sleep_ms / 50).max(1);
                for _ in 0..steps {
                    if stop_bg.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                match get_leader_tpu_addresses(&rpc, Some(8)) {
                    Ok(leaders) => {
                        sleep_ms = 400;
                        *l_leader_bg.lock().unwrap() = leaders.clone();

                        // Retry all pending batches against the fresh leader list.
                        let now = Instant::now();
                        let mut pending = pending_bg.lock().unwrap();
                        pending.retain(|b| now.duration_since(b.sent_at) < PENDING_TX_TTL);
                        for batch in &*pending {
                            for leader in &leaders {
                                if let Err(e) = retry_quic.send_wire_transactions_batch(
                                    leader.tpu_quic_addr,
                                    &batch.txs,
                                ) {
                                    debug!("retry send to {} failed: {e}", leader.tpu_quic_addr);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("leader refresh failed: {e}");
                        // Back off up to 5 s so we don't hammer a flaky RPC.
                        sleep_ms = (sleep_ms * 2).min(5_000);
                    }
                }
            }
        });

        Ok(Self {
            quic: quic_client,
            l_tx: Vec::default(),
            response: TransactionResponse { sent_count: 0 },
            l_leader,
            pending,
            refresh_stop: stop,
            refresh_thread: Some(handle),
        })
    }
}

impl Drop for QuicRequestHandler {
    fn drop(&mut self) {
        self.refresh_stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.refresh_thread.take() {
            let _ = t.join();
        }
    }
}

impl RequestHandler<TransactionRequest, TransactionResponse> for QuicRequestHandler {
    fn on_request(
        &mut self,
        request: &TransactionRequest,
    ) -> Result<&TransactionResponse, CatscopeZerohopError> {
        let mut start = 0;
        let n = request.count as usize;
        debug!("on_request - 1 - n {n}");
        self.l_tx.clear();
        for i in 0..n {
            let finish = request.list[i];
            self.l_tx
                .push(request.data[(start as usize)..(finish as usize)].to_vec());
            start = finish;
        }
        self.response.sent_count = 0;

        // Queue for background retry before the first send attempt.
        self.pending.lock().unwrap().push_back(PendingBatch {
            txs: self.l_tx[0..n].to_vec(),
            sent_at: Instant::now(),
        });

        // Snapshot the current leader list without holding the lock during sends.
        let leaders = self.l_leader.lock().unwrap().clone();
        debug!("on_request - 2 - {} leaders", leaders.len());
        for leader in &leaders {
            match self
                .quic
                .send_wire_transactions_batch(leader.tpu_quic_addr, &self.l_tx[0..n])
            {
                Ok(_) => self.response.sent_count += 1,
                Err(e) => debug!("failed to send to {}: {e}", leader.tpu_quic_addr),
            }
        }
        debug!("on_request - 3 - sent_count {}", self.response.sent_count);
        if self.response.sent_count == 0 {
            Err(CatscopeZerohopError::OutofRange)
        } else {
            Ok(&self.response)
        }
    }
}

pub const MAX_TX_BUNDLE_SIZE: usize = 20;
pub const MAX_TX_SIZE: usize = 1_500;

#[derive(Debug, Clone, Copy, ZeroCopySend)]
#[repr(C)]
pub struct TransactionRequest {
    pub count: u8,
    pub list: [u16; MAX_TX_BUNDLE_SIZE],
    /// send a max of 10 transactions
    pub data: [u8; MAX_TX_SIZE * MAX_TX_BUNDLE_SIZE],
}
impl Default for TransactionRequest {
    fn default() -> Self {
        Self {
            count: 0,
            list: [0u16; MAX_TX_BUNDLE_SIZE],
            data: [0u8; MAX_TX_SIZE * MAX_TX_BUNDLE_SIZE],
        }
    }
}
impl TransactionRequest {
    /// Append a transaction. Returns true if successful.
    pub fn append(&mut self, data: &[u8]) -> bool {
        if MAX_TX_BUNDLE_SIZE <= self.count as usize {
            return false;
        }
        let i = self.count;
        self.count += 1;
        let start = (if i == 0 { 0 } else { self.list[i as usize - 1] }) as usize;
        if MAX_TX_SIZE < data.len() {
            return false;
        }
        let finish = start + data.len();
        let subbuf = &mut self.data[start..finish];
        subbuf.copy_from_slice(data);
        true
    }
}

#[derive(Debug, Clone, Copy, ZeroCopySend)]
#[repr(C)]
pub struct TransactionResponse {
    pub sent_count: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let keypair = Keypair::new();
        let pubkey = keypair.pubkey();
        let client = SolanaQuicClient::new(keypair, None).unwrap();

        assert_eq!(client.pubkey(), pubkey);
    }

    #[test]
    fn test_client_with_config() {
        let keypair = Keypair::new();
        let config = QuicClientConfig {
            connection_timeout: Duration::from_secs(30),
            skip_preflight: false,
            connection_pool_size: 8,
        };

        let client = SolanaQuicClient::new(keypair, Some(config.clone())).unwrap();
        assert!(!client.config.skip_preflight);
        assert_eq!(client.config.connection_pool_size, 8);
    }

    #[test]
    fn test_default_config() {
        let config = QuicClientConfig::default();
        assert!(config.skip_preflight);
        assert_eq!(config.connection_pool_size, 4);
    }

    #[test]
    fn test_tpu_quic_port() {
        assert_eq!(tpu_quic_port(8000), 8006);
        assert_eq!(tpu_quic_port(0), 6);
        assert_eq!(tpu_quic_port(u16::MAX - 10), u16::MAX - 4);
        // Test saturation
        assert_eq!(tpu_quic_port(u16::MAX), u16::MAX);
    }

    #[test]
    fn test_to_tpu_quic_addr() {
        let tpu_addr: SocketAddr = "127.0.0.1:8000".parse().unwrap();
        let quic_addr = to_tpu_quic_addr(tpu_addr);
        assert_eq!(quic_addr.to_string(), "127.0.0.1:8006");
    }
}
