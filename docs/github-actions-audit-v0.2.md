# GitHub Actions Audit — v0.2.0-alpha.1

Overall score: **8.4/10**.

| Dimension | Score / 10 | Evidence |
|---|---:|---|
| Trigger design | 9 | Main pushes and pull requests only; concurrency cancels stale runs |
| Permissions | 10 | Workflow-level `contents: read` |
| Secrets handling | 10 | No secrets, deployments, releases, or privileged PR triggers |
| Caching | 8 | npm cache enabled; Cargo cache omitted to minimize third-party actions |
| Build/test steps | 9 | Format, Clippy, tests, release build, npm audit/build, Tauri check |
| Artifacts | 5 | CI validates but does not publish artifacts; appropriate before release workflow |
| Failure messages | 8 | Small named steps and matrix job labels make failures localizable |
| Reuse and maintainability | 8 | One workflow, locked installs, pinned external actions, Dependabot |

Next iteration: add a separate tag-only release workflow with signed checksums and platform artifacts. It must not run on pull requests.
