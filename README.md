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

## Archive Notes
- `binance-api-testing-rust` is the canonical repo.
- On 2026-02-24, selected functionality was merged from `binance-api` into `public_data_streaming`.
- Merged features: unified event/message models (trade/aggTrade/ticker/kline), runtime message timing/frequency stats, and periodic unsolicited pong heartbeat behavior.
- Source provenance (`binance-api`) commit: `60078daf41fc7cf1f449d5d52fbb57c83d9321d1`.
- Original source file origin date: 2024-09-08T18:05:13+09:00.
- `binance-api` first and last commit dates are both 2024-09-08 (single-commit repo).
