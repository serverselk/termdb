# termdb

A lightweight, native multi-engine database client for **MySQL** and
**PostgreSQL**.

termdb is a fast, no-electron alternative to heavy DB GUIs: a single native
binary, immediate-mode UI, and all database work off the UI thread. Originally
written as a Rust rewrite of an Electron SQL client, it keeps the same
field-level session tools (browse, filter, edit, run SQL, export) without the
webview.

## Features

- **Multi-engine** — MySQL and PostgreSQL via `sqlx` with typed, prepared
  statements (no string-built SQL for user values).
- **Connection manager** — server config in a local SQLite store, passwords in
  the OS keyring (Secret Service / Keychain / DPAPI) with a chmod-0600
  plaintext fallback when no keyring is available.
- **Browse** — sidebar tree of connections → databases → tables, favorite
  stars, click-to-connect.
- **Data grid** — virtualized, resizable columns, pagination
  (25/50/100/200 per page), typed headers from `information_schema` describe
  (Field / Type / Null / Key / Default / Extra), stable primary-key ordering.
- **Filters** — `+ Add Filter` WHERE builder (column + operator + value, always
  parametrized).
- **Row editing** — add / edit / delete rows from centered dialogs, auto-increment
  & serial-aware, with delete confirmations.
- **SQL editor** — syntax-highlighted query editor with persistent history and
  CSV / JSON export via native save dialogs.
- **Live themes** — a dark slate theme by default; on Linux it follows your
  [Omarchy](https://omarchy.org/) theme and repaints when you switch
  (`omarchy theme set`). Fully read-only, safe fallback to the default palette,
  and a no-op on macOS/Windows.

## Requirements

- Rust (stable, 1.8x+) — `cargo`
- A display + the usual GUI dev libraries on Linux (Wayland/X11 via `egui`)

## Quick start

```sh
cargo build --release
cargo run
```

1. Click **+ NEW CONNECTION** in the top bar and save a MySQL/Postgres server.
2. Click the connection to connect, expand a database, pick a table.
3. Browse pages, add filters, edit rows, or run ad-hoc SQL in the **QUERY EDITOR**.

Passwords are stored in the OS keyring and never written into the UI log.

## Architecture

egui runs immediate-mode on the UI thread and never touches the network. A
dedicated thread hosts a tokio runtime (sqlx pools); the two sides talk over
`mpsc` channels drained every frame.

```
UI thread (egui)  ──requests──▶  tokio runtime  ──sqlx──▶  MySQL / PostgreSQL
      ▲                             │
      └───────────── events / results ─┘
```

- `crates/app` — the native GUI (`src/app/ui/{header,sidebar,table,query}`),
  the backend worker (`src/db`), theme & Omarchy sync (`src/theme.rs`)
- `crates/core` — shared repos: `rusqlite` config store, `keyring` vault,
  models (no UI dependencies)

## Development

```sh
cargo build
cargo test             # unit + live-engine integration tests
cargo clippy --workspace --all-targets
cargo fmt --check
cargo run
```

Live-engine tests spin up containers via `dev/compose.yaml`
(Postgres on `:5433`, MySQL on `:3306`) and auto-skip with a note when a
container is unreachable:

```sh
docker compose -f dev/compose.yaml up -d
cargo test --workspace
```

### Project layout

```
crates/
  app/
    src/
      app.rs            # app state & event handling
      app/ui/           # header bar, sidebar, table, query/results cards
      db/               # backend worker + sqlx engine
      theme.rs          # palette + Omarchy ThemeWatcher
      export.rs         # CSV/JSON serializers (unit-tested)
      record.rs         # form defaults (unit-tested)
    tests/live_engines.rs
  core/                 # config store, vault, models
dev/compose.yaml        # local test databases
```

## Roadmap

- **More database engines** (SQLite, SQL Server, Oracle, CockroachDB…)
- MCP server (JSON-RPC on loopback) so AI tooling can drive the client
- SSH tunnels, ERD view, and schema/DDL helpers

## Contributing

Open an issue or pull request. Keep changes focused and run the check list
above (`fmt`, `clippy`, `test`) before submitting.

## License

MIT — see [LICENSE](LICENSE).