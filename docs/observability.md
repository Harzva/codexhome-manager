# Observability Contract

CodexHome observability uses a strict append-only event stream. It links tasks, runs, attempts, Codex threads, Homes, accounts, providers, and models without storing prompts, responses, credentials, or arbitrary payloads.

## Store

The default store is `~/.codexhome/observability/events.jsonl`. Override it with `--observability-store <FILE>` or `CODEXHOME_OBSERVABILITY_STORE`.

Each line is one `codexhome.observability-event.v1` object. Readers and writers
coordinate through a stable sidecar lock. A mutation validates the complete
trace, writes the full replacement JSONL to an owner-only temporary file,
`fsync`s it, atomically renames it, and syncs the parent directory before
returning. Multi-event dispatches therefore cannot expose a valid partial
prefix after a concurrent read or process crash. Agent Run and Scheduler
mutations perform lifecycle, dependency, budget, concurrency, and lease checks
under that same exclusive lock.

## Trace order

The minimum successful trace is:

1. `task_created`
2. `run_started`
3. optional `scheduler_job_queued`
4. optional atomic Scheduler dispatch group: `route_decided`, `attempt_started`, and `scheduler_dispatch_decided`
5. optional `scheduler_dispatch_deferred`, `scheduler_lease_renewed`, pause/resume, or recovery events
6. optional `thread_linked`, `tool_call_completed`, `artifact_created`, and `verification_completed`
7. `attempt_completed` or `attempt_failed`
8. optional `run_completed` or `run_failed`

IDs must be unique. Child links must reference earlier events and timestamps cannot move backwards within a run. Attempt execution events require `homeId` and `model`; health events require `homeId` and a health snapshot.

Scheduler events add immutable job specifications, dispatch/lease ownership,
explainable deferral codes, bounded state-change reasons, and lease renewal
windows. A dispatch is accepted only when its job, Run, Attempt, Route decision,
selected candidate, identity, dispatch ordinal, and lease links agree. Expired
leases produce `scheduler_recovery_required`; an explicit
`scheduler_recovery_resolved` is required before another dispatch.

Optional strict `details` fields carry only bounded orchestration metadata:
task label/kind, Run budget, the safe route request, complete candidate runtime
and score snapshots, route reason, retry or migration source, opaque
artifact/verification IDs, and final artifact IDs. They cannot carry prompts,
model responses, raw tool arguments, environment dumps, or credential material.
Scheduler reports may identify the local control-plane store path, which is
declared by their safety summary and should be reviewed before sharing.

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

Summary metrics include route-decision count, Scheduler jobs, dispatches,
deferrals, pauses, cancellations, timeouts, recoveries, token counts, cached input tokens,
cache hit/miss counts, duration, estimated cost in micro-USD, retries, terminal
attempt failure rate, tools, artifacts, verifications, and latest Home health.
Aggregations are available by Home, account, model, and thread.

The terminal `attempt_completed` or `attempt_failed` event carries the authoritative total usage for that Attempt. Usage attached to tool or verification events is diagnostic breakdown data and is exported, but it is not added again to aggregate Run/Home/model totals. This prevents double-counting when both detailed spans and the terminal Attempt total are present.

## Safety

The event schema denies unknown fields. This deliberately prevents callers from attaching prompts, model responses, environment dumps, tool arguments, cookies, tokens, or arbitrary metadata. Failure reasons are bounded and checked for obvious secret patterns.

The canonical schemas are:

- [Observability event](../schemas/observability-event.schema.json)
- [Observability summary](../schemas/observability-summary.schema.json)
- [Agent Run projection](../schemas/agent-runs.schema.json)
- [Agent Run mutation](../schemas/agent-run-mutation.schema.json)
- [Route request](../schemas/route-request.schema.json)
- [Route policy](../schemas/route-policy.schema.json)
- [Route decision](../schemas/route-decision.schema.json)
- [Scheduler job](../schemas/scheduler-job.schema.json)
- [Scheduler policy](../schemas/scheduler-policy.schema.json)
- [Scheduler report](../schemas/scheduler-report.schema.json)
- [Scheduler mutation](../schemas/scheduler-mutation.schema.json)
- [Scheduler dispatch](../schemas/scheduler-dispatch.schema.json)
- [Scheduler tick](../schemas/scheduler-tick.schema.json)

JSON exports use `codexhome.observability-export.v1`. CSV exports flatten stable
event identity, usage, failure, route-evaluation timestamp, and observed-event
count columns for analysis tools.
