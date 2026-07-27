# Effective Codex Environment

CodexHome Manager composes three independent concerns:

```text
Account Profile + Expert Pack(s) + Project Binding
                         |
                         v
            Effective Runtime Manifest
                         |
                         v
               Runtime Projection
```

## Account Profile

An Account Profile selects auth, provider, model, endpoint host, and base config. Reports expose only safe metadata and whether auth/config files exist. They never serialize credential contents.

## Expert Pack

An Expert Pack is a reusable capability declaration. It references Skills by logical registry ID and may declare Rules, AGENTS files, MCP server IDs, capabilities, static context estimates, and a hard-isolation requirement. It is safe to combine with different accounts and projects.

## Project Binding

A Project Binding selects one account and zero or more Expert Packs. Its optional Project Home layer may add only project-local Skills, Rules, AGENTS files, MCP declarations, and context estimates. Project-local paths must stay under `projectRoot`, and `auth.json` is rejected.

When no project-specific layer exists, `hasProjectHome` is false. Clients should not display a separate Project Home card in that case.

## Runtime Projection

`environment resolve` produces a deterministic manifest. `projection plan`, `apply`, `verify`, and `clean` are the only supported filesystem projection path.

- Shared mode projects Skills into `<project>/.agents/skills` and keeps the selected Account Home active.
- Hard mode creates a manager-owned runtime directory, projects Skills there, and links account `config.toml` and `auth.json` without reading or copying them.
- `--mode copy` applies only to Skill directories. Account auth/config are always symlink references and are never copied.
- A required missing Skill fails closed.
- Every existing Skill source is hashed deterministically (relative paths, entry kinds, file lengths/content, and symlink targets) and must match its Registry digest before projection.
- Targets outside the manifest's managed root are rejected.
- Apply rolls back entries it created when a later operation fails.

Agent Households and any Expert Pack with `requiresHardIsolation=true` always resolve to hard mode.

## Context diagnostics

The manifest carries a static estimate for Skill catalog entries, activated Skill bodies, AGENTS files, Rules, and tool schemas. Static estimates are suitable for Home cards.

Run Inspector must keep runtime measurements separate: actual loaded Skill IDs and body tokens, conversation history, cached/reused tokens, context-window capacity, total tokens, and percentage. A large registry count never implies every Skill body entered context.

Launch adapters can emit `codexhome.runtime-context-snapshot.v1`. Validate and
classify it with:

```bash
codexhome environment context-inspect runtime-context.json --json
```

Pressure thresholds are `<1%` healthy, `1%–2%` notice, `>2%–5%` warning, and
`>5%` critical.
