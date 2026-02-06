# binance-api-testing-rust

Workspace with two Rust binaries for Binance API experiments.

## Packages
- `account_update_streaming`: private stream (requires `BINANCE_API_KEY`)
- `public_data_streaming`: public trades stream (no key required)

## Setup
```bash
export BINANCE_API_KEY=your_key_here
```

## Run
```bash
cargo run -p account_update_streaming
cargo run -p public_data_streaming
```
