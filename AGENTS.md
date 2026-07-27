# CodexHome Manager Repository Rules

## Scope

This repository is the generic, public-safe implementation of CodexHome Manager. It must not depend on Harvis, Harzva private memory, or machine-specific credentials.

## Product boundaries

- An Account Profile owns auth, provider, model, endpoint, and base config only.
- An Expert Pack owns reusable Skills, Rules, AGENTS instructions, MCP declarations, and context estimates.
- A Project Binding may add project-local capability overrides, but never credentials.
- An Effective Runtime Manifest is the only authoritative resolved Account + Expert + Project view.
- A legacy `CODEX_HOME` is supported through `LegacyHome`; new features must not extend the legacy shape.
- An Agent Household is orchestration over one or more Expert Packs and always uses a hard-isolated runtime.
- The manager discovers, validates, projects, launches, and delegates; it does not copy authentication material between Homes.
- Public manifests may contain capability metadata and local reference placeholders, never credentials or private local paths.

## Safety

- Never read or print the contents of `auth.json`, token files, cookies, `.env` files, or credential stores.
- Configuration inspection must use an allowlist of safe keys.
- Destructive actions require an explicit command and confirmation or `--force`.
- Skill, MCP, and Hook installation must support dry-run, explicit placement confirmation, verification, and rollback.
- Child agents default to isolated worktrees and a maximum orchestration depth of two.

## Engineering

- Keep `codexhome-core` independent of the CLI and future desktop UI.
- Human output and `--json` output are separate stable contracts.
- Add tests for happy paths, malformed configuration, missing paths, and secret redaction.
- Prefer additive schema evolution and version every serialized contract.
- Runtime Projection must be dry-run-first, target only manager-owned roots, and never copy authentication material.
- Keep macOS, Linux, and Windows behavior explicit; do not silently replace one platform path with another.

## Publishing

Before publishing, check tracked files for tokens, cookies, `.env` values, credential dumps, raw sessions, and private local paths. Do not create or push a remote without explicit user approval.
