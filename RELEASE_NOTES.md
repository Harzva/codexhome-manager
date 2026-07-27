# Release Notes

## 0.3.0-alpha.1

CodexHome Manager now composes account identity, reusable expertise, and project-local capability without cloning a complete `CODEX_HOME` for every combination.

Highlights:

- `AccountProfile`, `ExpertPack`, `ProjectBinding`, `LegacyHome`, and `EffectiveRuntimeManifest` in `codexhome-core`;
- Skill Registry entries with logical IDs, versions, source digests, and context estimates;
- `environment resolve` plus Runtime Projection plan/apply/verify/clean commands;
- hard-isolated runtimes for Agent Households and isolation-required Expert Packs;
- Account auth/config links without credential reads or copies;
- project-local auth rejection, manager-owned target checks, rollback, and required-Skill fail-closed behavior;
- bounded worktree test execution with a configurable timeout.

Alpha limits:

- process launch adapters and remote Expert/Skill package distribution are not included;
- runtime context snapshots depend on launch adapters reporting actual loaded Skills and token composition;
- no signed binary packages are published yet.

## 0.2.0-alpha.1

CodexHome Manager now turns discovered `CODEX_HOME` directories into a persistent registry of specialized Agent Households.

Highlights:

- unique `@alias` routing names, labels, and Unicode specialty tags;
- safe Home create, import, clone, dry-run, and rollback;
- human-readable and versioned JSON output;
- a Tauri Desktop UI connected directly to `codexhome-core`;
- credential-excluding clone policy and public JSON Schemas;
- append-only execution observability with JSON/CSV export;
- durable Tasks and Agent Runs with retry/migration lineage;
- token, duration, cost, and Attempt budgets plus failed-attempt cost;
- thread, opaque artifact, verification, and final-artifact tracking.

Alpha limits:

- Agent Run state is available, but process delegation and `@alias` execution routing are roadmap work;
- Skill placement, MCP management, and provider setup are not yet automated;
- no signed binary packages are published yet;
- discovery reports contain local paths and should be reviewed before sharing.

See [CHANGELOG.md](CHANGELOG.md) for the detailed change list and [SECURITY.md](SECURITY.md) for trust boundaries.
