# Release Notes

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
