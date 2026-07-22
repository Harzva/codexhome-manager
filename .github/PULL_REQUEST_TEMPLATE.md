## Summary

Describe the user-visible change and why it belongs in CodexHome Manager.

## Validation

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] Desktop changes: `npm ci && npm run build`
- [ ] I reviewed changed files for credentials, private paths, raw sessions, and generated local state.

## Safety boundary

Explain any effect on authentication, configuration, file copying, provider routing, Skills, MCP, Rules, Hooks, or Agent delegation. Write “none” when not applicable.
