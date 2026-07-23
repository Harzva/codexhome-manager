# Explainable Policy Router

The Policy Router recommends a registered Home, account, provider, and model from explicit policy data plus the append-only observability stream. It does not inspect prompts, read credentials, or launch a model process.

## Inputs

A route request contains bounded task metadata:

- task kind: `simple_code`, `architecture`, `long_document`, `batch`, `retrieval`, `visual`, or `general`;
- estimated context and output tokens;
- required capabilities and preferred specialties;
- optional cost ceiling;
- optional locked Home and model;
- optional security-domain label;
- `sensitiveDirectory`, which requires an explicit `lockedHome`.

The request stores no prompt, source text, directory path, or credential. See `schemas/route-request.schema.json`.

The policy declares each candidate's identity, specialties, capabilities, security domains, context limit, model-strength score, token prices, concurrency capacity, and optional baseline duration. Prices and model capabilities are configuration data because provider offers change independently of this binary.

Copy `examples/router-policy.example.json`, replace the placeholder Home IDs, account labels, provider/model names, and prices, then set:

```bash
export CODEXHOME_ROUTE_POLICY=/absolute/path/to/router-policy.json
```

`--route-policy <FILE>` overrides the environment variable. The default path is `~/.codexhome/router-policy.json`.

## Decision Process

The router is deterministic and has two stages.

### Hard constraints

A candidate is rejected before scoring when any of these conditions applies:

- disabled or absent/unavailable in the active Home registry;
- mismatched user-locked Home or model;
- missing required capability or security domain;
- insufficient context window or model-strength minimum;
- estimated cost above the request ceiling;
- unreachable service, invalid authentication, exhausted quota, or active rate limit;
- unhealthy Home when degraded routing is not allowed;
- active Attempts at the candidate's concurrency limit.

Every rejection appears in `candidates[].rejectionReasons`.

### Explainable score

Eligible candidates receive nine itemized scores in basis points:

1. task/specialty fit;
2. declared model strength;
3. context headroom;
4. latest account quota;
5. smoothed historical Attempt success;
6. relative expected speed;
7. relative estimated cost;
8. current active-Attempt load;
9. latest Home health.

Weights sum to `10000`. A task rule may replace the global weights, which makes cost dominant for batch work or model strength dominant for architecture work. Unknown quota, health, success, and speed use visible policy priors instead of hidden defaults.

Historical success, duration, quota, health, rate-limit state, and load come from verified observability events for the exact Home/account/model identity. Candidate ties are broken by `candidateId`, so identical inputs produce identical output.

## CLI

Validate a policy:

```bash
codexhome route validate --route-policy router-policy.json --json
```

Preview a decision without mutating the event stream:

```bash
codexhome route recommend route-request.json \
  --route-policy router-policy.json \
  --registry registry.json \
  --json
```

Record the same decision in an active Run:

```bash
codexhome route decide route-request.json \
  --run-id run-123 \
  --route-policy router-policy.json \
  --registry registry.json \
  --observability-store events.jsonl \
  --json
```

`route decide` acquires the append-only event-store lock before it reads active
Attempts, health, quota, rate limits, and history. It evaluates and appends the
`route_decided` event under that same lock, so the recorded candidate snapshot
cannot describe an older load state than an event that precedes it.

The command returns a `route_decided` event ID. Bind the selected identity to the next Attempt:

```bash
codexhome run attempt start run-123 \
  --home-id home-spark \
  --account spark-account \
  --model spark-model \
  --route-decision-id evt-route-123 \
  --route-reason "follow recorded policy decision"
```

Verification rejects an Attempt whose Home, account, or model differs from the linked successful decision.

## Reproducibility

Each `route_decided` event stores:

- request ID, policy ID, and policy revision;
- the complete bounded route request, including task kind, token estimates,
  capability/specialty labels, locks, security domain, and cost ceiling;
- evaluation timestamp and exact number of prior events observed;
- lock flags and selected identity;
- selected score and estimated cost;
- each candidate's active load, quota, historical attempts/success, and estimated cost;
- each eligible candidate's score vector, weights, and component explanations;
- every rejected candidate's reasons.

This snapshot is immutable even if the active policy changes later. Trace
verification rejects a recorded decision whose `observedEventCount` does not
equal the number of events that actually preceded it. JSON export preserves the
full snapshot; CSV includes the evaluation timestamp, observed-event count,
selected request, policy, candidate, score, and linked Attempt decision ID.

## Current Boundary

The router recommends and records. It does not yet start processes, create worktrees, retry jobs, or migrate active runs automatically. Those actions belong to the execution engine and scheduler, which must consume the recorded decision rather than silently re-running policy with newer inputs.
