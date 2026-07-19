# Home Registry

The registry assigns a stable local identity, a unique `@alias`, a human label, and specialty tags to each managed Home. It is local runtime state, not a public manifest.

## Location and precedence

1. Global `--registry <FILE>` CLI flag.
2. `CODEXHOME_REGISTRY` environment variable.
3. `~/.codexhome/registry.json` using `HOME` on macOS/Linux or `USERPROFILE` on Windows.

The parent directory is mode `0700` and registry/lock files are mode `0600` on Unix. Writes use an exclusive lock, revision check, temporary file, sync, and atomic replacement.

## Entry contract

- `id`: 12-character path-derived identifier.
- `alias`: unique lowercase routing name such as `@frontend`.
- `label`: human-facing household name.
- `path`: canonical local Home path.
- `specialties`: normalized, unique clustering tags; Unicode tags are supported.
- `origin`: `created`, `imported`, or `cloned`.
- `derivedFrom`: source Home ID for clones.

The JSON contract is [../schemas/registry.schema.json](../schemas/registry.schema.json).

## Lifecycle semantics

- `create` makes a minimal valid Home with `config.toml`, `skills`, `rules`, and `hooks`, then registers it.
- `import` validates and registers an existing Home without changing its files.
- `clone` creates a new Home from safe allowlisted settings. Authentication, provider configuration, sessions, databases, logs, and plugins are never copied.
- `clone --copy-capabilities` additionally copies Skills, Rules, and Hooks. Symlinks and common credential filenames are skipped, with a limit of 20,000 files or 512 MiB.
- `--dry-run` validates aliases, paths, conflicts, and clone size without writing the Home or registry.

Create and clone roll back newly created destination directories if filesystem copying or registry commit fails. Import has no source-side mutation to roll back.
