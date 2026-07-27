# Architecture

CodexHome Manager separates account identity from reusable expertise and project-local capability. The filesystem remains the source of truth; `codexhome-core` is the sole authority that resolves those layers into an immutable runtime contract.

## Layers

```text
Desktop UI / CLI / Harvis CAM
              |
              v
        codexhome-core
              |
    +---------+---------+
    v         v         v
 Account   Expert     Project
 Profile   Pack(s)    Binding
    +---------+---------+
              v
 Effective Runtime Manifest
              |
              v
 Runtime Projection / isolated workers
```

- `codexhome-core` contains discovery, legacy registry support, environment resolution, Runtime Projection, the append-only event store, and deterministic Agent Run projection without terminal concerns.
- `codexhome-cli` is the first adapter. It keeps JSON output clean and exposes useful process exit codes.
- Tauri Desktop and Harvis CAM consume Core models or versioned Core JSON. They must not independently infer account/expert/project semantics.

## Environment layers

1. `AccountProfile` owns `auth.json`, provider, model, endpoint, and base config. It never owns project Skills.
2. `ExpertPack` owns reusable Skills, Rules, AGENTS instructions, MCP declarations, and static context estimates. It contains no account credentials.
3. `ProjectBinding` selects one Account Profile and zero or more Expert Packs, then optionally adds project-local capabilities.
4. `EffectiveRuntimeManifest` is a deterministic snapshot of the resolved environment. Adapters launch and display this snapshot rather than merging layers themselves.
5. `LegacyHome` wraps old registry entries so existing users can migrate without breaking v0.2 workflows.

Projects without a project-local Home set `hasProjectHome=false`; clients should show only the default Account/Expert environment instead of inventing an empty project Home card.

## Agent Run state

`Task -> Run -> Attempt -> Thread/Tool/Artifact/Verification` is an event-sourced lifecycle. The append-only observability JSONL file is the fact store; the Agent Run report is rebuilt by deterministic replay. Mutations take one exclusive file lock, reload current facts, validate budgets and transition invariants, then append and sync before returning.

A retry or migration creates another Attempt under the same `run_id`. This preserves successful cost, failed-attempt cost, route reasons, and final artifact lineage without conflating a Codex conversation with the task itself.

## Trust boundaries

1. An Account Profile owns its credentials. Credentials are never copied to an Expert Pack, Project Binding, report, or event log.
2. Discovery may read `config.toml`, but it emits only an allowlist of non-secret fields. Provider URLs are reduced to hostnames.
3. Discovery checks only whether `auth.json` exists; it never opens that file.
4. Runtime Projection is dry-run-first, verifies every target, and cleans only paths inside the manifest's manager-owned root.
5. Hard runtimes link selected Account auth/config files without opening them. Skill copy mode rejects nested symlinks; symlink mode is the default.
6. Household delegation is limited to L0 (user/main), L1 (Expert Pack runtime), and L2 (temporary members). L3 creation is denied by default.

## Household execution

An L1 Household receives a goal rather than a fixed worker count. It references Expert Packs and always receives a hard-isolated Effective Runtime with separate projected Skills, session state, token/cost attribution, and runtime root. Planner, Executor, Tester, and Reviewer members return artifacts and evidence; only reviewed results flow back to the parent.

## Routing

Routing selects an Account Profile, Expert Pack set, and Project Binding. Resolution checks capability fit, provider and cost policy, workspace permissions, recursion depth, isolation requirements, and user confirmation requirements before starting work. The caller receives a result envelope and runtime manifest identity, not credentials or private runtime state.
