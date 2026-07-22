# Troubleshooting

## No Homes are discovered

Pass an explicit candidate without changing the active shell:

```bash
codexhome --home /absolute/path/to/CODEX_HOME scan
```

The directory must contain a recognizable Codex artifact such as `config.toml`, `auth.json`, `sessions`, `skills`, or `state_5.sqlite`.

## A Home cannot be imported

Run a dry-run and use an absolute path:

```bash
codexhome home import /absolute/path/to/home --alias @research --dry-run
```

Aliases contain 2–32 lowercase ASCII letters, digits, or hyphens, start with a letter, and may be entered with or without `@`.

## Clone does not include authentication or provider settings

This is intentional. Authentication, provider endpoints, sessions, databases, logs, and plugins never cross the Home boundary. Configure the provider separately inside the clone.

## Registry JSON is invalid

Do not edit the registry while CodexHome Manager is running. Keep the damaged file for diagnosis, redact local paths before sharing it, and point the CLI at a clean temporary registry to recover:

```bash
codexhome --registry /absolute/path/to/recovery.json registry list
```

## Desktop dependencies fail to resolve

The Tauri shell requires access to crates.io and platform build tools. The browser frontend can still be checked independently:

```bash
cd apps/desktop
npm ci
npm run build
```

Then retry `npm run tauri dev` after Rust dependencies and platform prerequisites are available.
