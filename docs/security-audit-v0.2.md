# Security and Privacy Audit — v0.2.0-alpha.1

## Scope

Reviewed the Rust Core/CLI, local registry, Home lifecycle, Tauri command bridge, browser frontend, lockfiles, documentation, and GitHub workflows. CodexHome Manager handles local paths and configuration metadata; it detects but does not open `auth.json`, has no telemetry, and performs no hosted processing.

Score: **62/70 — release-ready posture for a public early-alpha repository**.

| Factor | Score / 5 | Evidence |
|---|---:|---|
| Asset inventory | 4 | Credentials, config, sessions, registry, filesystem mutations documented |
| Secret hygiene | 5 | Auth exclusion, ignored local state, staged secret/path scans |
| Configuration safety | 5 | Local defaults, allowlisted output, provider configuration not cloned |
| Permission boundaries | 5 | Explicit capability copy, dry-run, CI `contents: read` |
| Input validation | 5 | Alias/path/tag checks, clone limits, traversal normalization |
| Output and log safety | 4 | Secrets excluded; local paths explicitly disclosed |
| Data handling | 5 | No telemetry; local retention and rollback documented |
| Dependency hygiene | 4 | Rust/npm lockfiles, npm audit, Dependabot; Rust audit automation pending |
| CI/CD security | 5 | Explicit triggers, no secrets, least privilege, pinned actions |
| Agent and tool safety | 4 | L0/L1/L2 limit and future execution guardrails documented |
| MCP and integration safety | 3 | No active MCP delegation yet; future boundary documented |
| RAG and knowledge safety | 3 | No RAG surface; private/public boundary established |
| Release and disclosure | 5 | MIT, SECURITY, private reporting, security release notes |
| Validation evidence | 5 | Tests, Clippy, npm audit, Semgrep/manual scans, CI policy review |

## Trust boundaries

| Boundary | Control |
|---|---|
| Home authentication → manager | Existence check only; contents never opened or serialized |
| Source Home → clone | Safe configuration projection; credentials, sessions, databases, logs, plugins excluded |
| Registry data → Desktop DOM | Dynamic strings are HTML-escaped; CSP restricts network and script sources |
| Pull request → CI | Read-only token, no repository secrets, no publishing step |

## Remaining risks

- `--copy-capabilities` copies user-approved source files after name and symlink filtering; content-aware secret detection is not guaranteed.
- Reports include local paths and provider hostnames and require review before public sharing.
- Signed/notarized Desktop packages are not yet available.
