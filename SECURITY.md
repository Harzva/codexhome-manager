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

## Home lifecycle boundary

- Import never changes source Home files.
- Create never creates or copies authentication.
- Clone copies only allowlisted non-provider settings by default.
- `--copy-capabilities` is explicit and limited to Skills, Rules, and Hooks; symlinks and common credential filenames are skipped.
- Clone never copies `auth.json`, sessions, state databases, logs, plugins, provider endpoints, or provider credentials.
- Dry-run performs validation and size inspection without changing the Home or registry.

## Execution boundary

Current Agent Run state:

- stores only bounded labels, identities, route reasons, failure reasons, and opaque evidence IDs;
- rejects obvious credential patterns and undeclared local paths in events;
- records no prompts, responses, environment dumps, raw tool arguments, or artifact paths;
- uses an owner-only event file and only changes permissions on directories it creates;
- validates transitions and budgets under one exclusive append lock.

Current policy routing:

- accepts bounded task metadata, capability labels, cost estimates, and opaque identity labels;
- never stores prompts, source text, sensitive directory paths, or credential values;
- requires `sensitiveDirectory` requests to lock an explicit Home;
- treats unavailable Homes, invalid auth, exhausted quota, active rate limits, and missing security domains as hard constraints;
- records the policy revision, complete candidate score snapshot, and rejection reasons in the same owner-only event stream;
- rejects an Attempt that links a route decision but changes its selected Home, account, or model.

Future process execution features must:

- use explicit target Homes;
- show third-party provider destinations;
- default code-writing tasks to isolated worktrees;
- enforce depth, concurrency, time, token, and cost budgets;
- require parent or user review before accepting changes;
- prevent child agents from reading another Home's auth files.

## Reporting a vulnerability

Use [GitHub private vulnerability reporting](https://github.com/harzva/codexhome-manager/security/advisories/new). Do not open a public issue for a vulnerability and do not include real credentials, private sessions, or sensitive local paths in public discussion.

If a real credential was committed or posted, revoke or rotate it immediately. Removing it from the latest commit is not sufficient because Git history and notification copies may retain it.

## Data retention and telemetry

CodexHome Manager has no telemetry, analytics, hosted service, or remote log upload. The registry remains on the local machine until the user deletes it. Create and clone roll back newly created destinations after a failed registry commit; imported Homes remain owned by the user and are never deleted by import.
