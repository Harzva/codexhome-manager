# Security Policy

## Sensitive data boundary

CodexHome Manager may detect authentication files, but it must not read, copy, serialize, log, or display their contents. It parses `config.toml` locally to extract an explicit allowlist and must never retain or emit credential values found in that file.

Sensitive examples include:

- `auth.json` contents.
- API keys, bearer tokens, cookies, refresh tokens, and passwords.
- `.env` values and credential dumps.
- Raw Codex sessions or private conversation history.

## Configuration inspection

The scanner uses an allowlist. It may report model name, provider display name, endpoint hostname, reasoning effort, approval policy, sandbox mode, Web Search mode, and capability counts. Unknown configuration fields are ignored.

Discovery output includes local Home paths and may include provider hostnames. Treat reports as local diagnostic data and review them before posting publicly.

## Execution boundary

Future execution features must:

- use explicit target Homes;
- show third-party provider destinations;
- default code-writing tasks to isolated worktrees;
- enforce depth, concurrency, time, token, and cost budgets;
- require parent or user review before accepting changes;
- prevent child agents from reading another Home's auth files.

## Reporting a vulnerability

Do not include real credentials, private sessions, or sensitive local paths in a public report. Open a minimal redacted issue after a private reporting channel is published.
