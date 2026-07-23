# Observability Contract

CodexHome observability uses a strict append-only event stream. It links tasks, runs, attempts, Codex threads, Homes, accounts, providers, and models without storing prompts, responses, credentials, or arbitrary payloads.

## Store

The default store is `~/.codexhome/observability/events.jsonl`. Override it with `--observability-store <FILE>` or `CODEXHOME_OBSERVABILITY_STORE`.

Each line is one `codexhome.observability-event.v1` object. Appends take an exclusive file lock, validate the complete trace, flush data before returning, and use owner-only permissions on Unix.

## Trace order

The minimum successful trace is:

1. `task_created`
2. `run_started`
3. `attempt_started`
4. optional `thread_linked`, `tool_call_completed`, `artifact_created`, and `verification_completed`
5. `attempt_completed` or `attempt_failed`
6. optional `run_completed` or `run_failed`

IDs must be unique. Child links must reference earlier events and timestamps cannot move backwards within a run. Attempt execution events require `homeId` and `model`; health events require `homeId` and a health snapshot.

## Usage

```bash
codexhome observe record run-events.jsonl
codexhome observe verify
codexhome observe summary --json
codexhome observe summary --home-id @research --model gpt-test --json
codexhome observe export --format json --output artifacts/events.json
codexhome observe export --format csv --output artifacts/events.csv
```

`record` accepts one JSON object, a JSON array, JSONL, or stdin with `-`.

Summary metrics include token counts, cached input tokens, cache hit/miss counts, duration, estimated cost in micro-USD, retries, terminal attempt failure rate, tools, artifacts, verifications, and latest Home health. Aggregations are available by Home, account, model, and thread.

## Safety

The event schema denies unknown fields. This deliberately prevents callers from attaching prompts, model responses, environment dumps, tool arguments, cookies, tokens, or arbitrary metadata. Failure reasons are bounded and checked for obvious secret patterns.

The canonical schemas are:

- [Observability event](../schemas/observability-event.schema.json)
- [Observability summary](../schemas/observability-summary.schema.json)

JSON exports use `codexhome.observability-export.v1`. CSV exports flatten stable event identity, usage, and failure columns for analysis tools.
