# Roadmap

The detailed product roadmap was developed before repository initialization. This repository tracks the implementation in smaller release milestones.

## v0.1 — Discovery foundation ✅

- Cross-platform Home discovery.
- Safe config inspection.
- Human and JSON output.
- `scan`, `inspect`, and `doctor` commands.
- Public schemas and redaction tests.

## v0.2 — Registry and Skill Spaces ✅

- Agent aliases and capability summaries.
- Home creation, import, clone, and validation.
- Atomic user-level registry with revision numbers.
- Family labels, Unicode specialty tags, dry-run, and rollback.
- Desktop read/mutation contracts.
- Tauri Desktop UI connected directly to `codexhome-core`.

## v0.3 — Skill placement

- Skill Space manifests and capacity budgets.
- Shared Skill Pack references and lockfile.
- Skill Classifier.
- Hard-constraint validation.
- Top-three Home recommendations with explainable scores.
- Dry-run, explicit placement confirmation, verification, and rollback.
- Unclassified inbox and new-Home recommendation.

## v0.4 — Execution engine

- `codexhome run @agent`.
- Isolated worktrees.
- JSONL event capture, final result, Diff, and test evidence.
- Timeout, cancellation, retry, and review states.

## v0.5 — Agent Household

- Household Manifest and reusable member templates.
- Complexity-based Member Plans.
- Planner, Executor, Tester, and Reviewer members.
- L0/L1/L2 depth limit, Budget Guard, and Artifact Aggregator.

## v0.6 — MCP delegation

- Agent listing and inspection tools.
- Delegate, cancel, status, result, Diff, and accept tools.
- Parent review before merge.
- No credential propagation between Homes.

## v0.7 — `@agent` routing

- Alias registry and mention routing.
- Third-party provider confirmation.
- Cross-Household policy and auditable call chains.

## v0.8 — Desktop manager

- Home and Agent cards.
- Config, Skills, MCP, Rules, and Hooks management.
- Skill Placement Inbox.
- Run history, Household members, budgets, and privacy controls.

## v1.0 — Public release

- Signed packages and reproducible builds.
- Stable schemas and migration tooling.
- Home, Agent, Household, Skill Pack, and workflow templates.
- Release safety checks and public documentation.
