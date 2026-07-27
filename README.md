# CodexHome Manager

[![CI](https://github.com/harzva/codexhome-manager/actions/workflows/ci.yml/badge.svg)](https://github.com/harzva/codexhome-manager/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-176b50.svg)](LICENSE)
[![Version](https://img.shields.io/badge/version-0.3.0--alpha.1-8a5a20.svg)](RELEASE_NOTES.md)

**Compose one Codex runtime from identity, expertise, and project context.**

CodexHome Manager is the public-safe control plane for Account Profiles, reusable Expert Packs, Project Bindings, Skill Registry entries, and hard-isolated Codex runtimes. Existing `CODEX_HOME` directories remain supported through an explicit `LegacyHome` compatibility layer.

> Status: v0.3 alpha. Account + Expert + Project resolution and real Runtime Projection are available alongside discovery, durable Agent Runs, observability, routing, worktree gates, Scheduler primitives, and the Tauri Desktop adapter. Process launch and remote package distribution remain roadmap work.

## Why

A single Codex Home can mix identity, credentials, hundreds of Skills, MCP servers, Rules, and project instructions. CodexHome Manager separates those concerns:

```text
Account Profile       Expert Pack          Project Binding
auth/provider/model + research capabilities + project-local rules
          \                 |                 /
           +------ Effective Runtime -------+
```

This avoids copying one complete directory for every account × expert × project combination.

## Current capabilities

- Discover the default and explicitly configured `CODEX_HOME` directories.
- Discover common launcher-managed Homes on macOS.
- Inspect an allowlisted set of safe configuration fields.
- Count Skills, MCP servers, enabled features, Rules, and Hooks.
- Report whether auth/config files exist without reading credential contents.
- Produce human-readable or stable JSON output.
- Diagnose missing auth, missing config, and inspection warnings.
- Persist unique `@alias` values, family labels, and Unicode specialty tags.
- Create new Homes, import existing Homes, and safely clone registered Homes.
- Preview every lifecycle mutation with `--dry-run`.
- Record append-only task/run/attempt/thread observability events.
- Aggregate token, cache, duration, retry, failure, cost, and Home health metrics.
- Export filtered observability events as versioned JSON or analysis-ready CSV.
- Create durable Tasks and Agent Runs without storing prompts or model responses.
- Track retries and Home/model migrations under one `run_id`.
- Enforce token, duration, cost, and attempt budgets before starting another attempt.
- Attribute failed-attempt cost separately from successful work.
- Link threads, tool calls, opaque artifacts, verification evidence, and final artifacts.
- Rank Home/account/model candidates with hard constraints and nine itemized score components.
- Use live quota, health, rate-limit, historical success, duration, cost, and active-load evidence.
- Lock a Home or model and record immutable candidate snapshots in the Agent Run event chain.
- Evaluate and append Run route decisions under one lock with reproducible event-count provenance.
- Create one unique `codex/` branch and isolated Git worktree per Run.
- Capture committed patch/test evidence with SHA-256, duration, and clean-tree state.
- Require a different Home to approve the latest evidence before Run completion.
- Detect target-branch conflicts and route them to a human or explicit replan.
- Project a durable Scheduler queue from the same append-only Run event stream.
- Dispatch atomically with priority, dependencies, time windows, budgets, concurrency limits, Route evidence, and leases.
- Defer unavailable, rate-limited, quota-exhausted, or unhealthy Homes with explainable fallback evidence.
- Renew long-task leases and require explicit recovery before retrying expired work.
- Resolve `AccountProfile + ExpertPack[] + ProjectBinding` into a deterministic Effective Runtime Manifest.
- Preserve old registries through `LegacyHome` without extending the legacy contract.
- Register versioned Skills by logical ID, source, digest, and context estimate.
- Plan, apply, verify, and clean Skill projections under manager-owned roots.
- Link Account auth/config into hard runtimes without reading or copying credential contents.
- Force Expert Packs that require isolation, including Agent Households, into separate runtime directories.
- Report static Skill catalog/body, AGENTS, Rules, and tool-schema context estimates.

## Five-minute quickstart

Requirements:

- Rust 1.85 or newer.
- A local Codex installation is optional for discovery tests.

```bash
git clone https://github.com/harzva/codexhome-manager.git
cd codexhome-manager
cargo install --path crates/codexhome-cli
codexhome scan
```

Register one discovered Home without changing it:

```bash
codexhome home import /absolute/path/to/home \
  --alias @research \
  --label "Research Family" \
  --specialty papers \
  --dry-run
```

The dry-run prints the exact registry action and warnings. Repeat without `--dry-run` only after reviewing the plan.

## Commands

```bash
codexhome scan
codexhome scan --json
codexhome inspect <id-or-label-or-path>
codexhome doctor
codexhome doctor --json
codexhome registry list
codexhome registry show @frontend
codexhome registry path
codexhome home create @frontend --path /path/to/frontend --specialty ui
codexhome home import /path/to/research --alias @research --specialty papers
codexhome home clone @frontend @reviewer --path /path/to/reviewer --dry-run
codexhome observe record events.jsonl
codexhome observe summary --home-id @research --json
codexhome observe verify
codexhome observe export --format csv --output events.csv
codexhome task create --label "Compile benchmark" --kind coding
codexhome run start <task-id> --max-total-tokens 300000 --max-duration-ms 5400000
codexhome run attempt start <run-id> --home-id home-main --model gpt-5.5
codexhome run show <run-id> --json
codexhome route validate --route-policy router-policy.json --json
codexhome route recommend route-request.json --route-policy router-policy.json --json
codexhome route decide route-request.json --run-id <run-id> --route-policy router-policy.json --json
# First replace the example runId with an active Run created above.
codexhome schedule enqueue examples/scheduler-job.example.json
codexhome schedule list --json
codexhome schedule dispatch \
  --scheduler-policy examples/scheduler-policy.example.json \
  --route-policy examples/router-policy.example.json \
  --json
codexhome schedule tick --json
codexhome schedule policy validate \
  --scheduler-policy examples/scheduler-policy.example.json \
  --json
codexhome run worktree prepare <run-id> <attempt-id> --repository /path/to/repo --dry-run
codexhome run worktree evidence <run-id> <attempt-id> --test-label tests --test-program cargo -- test --workspace
codexhome run worktree review <run-id> <attempt-id> <evidence-id> --decision approved --reason reviewed --home-id home-review --model reviewer-model
codexhome run worktree conflict-check <run-id> <attempt-id> <evidence-id> --target-ref main --home-id home-review --model reviewer-model
codexhome environment resolve examples/project-binding.example.json \
  --account-profile examples/account-profile.example.json \
  --expert-pack examples/expert-pack.example.json \
  --skill-registry examples/skill-registry.example.json \
  --runtime-root /absolute/path/to/runtimes \
  --output runtime.json --json
codexhome projection plan runtime.json --json
codexhome projection apply runtime.json --dry-run --json
codexhome projection apply runtime.json --json
codexhome projection verify runtime.json --json
codexhome environment context-inspect examples/runtime-context-snapshot.example.json --json
codexhome skill digest /absolute/path/to/skill --json
codexhome skill validate examples/skill-registry.example.json --json
```

Clone Skills, Rules, and Hooks only after reviewing the source Home:

```bash
codexhome home clone @frontend @frontend-copy \
  --path /path/to/frontend-copy \
  --copy-capabilities \
  --dry-run
```

Add extra Home candidates without changing your shell's active `CODEX_HOME`:

```bash
codexhome --home /path/to/another-home scan
```

Or use a platform path list:

```bash
CODEXHOME_PATHS="/path/one:/path/two" codexhome scan
```

Precedence and discovery order:

1. Default `~/.codex`.
2. Current `CODEX_HOME`.
3. Repeated `--home` arguments.
4. `CODEXHOME_PATHS`.
5. Known launcher-managed locations for the current platform.

Registry path precedence is `--registry`, then `CODEXHOME_REGISTRY`, then `~/.codexhome/registry.json`. See [docs/registry.md](docs/registry.md).

## JSON contract

`--json` writes only JSON to stdout. Progress and diagnostics must never corrupt the JSON stream. Reports exclude credential values but include local Home paths and provider hostnames, so review them before sharing publicly.

The v0.3 environment boundary is `codexhome.account-profile.v1`, `codexhome.expert-pack.v1`, `codexhome.project-binding.v1`, `codexhome.skill-registry.v1`, `codexhome.effective-runtime.v1`, and `codexhome.runtime-projection.v1`. Existing v0.2 registry, observability, Agent Run, route, Scheduler, and worktree contracts remain supported.

Agent Run and Scheduler state are projected from the same append-only observability stream, so queue state, cost analysis, and lifecycle state cannot silently drift. The stream excludes prompts, responses, credential values, arbitrary payloads, raw tool arguments, and artifact paths. See [docs/observability.md](docs/observability.md), [docs/agent-runs.md](docs/agent-runs.md), and [docs/scheduler.md](docs/scheduler.md).

## Security

CodexHome Manager never reads the contents of `auth.json`. Hard runtimes may link the selected Account Profile's auth/config files, but Project Bindings and Expert Packs cannot contain or copy account credentials.

Third-party provider execution, Skill installation, MCP changes, Hooks, and child-agent delegation will require explicit policy and confirmation in later milestones.

See [SECURITY.md](SECURITY.md) and [ROADMAP.md](ROADMAP.md).

The core trust boundaries and planned Household execution model are described in [docs/architecture.md](docs/architecture.md). The stable adapter boundary used by the desktop UI is in [docs/desktop-api.md](docs/desktop-api.md).

## Desktop UI

The Tauri desktop adapter lives in `apps/desktop` and calls `codexhome-core` directly. It provides Home cards, visible inactive/error states, alias and specialty search, and dry-run-first Create/Import/Clone forms.

![CodexHome Manager desktop](assets/desktop-v0.2.png)

Preview source: [apps/desktop/src/main.ts](apps/desktop/src/main.ts) and [apps/desktop/src/styles.css](apps/desktop/src/styles.css).

```bash
cd apps/desktop
npm install
npm run tauri dev
```

For frontend-only development, use `npm run dev`. See [docs/troubleshooting.md](docs/troubleshooting.md) when platform or dependency setup fails.

## Documentation

- [Architecture and trust boundaries](docs/architecture.md)
- [Effective environment and runtime projection](docs/effective-environment.md)
- [Registry format and legacy lifecycle semantics](docs/registry.md)
- [Desktop adapter contract](docs/desktop-api.md)
- [Observability event and metric contract](docs/observability.md)
- [Agent Run lifecycle and recovery contract](docs/agent-runs.md)
- [Explainable policy router](docs/policy-router.md)
- [Scheduler v1 queue, lease, fallback, and recovery contract](docs/scheduler.md)
- [Troubleshooting](docs/troubleshooting.md)
- [Product readiness audit](docs/product-readiness-v0.2.md)
- [Security and privacy audit](docs/security-audit-v0.2.md)
- [Roadmap](ROADMAP.md)
- [Release notes](RELEASE_NOTES.md)

## Contributing and support

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request and [SUPPORT.md](SUPPORT.md) before filing an issue. Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md).

## Name and trademark

The repository and CLI use the descriptive name CodexHome Manager. This is an independent community project and is not affiliated with or endorsed by OpenAI. Codex is a trademark of its respective owner.

## License

MIT
