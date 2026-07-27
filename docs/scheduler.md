# Scheduler v1 Contract

CodexHome Scheduler v1 is a durable local control plane for selecting, leasing,
pausing, recovering, and terminating Agent Runs. It projects queue state from the
same append-only observability stream as Tasks, Runs, Attempts, route decisions,
and usage totals. It does not maintain a second mutable queue database.

The Scheduler records dispatch intent and ownership. Launching or supervising an
external model process remains the responsibility of the execution adapter.

## Atomic append-only dispatch

Every mutation takes the observability store's exclusive file lock, validates the
existing trace, evaluates the transition, appends the resulting events, flushes
them, and then returns the projected state. A successful dispatch records the
route decision, Attempt start, Scheduler dispatch decision, and lease in one
checked append. Two workers cannot claim the same job from the same store.

Scheduler events are immutable facts. Current job state is reconstructed from
those facts and the authoritative Agent Run projection. This preserves one audit
chain for routing, retries, migrations, token use, duration, cost, and recovery.

## Job contract

An input job uses `codexhome.scheduler-job.v1` and binds exactly one `jobId` to
one existing `runId`. It contains bounded orchestration metadata only:

- `priority`: `background`, `low`, `normal`, `high`, or `critical`.
- `dependencies`: earlier Scheduler jobs that must succeed first.
- `notBeforeMs`: optional earliest dispatch time.
- `deadlineMs`: optional dispatch deadline.
- `attemptTimeoutMs`: maximum duration of one dispatched Attempt.
- `leaseDurationMs`: initial ownership window, no longer than the Attempt timeout.
- `maxDispatches`: total dispatch ceiling.
- `maxConsecutiveFailures`: automatic attention/pause threshold.
- `budget`: optional token, duration, cost, and Attempt limits layered over the Run budget.
- `routeRequest`: the safe, versioned policy-router request.
- `candidatePreference`: optional ordered candidate IDs for preferred/fallback selection.

Dependencies form an append-time DAG because every referenced job must already
exist, duplicate and self-dependencies are rejected, and a dependent job remains
waiting until every dependency succeeds. A failed dependency, expired deadline,
exhausted budget, dispatch limit, or failure threshold moves the projection to a
non-dispatchable attention state with an explicit reason code.

## Priority and scheduling

Without `--job-id`, dispatch selects from currently eligible jobs by priority and
then deterministic queue order. `notBeforeMs` defers early work and reports
`nextEligibleAtMs`; `deadlineMs` prevents late dispatch. Paused, active,
recovering, terminal, dependency-blocked, and over-budget jobs are never selected.

The Scheduler checks both the job budget and the underlying Run budget before a
new Attempt. The limits cover total tokens, elapsed duration, estimated
micro-USD cost, and Attempt count. Scheduler-specific dispatch and consecutive
failure ceilings are checked separately.

## Concurrency policy

`codexhome.scheduler-policy.v1` sets:

- `maxActiveJobs`: machine-wide active Attempt limit, including Runs started outside the Scheduler.
- `defaultHomeConcurrency`: default active lease limit per Home.
- `defaultModelConcurrency`: default active lease limit per model.
- `homeConcurrency`: case-insensitive per-Home overrides.
- `modelConcurrency`: case-insensitive per-model overrides.
- `maxLeaseMs`: upper bound for one lease.

The default policy path is `~/.codexhome/scheduler-policy.json`. Override it with
`--scheduler-policy <FILE>` or `CODEXHOME_SCHEDULER_POLICY`.

Concurrency denials are durable, explainable deferrals:
`global_concurrency`, `home_concurrency`, or `model_concurrency`.

## Routing and fallback

Dispatch evaluates the normal Route policy while holding the append lock.
Unavailable Homes, invalid authentication, exhausted quota, active rate limits,
unhealthy endpoints, capability failures, and policy violations are rejected by
the Router before a lease is created.

When `candidatePreference` is empty, the highest-ranked eligible Route candidate
is selected in `dynamic` mode. With preferences, the first eligible preferred
candidate is `preferred`; a later eligible candidate is recorded as `fallback`.
Rejected candidates and the immutable Route decision remain in the event chain.
If no candidate is eligible, the job is deferred with `no_eligible_route` rather
than silently switching identity or starting unsafe work.

## Leases and recovery

A dispatch lease identifies the job, dispatch, Attempt, route decision, selected
candidate, owner identity, lease start, and expiry. Long-running workers renew
the active lease explicitly within the Scheduler policy and Attempt deadline.

