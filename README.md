# glittr-core

## Building

Install prerequisites (example for ubuntu/debian)
```bash
sudo apt install clang build-essential
```

Install rust:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Clone the glittr-core repo:
```bash
git clone https://github.com/Glittrfi/glittr-core
```

Build
```bash
cargo build --release
```

The `glittr` binary will be on `./target/release/glittr`

## Running

Set your node settings on `settings.yaml`.

```yaml
btc_rpc_url: http://127.0.0.1:18443
btc_rpc_username: user
btc_rpc_password: password
rocks_db_path: db_data
api_url: 127.0.0.1:3001
```

1. btc_rpc_url - URL for your local Bitcoin RPC (you can run your own or check`./local_env.sh`)
2. btc_rpc_username - Username for the Bitcoin RPC
3. btc_rpc_password - Password for the Bitcoin RPC
4. rocks_db_path - Local storage path for glittr node database (you can leave it as is)
5. api_url - Your glittr node API URL

After preparing all of the above, run the glittr node `./target/release/glittr`
