# CodexHome Manager

CodexHome Manager turns multiple `CODEX_HOME` directories into discoverable Skill Spaces and specialized Agent Households.

> Status: v0.2 alpha. Discovery, the persistent Home registry, aliases, specialty tags, and safe Home lifecycle commands work. Delegation, Skill placement, MCP routing, and the desktop app are roadmap work.

## Why

A single Codex Home can accumulate too many Skills, MCP servers, Rules, Hooks, providers, and sessions. CodexHome Manager keeps these capabilities separated:

```text
Main Home
├─ @frontend  UI Skills + browser/Figma MCP
├─ @research  research and data Skills
├─ @reviewer  read-only review Skills
└─ @ops       CI and release Skills
```

The main Home can stay small and delegate work to a specialized Home when needed.

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

## Install for development

Requirements:

- Rust 1.85 or newer.
- A local Codex installation is optional for discovery tests.

```bash
git clone https://github.com/harzva/codexhome-manager.git
cd codexhome-manager
cargo install --path crates/codexhome-cli
```

Run without installing:

```bash
cargo run -p codexhome-cli -- scan
```

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

The discovery schema is `codexhome.discovery.v1`; v0.2 adds `codexhome.registry.v1`, `codexhome.registry-report.v1`, and `codexhome.home-mutation.v1`.

## Security

CodexHome Manager never reads the contents of `auth.json`. It parses `config.toml` locally, retains only allowlisted non-secret fields in its report, and reduces provider URLs to hostnames.

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

## Name and trademark

The repository and CLI use the descriptive name CodexHome Manager. This is an independent community project and is not affiliated with or endorsed by OpenAI. Codex is a trademark of its respective owner.

## License

MIT
