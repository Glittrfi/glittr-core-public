# glittr-core
Native Bitcoin Smart Contracts, Execution, & Settlement

Direct Bitcoin Layer 1 TXN Settlement with Proof-of-Stake Secured Network. No Bridges, No Multisig, just Bitcoin Finality.

# Design Docs
All protocol information is available at https://docs.glittr.fi/

1st version of whitepaper is available at https://github.com/Glittrfi/whitepaper/ (next version on progress)

# Contributing
## Prerequisites
### Required Tools
Install system dependencies (Ubuntu/Debian):
```bash
sudo apt install clang build-essential
```

Install Rust toolchain (https://doc.rust-lang.org/cargo/getting-started/installation.html):
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env  # Load Rust into current shell
```

## Installation

1. Clone the repository:
```bash
git clone https://github.com/Glittrfi/glittr-core
cd glittr-core
```

2. Build the project:
```bash
cargo build --release --features helper-api
```

The compiled binary will be available at `./target/release/glittr`

## Configuration

Create a `settings.yaml` file in your project directory:

```yaml
btc_rpc_url: http://127.0.0.1:18443
btc_rpc_username: user
btc_rpc_password: password
rocks_db_path: db_data
api_url: 127.0.0.1:3001
```

### Configuration Options

| Option | Description | Default |
|--------|-------------|---------|
| `btc_rpc_url` | Bitcoin RPC endpoint URL | `http://127.0.0.1:18443` |
| `btc_rpc_username` | Bitcoin RPC authentication username | - |
| `btc_rpc_password` | Bitcoin RPC authentication password | - |
| `rocks_db_path` | Local storage path for the node database | `db_data` |
| `api_url` | Glittr node API listening address | `127.0.0.1:3001` |

## Running the Node

Start the Glittr node:
```bash
./target/release/glittr
```

## Local environment
### Required tools

1. bitcoin-cli
   - Install via Bitcoin Core: https://bitcoin.org/en/download
   - Or build from source: https://github.com/bitcoin/bitcoin

2. bitcoind
   - Included with Bitcoin Core installation
   - Or build from source: https://github.com/bitcoin/bitcoin

3. Blockstream's electrs
   - Follow installation guide at: https://github.com/Blockstream/electrs

### How to run

For local development:

1. Start the local Bitcoin node and services using `local_env.sh`
2. Create your bitcoin wallet, run `scripts/create_wallet.sh`
3. Generate blocks every 10s with `scripts/generate_blocks.sh`

### How to get BTC to your wallet
```bash
bitcoin-cli -rpcpassword=password -rpcuser=user -chain=regtest -named sendtoaddress address="{bitcoin_address}" amount=0.5 fee_rate=1
```

If this your first time running bitcoind, generate 100 blocks for the coinbase maturity requirement:
```bash
bitcoin-cli -rpcpassword=password -rpcuser=user  -chain=regtest -generate 100 1000
```


