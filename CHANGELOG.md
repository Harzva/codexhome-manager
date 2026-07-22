# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added

- Three-platform Rust CI plus Desktop frontend and macOS Tauri bridge checks.
- Dependabot for Cargo, npm, and GitHub Actions dependencies.
- Public contribution, support, troubleshooting, release, and audit documentation.
- Structured issue and pull request templates with credential-safety reminders.

### Security

- CI uses a read-only token and pinned external actions.
- GitHub private vulnerability reporting replaces public security issues.

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
