# Contributing

CodexHome Manager welcomes focused issues and pull requests for discovery, registry safety, lifecycle operations, schemas, CLI usability, and the Desktop UI.

## Development setup

Requirements:

- Rust 1.85 or newer.
- Node.js 24 for the Desktop frontend.
- Platform prerequisites required by Tauri when building the native shell.

```bash
git clone https://github.com/harzva/codexhome-manager.git
cd codexhome-manager
cargo test --workspace

cd apps/desktop
npm ci
npm run build
```

## Before opening a pull request

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release --workspace
```

Desktop changes must also pass `npm run build`. Tauri bridge changes should pass:

```bash
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml
```

## Design and compatibility rules

- Keep `codexhome-core` independent from terminal and webview concerns.
- Preserve human and versioned JSON output contracts.
- Treat macOS, Linux, and Windows as peer targets.
- Add tests for validation, rollback, malformed input, and credential redaction.
- New mutations need dry-run or an equally clear preview before risky side effects.
- Do not weaken the L0/L1/L2 orchestration depth boundary.

## Public safety

Never commit or paste:

- `auth.json`, API keys, cookies, `.env` values, or credential dumps;
- raw sessions, chat logs, or private memory stores;
- machine-specific registry files or unredacted personal paths;
- generated runtime state from a real `CODEX_HOME`.

Use synthetic fixtures and placeholders. If a secret was exposed, revoke it first and follow [SECURITY.md](SECURITY.md).
