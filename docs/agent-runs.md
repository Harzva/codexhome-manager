# Agent Run Contract

Agent Runs separate a durable task from any one Codex conversation or model attempt:

```text
Task
  -> Run
      -> Attempts
      -> Threads and forks
      -> Tool calls
      -> Artifacts
      -> Verification
```

All state is projected from the same append-only event stream used by observability. There is no second mutable Run database. One file lock protects lifecycle validation and append, so a concurrent command cannot complete the same Attempt or Run twice.

## Lifecycle

```bash
codexhome task create --label "Compile benchmark" --kind coding --json
codexhome run start <task-id> \
  --max-total-tokens 300000 \
  --max-duration-ms 5400000 \
  --max-cost-microusd 5000000 \
  --max-attempts 3

codexhome run attempt start <run-id> \
  --home-id home-primary \
  --account account-a \
  --provider openai \
  --model gpt-5.5 \
  --route-reason "complex architecture task"
```

Record the complete usage delta on the terminal Attempt event:

```bash
codexhome run attempt fail <run-id> <attempt-id> \
  --input-tokens 120000 \
  --output-tokens 8000 \
  --cached-input-tokens 90000 \
  --duration-ms 180000 \
  --estimated-cost-microusd 240000 \
  --failure-code rate_limited \
  --failure-phase inference \
  --failure-reason "provider returned a retryable limit" \
  --retryable
```

Terminal Attempt usage is mandatory in the CLI: input tokens, output tokens, duration, and estimated cost must all be supplied explicitly. Detailed tool/verification durations may also be recorded, but aggregate totals use the terminal Attempt value to avoid counting the same work twice.

A retry keeps the same identity unless the caller changes it. A migration must change the Home, account, or model. Both remain under the original `run_id` and require an explicit route reason:

```bash
codexhome run attempt migrate <run-id> <failed-attempt-id> \
  --home-id home-fallback \
  --account account-b \
  --model gpt-5.4 \
  --route-reason "primary account is rate limited"
```

## Evidence

Thread IDs and artifact IDs are opaque identifiers, not paths or payloads:

```bash
codexhome run thread link <run-id> <thread-id> --attempt-id <attempt-id>
codexhome run artifact record <run-id> <attempt-id> \
  --artifact-id patch-1 --artifact-kind patch
codexhome run verification record <run-id> <attempt-id> \
  --verification-id tests-1 \
  --verification-kind tests \
  --target-artifact-id patch-1 \
  --duration-ms 18000
codexhome run attempt complete <run-id> <attempt-id> \
  --thread-id <primary-thread-id> \
  --input-tokens 90000 --output-tokens 12000 \
  --duration-ms 240000 --estimated-cost-microusd 180000
codexhome run complete <run-id> --final-artifact-id patch-1
```

`codexhome run show <run-id> --json` reports total and failed-attempt token, duration, and cost separately. It also returns budget exhaustion, whether retry or migration is allowed, and a deterministic recovery recommendation.

## Budget behavior

Token, duration, cost, and Attempt limits are checked before another Attempt starts. A terminal Attempt is always recorded even when its actual usage exceeds the budget; otherwise the cost ledger would hide overspend. Afterward the Run recommends `pause` and rejects retry/migration until a later policy layer explicitly creates a new Run.

## Safety

Task labels, task kinds, route reasons, failure reasons, and identity labels are length-bounded and checked for obvious credential patterns. The contract stores no task prompt, response, raw tool arguments, credentials, environment dump, local artifact path, or arbitrary JSON payload.

Canonical JSON Schemas:

- [Task list](../schemas/tasks.schema.json)
- [Task detail](../schemas/task.schema.json)
- [Agent Run list](../schemas/agent-runs.schema.json)
- [Agent Run detail](../schemas/agent-run.schema.json)
- [Agent Run mutation](../schemas/agent-run-mutation.schema.json)
