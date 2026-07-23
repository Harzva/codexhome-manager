# Desktop Adapter Contract

The Desktop UI should consume `codexhome-core` directly when embedded in Rust. Other desktop runtimes may initially invoke the CLI with `--json`; they must never parse human-readable output or edit `registry.json` directly.

## Read models

- `codexhome registry list --json` returns `codexhome.registry-report.v1` for Home cards, availability, aliases, specialties, providers, and capability counts.
- `codexhome registry show @alias --json` returns one expanded Home card.
- `codexhome scan --json` returns unregistered discovery candidates.
- `codexhome observe summary --json` returns `codexhome.observability-summary.v1` for token, duration, cache, failure, cost, trace, and Home health views.
- `codexhome observe export --format json|csv --output <FILE>` produces detailed filtered events for local analysis.

## Mutations

- `codexhome home create ... --dry-run --json`
- `codexhome home import ... --dry-run --json`
- `codexhome home clone ... --dry-run --json`

The UI should show `plannedActions` and `warnings`, request confirmation, then repeat the command without `--dry-run`. A successful mutation returns the new registry revision. Exit code `2` returns a `codexhome.error.v1` JSON envelope when `--json` is active.

## Privacy rules

Reports contain local paths and may contain provider hostnames, but exclude credential contents. The UI must not upload reports without an explicit user action and a redaction preview. Authentication setup belongs to the target Home and is never inherited from another Home.

Observability views must consume the typed summary or export contracts. They must not infer metrics from raw chat text, read `auth.json`, or accept arbitrary event payloads.
