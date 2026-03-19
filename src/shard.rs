use std::{
    collections::HashSet,
    hash::{BuildHasherDefault, Hasher as _},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use crossbeam::queue::SegQueue;
use crossbeam_channel::bounded;
use solana_sdk::pubkey::Pubkey;
use twox_hash::XxHash64;

/// Capacity of the per-shard inbound channel.
///
/// Larger values buffer more items between readers and workers (higher
/// throughput when workers lag briefly), at the cost of extra memory.
/// At 40 bytes per `(u64, Pubkey)`, 64 K entries ≈ 2.5 MB per shard.
const CHANNEL_CAP: usize = 1 << 16; // 65 536

/// Micro-batch size for reader threads.
///
/// Readers collect this many items from the `SegQueue` before routing
/// them to shard channels, amortising per-pop overhead.
const BATCH: usize = 256;

// ── internal helper ──────────────────────────────────────────────────────────

/// Hash a [`Pubkey`] with XxHash64 (seed 0) and return `(hash, shard_index)`.
#[inline(always)]
fn route(key: &Pubkey, n_shards: u64) -> (u64, usize) {
    let mut h = XxHash64::with_seed(0);
    h.write(key.as_ref());
    let hash = h.finish();
    (hash, (hash % n_shards) as usize)
}

// ── public API ───────────────────────────────────────────────────────────────

/// Drain a [`SegQueue<Pubkey>`], shard every key by XxHash64, and call
/// `on_key` **once per unique key per shard**.
///
/// # Pipeline
///
/// ```text
///  SegQueue<Pubkey>
///      │
///      ├─ reader-0 ─┐
///      ├─ reader-1 ─┤  XxHash64 % n_shards  →  shard index (u64 → usize)
///      └─ reader-N ─┘
///                   │
///         ┌─────────┴──────────────────┐
///         ▼                            ▼
///   chan[0] (bounded)     …     chan[n-1] (bounded)
///         │                            │
///   shard-worker-0             shard-worker-n-1
///   HashSet<Pubkey, XxHash64>  HashSet<Pubkey, XxHash64>
///         │
///   on_key(shard_id, xxhash64, pubkey)   ← unique keys only
/// ```
///
/// # Arguments
///
/// * `queue`          – Shared source queue. Producers push [`Pubkey`]s here.
/// * `done`           – Set to `true` **after** all producers have finished
///                      enqueuing. The readers perform one final drain after
///                      observing this flag and then shut down.
/// * `n_shards`       – Number of independent shards. One OS thread is spawned
///                      per shard. Recommended: `num_cpus::get()` or a power of two.
/// * `n_readers`      – Threads that pop from the queue and route keys.
///                      2–4 is typically optimal; more can saturate the queue.
/// * `capacity_hint`  – Expected unique keys *per shard* for `HashSet`
///                      pre-sizing. Pass `0` to skip pre-allocation.
/// * `on_key`         – Called once per unique [`Pubkey`] within its shard.
///                      Signature: `fn(shard_id: usize, hash: u64, key: Pubkey)`.
///
/// # Completion channel
///
/// Returns a [`crossbeam_channel::Receiver<()>`] that receives a single `()`
/// value once all pubkeys have been processed.  The caller can `.recv()` on
/// it to block, or use it in a `crossbeam_channel::select!` with other work.
///
/// # Panics
///
/// Panics if `n_shards == 0` or `n_readers == 0`.
pub fn shard_dedup_pubkeys<F>(
    queue: Arc<SegQueue<Pubkey>>,
    done: Arc<AtomicBool>,
    n_shards: usize,
    n_readers: usize,
    capacity_hint: usize,
    on_key: F,
) -> crossbeam_channel::Receiver<()>
where
    F: Fn(usize, u64, Pubkey) + Send + Sync + 'static,
{
    assert!(n_shards >= 1, "n_shards must be >= 1");
    assert!(n_readers >= 1, "n_readers must be >= 1");

    let on_key = Arc::new(on_key);
    let n = n_shards as u64;

    // ── shard worker threads (one per shard) ──────────────────────────────
    //
    // Each worker owns a `HashSet<Pubkey>` and calls `on_key` only on the
    // first occurrence of each key.  Using `BuildHasherDefault<XxHash64>`
    // for the inner HashSet avoids a second round of heavy cryptographic
    // hashing: Pubkeys are already high-entropy 32-byte values.

    let (txs, rxs): (Vec<_>, Vec<_>) = (0..n_shards)
        .map(|_| bounded::<(u64, Pubkey)>(CHANNEL_CAP))
        .unzip();

    let shard_handles: Vec<_> = rxs
        .into_iter()
        .enumerate()
        .map(|(shard_id, rx)| {
            let on_key = on_key.clone();
            thread::Builder::new()
                .name(format!("shard-worker-{shard_id}"))
                .spawn(move || {
                    let mut seen: HashSet<Pubkey, BuildHasherDefault<XxHash64>> =
                        if capacity_hint > 0 {
                            HashSet::with_capacity_and_hasher(
                                capacity_hint,
                                BuildHasherDefault::default(),
                            )
                        } else {
                            HashSet::with_hasher(BuildHasherDefault::default())
                        };

                    for (hash, key) in rx {
                        if seen.insert(key) {
                            on_key(shard_id, hash, key);
                        }
                    }
                })
                .expect("failed to spawn shard worker thread")
        })
        .collect();

    // ── reader threads ────────────────────────────────────────────────────
    //
    // Each reader micro-batches pops from the SegQueue to amortise the
    // per-pop overhead, then routes each key to the correct shard channel
    // via a blocking `send` (back-pressure when a worker is lagging).

    let txs = Arc::new(txs);

    let reader_handles: Vec<_> = (0..n_readers)
        .map(|i| {
            let queue = queue.clone();
            let done = done.clone();
            let txs = txs.clone();

            thread::Builder::new()
                .name(format!("shard-reader-{i}"))
                .spawn(move || {
                    let mut batch: Vec<Pubkey> = Vec::with_capacity(BATCH);

                    loop {
                        // Drain a micro-batch from the queue.
                        while batch.len() < BATCH {
                            match queue.pop() {
                                Some(key) => batch.push(key),
                                None => break,
                            }
                        }

                        // Route the batch to the appropriate shard channels.
                        for key in batch.drain(..) {
                            let (hash, shard) = route(&key, n);
                            // Blocking send: if the shard worker is slow,
                            // this naturally throttles the reader.
                            txs[shard].send((hash, key)).ok();
                        }

                        // Check for termination only after the fast path.
                        if done.load(Ordering::Acquire) {
                            // Final drain: no more producers will enqueue
                            // after `done` is set, so one pass is enough.
                            while let Some(key) = queue.pop() {
                                let (hash, shard) = route(&key, n);
                                txs[shard].send((hash, key)).ok();
                            }
                            return;
                        }

                        // Queue was empty but producers are not done yet.
                        thread::yield_now();
                    }
                })
                .expect("failed to spawn shard reader thread")
        })
        .collect();

    // Coordinator thread: waits for readers → closes channels → waits for
    // shard workers → signals completion on the returned channel.
    let (complete_tx, complete_rx) = bounded::<()>(1);

    thread::Builder::new()
        .name("shard-coordinator".to_string())
        .spawn(move || {
            for h in reader_handles {
                h.join().expect("shard reader thread panicked");
            }
            drop(txs); // last Arc ref → Sender halves drop → channels close

            for h in shard_handles {
                h.join().expect("shard worker thread panicked");
            }

            complete_tx.send(()).ok();
        })
        .expect("failed to spawn shard coordinator thread");

    complete_rx
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    fn random_pubkey(seed: u64) -> Pubkey {
        let mut h = XxHash64::with_seed(seed);
        h.write_u64(seed);
        let a = h.finish();
        h.write_u64(a);
        let b = h.finish();
        h.write_u64(b);
        let c = h.finish();
        h.write_u64(c);
        let d = h.finish();
        let mut bytes = [0u8; 32];
        bytes[0..8].copy_from_slice(&a.to_le_bytes());
        bytes[8..16].copy_from_slice(&b.to_le_bytes());
        bytes[16..24].copy_from_slice(&c.to_le_bytes());
        bytes[24..32].copy_from_slice(&d.to_le_bytes());
        Pubkey::new_from_array(bytes)
    }

    #[test]
    fn test_shard_dedup_basic() {
        const N: usize = 100_000;
        const N_SHARDS: usize = 8;
        const DUPES: usize = 4; // each key inserted DUPES times

        let queue: Arc<SegQueue<Pubkey>> = Arc::new(SegQueue::new());
        let done = Arc::new(AtomicBool::new(false));

        // Push N unique keys × DUPES duplicates.
        for i in 0..N {
            let key = random_pubkey(i as u64);
            for _ in 0..DUPES {
                queue.push(key);
            }
        }
        done.store(true, Ordering::Release);

        let unique_count = Arc::new(AtomicU64::new(0));
        let uc = unique_count.clone();

        let rx = shard_dedup_pubkeys(
            queue,
            done,
            N_SHARDS,
            2,
            N / N_SHARDS + 1,
            move |_shard, _hash, _key| {
                uc.fetch_add(1, Ordering::Relaxed);
            },
        );
        rx.recv().unwrap();

        assert_eq!(unique_count.load(Ordering::Relaxed), N as u64);
    }

    #[test]
    fn test_shard_routing_correctness() {
        const N: usize = 10_000;
        const N_SHARDS: usize = 4;

        let queue: Arc<SegQueue<Pubkey>> = Arc::new(SegQueue::new());
        let done = Arc::new(AtomicBool::new(false));

        for i in 0..N {
            queue.push(random_pubkey(i as u64));
        }
        done.store(true, Ordering::Release);

        // Verify each key is routed to the shard that matches its hash.
        let rx = shard_dedup_pubkeys(
            queue,
            done,
            N_SHARDS,
            2,
            0,
            move |shard_id, hash, _key| {
                let expected = (hash % N_SHARDS as u64) as usize;
                assert_eq!(shard_id, expected, "key routed to wrong shard");
            },
        );
        rx.recv().unwrap();
    }
}
