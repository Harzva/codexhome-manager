# 11-factor CLI Audit — v0.2.0-alpha.1

## Summary

- Project: CodexHome Manager
- Runtime: Rust 1.85+
- Entrypoint: `codexhome`
- Overall score: **89/100**
- Readiness: strong local alpha; public binary release still needs CI, package publishing, and signed artifacts.

## Command map

| Command | Purpose | Status |
|---|---|---|
| `scan` | Discover local Homes | Pass |
| `inspect` | Inspect a discovered Home safely | Pass |
| `doctor` | Diagnose discovered Homes | Pass |
| `registry list/show/path` | Read persistent registry state | Pass |
| `home create/import/clone` | Manage Home lifecycle | Pass |
| `observe record/summary/export/verify` | Record and analyze execution facts | Pass |
| `task create/list/show` | Manage durable task identities | Pass |
| `run start/list/show/...` | Track Agent Run lifecycle, budgets, recovery, and artifacts | Pass |

## Score

| Dimension | Score | Evidence |
|---|---:|---|
| Installability | 11/12 | Cargo entrypoint, Rust requirement, and local install documented |
| Command surface | 10/10 | Consistent noun/verb hierarchy; risky copying is opt-in |
| Help and examples | 9/10 | Root and clone help tested; README recipes align |
| Input validation | 8/8 | Alias, path, specialty, duplicate, size, and nesting checks precede writes |
| Error handling | 9/10 | Actionable context, JSON error envelope, stable 0/1/2 exits |
| Configuration and environment | 8/8 | Flag/env/default precedence documented; credentials excluded |
| Output contract | 10/10 | Versioned JSON Schemas and clean stdout contract |
| Logging and verbosity | 2/6 | Clean default output, but no dedicated verbose/quiet modes yet |
| Automation friendliness | 8/8 | Dry-run, JSON, deterministic exits, and no interactive prompts |
| Testing and smoke checks | 10/10 | Root/subcommand help, lifecycle, redaction, rollback, and failures tested |
| Packaging and release readiness | 4/8 | Version/changelog/release build exist; CI and signed publishing remain |

## Findings

### What works

- A clean user can install with Cargo and discover or register Homes immediately.
- JSON mode never mixes human diagnostics into stdout, including command failures.
- Create and clone roll back their new destination when registry commit or copying fails.
- Desktop adapters have versioned read and mutation contracts.

### Public release gaps

- Add macOS/Linux/Windows CI before claiming cross-platform release support.
- Add `--verbose` diagnostics and a script-friendly quiet mode where output is optional.
- Document crates/binary publishing and add signed release artifacts.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release --workspace
cargo run -p codexhome-cli -- --help
cargo run -p codexhome-cli -- home clone --help
```

## Next refactor iteration

1. Establish cross-platform CI and release packaging.
2. Add verbose/quiet behavior without changing JSON contracts.
3. Add schema conformance fixtures consumed by both CLI and Desktop UI tests.
