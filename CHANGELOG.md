# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added

- Three-platform Rust CI plus Desktop frontend and macOS Tauri bridge checks.
- Dependabot for Cargo, npm, and GitHub Actions dependencies.
- Public contribution, support, troubleshooting, release, and audit documentation.
- Structured issue and pull request templates with credential-safety reminders.
- Append-only observability events, token/cost summaries, and JSON/CSV export.
- Durable Task and Agent Run projections with retry and migration lineage.
- Token, duration, cost, and Attempt budgets with failed-attempt attribution.
- Thread, tool, opaque artifact, verification, final-artifact, and recovery tracking.
- Versioned `task` and `run` CLI JSON contracts and public schemas.
- Explainable Home/account/model routing with hard constraints and user locks.
- Immutable route candidate, score, cost, and rejection snapshots in Agent Run events.
- Atomic route evaluation/append with evaluation timestamps and observed-event counts.
- Versioned `route validate`, `route recommend`, and `route decide` CLI contracts.

### Security

- CI uses a read-only token and pinned external actions.
- GitHub private vulnerability reporting replaces public security issues.
- Agent Run events reject obvious secrets and undeclared local paths.
- Existing event-store parent directories keep their permissions; files remain owner-only.
- Sensitive-directory requests require an explicit locked Home and record no directory path.
- Linked Attempts must match the Home, account, and model selected by their route decision.

## [0.2.0-alpha.1]

### Added

- Atomic persistent Home registry with revisions and Unix permission hardening.
- Unique `@alias` routing names, family labels, and Unicode specialty tags.
- `registry list`, `registry show`, and `registry path` commands.
- Safe `home create`, `home import`, and `home clone` commands.
- Dry-run plans, rollback, capability copy limits, JSON errors, and Desktop adapter contracts.
- Registry, registry-report, and Home-mutation JSON Schemas.
- Tauri Desktop UI with Home cards, search, live registry refresh, and dry-run-first lifecycle forms.

### Security

- Authentication, sessions, state, logs, plugins, provider endpoints, and provider credentials are excluded from clones.
- Capability clones skip symlinks and common credential filenames.

## [0.1.0-alpha.1]

### Added

- Initial Rust workspace.
- Public-safe repository policy and product roadmap.
- Working `scan`, `inspect`, and `doctor` CLI commands with human and JSON output.
- Safe discovery for default, explicit, launcher-managed, CodexUse, and managed-agent Homes.
- Versioned discovery and Agent Household schemas.
- Secret-redaction, malformed-config, help, JSON-stream, and exit-code tests.
