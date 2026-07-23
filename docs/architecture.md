# Architecture

CodexHome Manager treats a `CODEX_HOME` as both a **Skill Space** and an **Agent Household**. The filesystem remains the source of truth; the manager adds discovery, safe metadata, policy, routing, and execution evidence around it.

## Layers

```text
Desktop UI / CLI / MCP
          │
          ▼
Registry, run state, and policy engine
          │
    ┌─────┼──────────────┐
    ▼     ▼              ▼
Discovery Skill router   Task router
    │     │              │
    ▼     ▼              ▼
CODEX_HOME directories   isolated workers
```

- `codexhome-core` contains discovery, the atomic registry, safe Home lifecycle logic, the append-only event store, and deterministic Agent Run projection without terminal concerns.
- `codexhome-cli` is the first adapter. It keeps JSON output clean and exposes useful process exit codes.
- Worktree placement, process execution, policy routing, MCP, and detailed Desktop Run adapters arrive in later milestones.

The v0.2 registry is the shared boundary between CLI and the future Desktop UI. Adapters call core operations or versioned JSON commands; they do not edit the registry directly.

## Agent Run state

`Task -> Run -> Attempt -> Thread/Tool/Artifact/Verification` is an event-sourced lifecycle. The append-only observability JSONL file is the fact store; `codexhome agent-runs.v1` is rebuilt by deterministic replay. Mutations take one exclusive file lock, reload current facts, validate budgets and transition invariants, then append and sync before returning.

A retry or migration creates another Attempt under the same `run_id`. This preserves successful cost, failed-attempt cost, route reasons, and final artifact lineage without conflating a Codex conversation with the task itself. The projection is disposable and never becomes a competing source of truth.

## Trust boundaries

1. A Home owns its own credentials. Credentials are never copied to a parent, sibling, child member, report, or event log.
2. Discovery may read `config.toml`, but it emits only an allowlist of non-secret fields. Provider URLs are reduced to hostnames.
3. Discovery checks only whether `auth.json` exists; it never opens that file.
4. Home lifecycle mutations support dry-run and rollback; future Skill/MCP/Hook mutations must add explicit confirmation and post-write verification.
5. Household delegation is limited to L0 (user/main), L1 (specialized Home), and L2 (temporary members). L3 creation is denied by default.

## Household execution model

An L1 Household receives a goal rather than a fixed worker count. Its policy engine may select zero or more reusable L2 member templates—for example Planner, Executor, Tester, and Reviewer—based on complexity, risk, cost, and concurrency budgets. Members return artifacts and evidence to the Household; only reviewed results flow back to the main Home.

## Planned routing

`@frontend`, `@research`, or another alias resolves to a registry entry. Resolution checks capability fit, provider and cost policy, workspace permissions, recursion depth, and user confirmation requirements before starting an isolated task. The calling Home receives a result envelope, not the callee's credentials or private runtime state.
