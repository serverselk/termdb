# termdb

Lightweight multi-engine database tool. A native rewrite of the Electron
`tup-db-client`, built with egui.

- GUI: `eframe`/`egui` — single native binary, no webview
- Async I/O: tokio runtime on a background thread
- Storage: `rusqlite` config store, `keyring` vault with plaintext fallback

## Layout

- `crates/app` — the native GUI binary (egui shell, backend skeleton)
- `crates/core` — config store, credential vault, shared models (no UI deps)

## Build & test

```sh
cargo build
cargo test
cargo clippy --workspace --all-targets
cargo fmt --check
```

## License

MIT, see [LICENSE](LICENSE).