`schedule tick` detects an expired lease and appends
`scheduler_recovery_required`. It does not assume that the external process is
dead and does not start a duplicate Attempt. An operator or supervisor must
reconcile the worker, then use `recover-retry` to close the expired Attempt with
authoritative usage and append `scheduler_recovery_resolved`. Normal retry and
migration lineage remains visible under the same `runId`.

A Run with a prepared worktree is intentionally excluded from automatic
`recover-retry`: its branch, evidence lineage, and external worker state must be
reconciled, then a new Run must be created through an explicit replan. This is a
safety policy, not a transient scheduler failure.

Jobs that exceed failure, budget, dispatch, or dependency safety thresholds can
be automatically paused. Cancellation and timeout close any active Attempt,
record terminal usage, append the Scheduler terminal event, and terminate the Run.

## CLI

Dispatch needs the shared observability store, Home registry, Route policy, and
Scheduler policy. Configure them with `--observability-store`, `--registry`,
`--route-policy`, and `--scheduler-policy`, or with
`CODEXHOME_OBSERVABILITY_STORE`, `CODEXHOME_REGISTRY`,
`CODEXHOME_ROUTE_POLICY`, and `CODEXHOME_SCHEDULER_POLICY`. Explicit paths do
not require `HOME`; empty environment values are ignored.

Create a Task and active Run before enqueueing its job. The Run must not already
have an Attempt. User-supplied reasons are bounded operational reason codes
(`1..=96` ASCII letters, numbers, `.`, `_`, `:`, or `-`), not prose or payloads.

```bash
codexhome schedule enqueue examples/scheduler-job.example.json
codexhome schedule list --json
codexhome schedule show <job-id> --json
codexhome schedule dispatch
codexhome schedule dispatch --job-id <job-id>
codexhome schedule pause <job-id> --reason maintenance
codexhome schedule resume <job-id> --reason capacity_restored
codexhome schedule renew <job-id> --dispatch-id <dispatch-id> --lease-id <lease-id> --extension-ms 300000 --reason worker_heartbeat
codexhome schedule tick --json
codexhome schedule recover-retry <job-id> --reason worker_stopped --input-tokens 0 --output-tokens 0 --duration-ms 0 --estimated-cost-microusd 0
codexhome schedule cancel <job-id> --reason operator_cancelled --input-tokens 0 --output-tokens 0 --duration-ms 0 --estimated-cost-microusd 0
codexhome schedule timeout <job-id> --reason deadline_exceeded --input-tokens 0 --output-tokens 0 --duration-ms 0 --estimated-cost-microusd 0
codexhome schedule policy validate --json
codexhome schedule policy path
```

Exit status `0` means the command succeeded. Dispatch returns `1` for a valid,
durably recorded deferral (`dispatched: false`). Validation, state, I/O, and
other command failures return `2`; with `--json`, failures after argument
parsing use the `codexhome.error.v1` envelope. Clap argument errors remain
human-readable because they occur before command execution.

The repository includes
[`scheduler-job.example.json`](../examples/scheduler-job.example.json) and
[`scheduler-policy.example.json`](../examples/scheduler-policy.example.json).
Replace the Run, Home, provider, and model placeholders with IDs from the local
registry and Route policy before dispatch.

## Data minimization

Scheduler job, policy, report, mutation, dispatch, tick, and event-detail schemas
deny unknown fields. The Scheduler never persists prompts, model responses,
credential values, cookies, environment dumps, raw tool arguments, or arbitrary
payloads. IDs, bounded reason codes, safe Route identifiers, execution identity,
usage totals, and local control-plane paths are the complete durable surface.

Canonical schemas:

- [Scheduler job](../schemas/scheduler-job.schema.json)
- [Scheduler policy](../schemas/scheduler-policy.schema.json)
- [Scheduler report](../schemas/scheduler-report.schema.json)
- [Scheduler mutation](../schemas/scheduler-mutation.schema.json)
- [Scheduler dispatch](../schemas/scheduler-dispatch.schema.json)
- [Scheduler tick](../schemas/scheduler-tick.schema.json)
- [Scheduler policy report](../schemas/scheduler-policy-report.schema.json)
- [Scheduler policy path](../schemas/scheduler-policy-path.schema.json)
- [Observability event](../schemas/observability-event.schema.json)
- [Observability summary](../schemas/observability-summary.schema.json)
