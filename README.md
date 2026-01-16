# CatScope Zerohop

![ZeroHop Network Diagram](ZeroHop%20Network%20Diagram.png)

**Zerohop** is CatScope’s validator-local execution and observation interface.  
It allows plugins to **read finalized on-chain state** then receive It allows plugins to **read finalized on-chain state**, receive **ultra-low-latency updates** for the accounts in that state, and  **submit transactions** directly from the validator environment.

Zerohop is designed for MEV, arbitrage, and latency-sensitive strategies that require:

- fast access to account state
- structured graph relationships between accounts
- tight feedback loops between observation and execution

Learn more about the CatScope ecosystem at **<https://catscope.io>**.

## Run a Transaction Forwarder

```bash
solana-keygen new -o ./sender.json
```

Start the forwarder:

```bash
RPC_URL=http://localhost:8899 KEYPAIR=./sender.json BUFFER_PATH=/txsub ./target/debug/catfwd
```

- `BUFFER_PATH` does not refer to a file path, but refers to a path in shared memory space in the OS
