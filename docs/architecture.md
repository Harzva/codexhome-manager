# Architecture

CodexHome Manager treats a `CODEX_HOME` as both a **Skill Space** and an **Agent Household**. The filesystem remains the source of truth; the manager adds discovery, safe metadata, policy, routing, and execution evidence around it.

## Layers

```text
Desktop UI / CLI / MCP
          │
          ▼
Registry and policy engine
          │
    ┌─────┼──────────────┐
    ▼     ▼              ▼
Discovery Skill router   Task router
    │     │              │
    ▼     ▼              ▼
CODEX_HOME directories   isolated workers
```

- `codexhome-core` contains discovery and redacted inspection logic without terminal concerns.
- `codexhome-cli` is the first adapter. It keeps JSON output clean and exposes useful process exit codes.
- Registry, placement, execution, MCP, and desktop adapters arrive in later milestones.

## Trust boundaries

1. A Home owns its own credentials. Credentials are never copied to a parent, sibling, child member, report, or event log.
2. Discovery may read `config.toml`, but it emits only an allowlist of non-secret fields. Provider URLs are reduced to hostnames.
3. Discovery checks only whether `auth.json` exists; it never opens that file.
4. Any future mutation must support preview, explicit confirmation, verification, and rollback.
5. Household delegation is limited to L0 (user/main), L1 (specialized Home), and L2 (temporary members). L3 creation is denied by default.

## Household execution model

An L1 Household receives a goal rather than a fixed worker count. Its policy engine may select zero or more reusable L2 member templates—for example Planner, Executor, Tester, and Reviewer—based on complexity, risk, cost, and concurrency budgets. Members return artifacts and evidence to the Household; only reviewed results flow back to the main Home.

## Planned routing

`@frontend`, `@research`, or another alias resolves to a registry entry. Resolution checks capability fit, provider and cost policy, workspace permissions, recursion depth, and user confirmation requirements before starting an isolated task. The calling Home receives a result envelope, not the callee's credentials or private runtime state.
