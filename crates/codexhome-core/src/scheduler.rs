use crate::agent_runs::{build_action_event, now_ms, project_agent_runs};
use crate::{
    generate_id, AgentRunAction, AgentRunReport, AttemptLifecycleStatus, AttemptTransition,
    AttemptTransitionKind, EventDetails, EventSafety, ExecutionIdentity, FailureInfo,
    ObservabilityEvent, ObservabilityEventType, ObservabilityStatus, ObservabilityStore,
    RouteCandidateEvaluation, RouteDecision, RouteEngine, RouteHomeState, RoutePolicy,
    RouteSelection, RunLifecycleStatus, SafetySummary, SchedulerDeferredCode,
    SchedulerDeferredDetails, SchedulerDispatchDetails, SchedulerJobSpec, SchedulerLeaseDetails,
    SchedulerPriority, SchedulerSelectionMode, SchedulerStateChangeDetails, TraceContext,
    UsageDelta, OBSERVABILITY_EVENT_SCHEMA_VERSION,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub const SCHEDULER_POLICY_SCHEMA_VERSION: &str = "codexhome.scheduler-policy.v1";
pub const SCHEDULER_REPORT_SCHEMA_VERSION: &str = "codexhome.scheduler-report.v1";
pub const SCHEDULER_MUTATION_SCHEMA_VERSION: &str = "codexhome.scheduler-mutation.v1";
pub const SCHEDULER_DISPATCH_SCHEMA_VERSION: &str = "codexhome.scheduler-dispatch.v1";
pub const SCHEDULER_TICK_SCHEMA_VERSION: &str = "codexhome.scheduler-tick.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SchedulerPolicy {
    pub schema_version: String,
    pub policy_id: String,
    pub revision: u64,
    #[serde(default = "default_max_active_jobs")]
    pub max_active_jobs: u16,
    #[serde(default = "default_home_concurrency")]
    pub default_home_concurrency: u16,
    #[serde(default = "default_model_concurrency")]
    pub default_model_concurrency: u16,
    #[serde(default)]
    pub home_concurrency: Vec<SchedulerConcurrencyLimit>,
    #[serde(default)]
    pub model_concurrency: Vec<SchedulerConcurrencyLimit>,
    #[serde(default = "default_max_lease_ms")]
    pub max_lease_ms: u64,
}

impl SchedulerPolicy {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEDULER_POLICY_SCHEMA_VERSION {
            bail!(
                "scheduler policy uses unsupported schemaVersion '{}'",
                self.schema_version
            );
        }
        validate_identifier("policyId", &self.policy_id)?;
        if self.max_active_jobs == 0
            || self.default_home_concurrency == 0
            || self.default_model_concurrency == 0
            || self.max_lease_ms == 0
        {
            bail!("scheduler policy concurrency and lease limits must be positive");
        }
        validate_limits("homeConcurrency", &self.home_concurrency)?;
        validate_limits("modelConcurrency", &self.model_concurrency)
    }

    fn home_limit(&self, home_id: &str) -> u16 {
        lookup_limit(
            &self.home_concurrency,
            home_id,
            self.default_home_concurrency,
        )
    }

    fn model_limit(&self, model: &str) -> u16 {
        lookup_limit(
            &self.model_concurrency,
            model,
            self.default_model_concurrency,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SchedulerConcurrencyLimit {
    pub key: String,
    pub max_active: u16,
}

#[derive(Debug, Clone)]
pub struct SchedulerPolicyStore {
    path: PathBuf,
}

impl SchedulerPolicyStore {
    pub fn from_environment(override_path: Option<PathBuf>) -> Result<Self> {
        let explicit = override_path.or_else(|| {
            env::var_os("CODEXHOME_SCHEDULER_POLICY")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        });
        Ok(Self {
            path: match explicit {
                Some(path) => path,
                None => crate::user_home()
                    .context("cannot discover the current user home directory")?
                    .join(".codexhome/scheduler-policy.json"),
            },
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<SchedulerPolicy> {
        let contents = fs::read_to_string(&self.path)
            .with_context(|| format!("failed to read {}", self.path.display()))?;
        let policy = serde_json::from_str::<SchedulerPolicy>(&contents)
            .with_context(|| format!("invalid scheduler policy {}", self.path.display()))?;
        policy.validate()?;
        Ok(policy)
    }
}

pub fn parse_scheduler_job(input: &str) -> Result<SchedulerJobSpec> {
    let job =
        serde_json::from_str::<SchedulerJobSpec>(input).context("invalid scheduler job JSON")?;
    validate_job_spec(&job)?;
    Ok(job)
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SchedulerJobStatus {
    Queued,
    Waiting,
    Ready,
    Running,
    Verifying,
    RetryReady,
    Paused,
    RecoveryRequired,
    NeedsAttention,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

impl SchedulerJobStatus {
    fn terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::TimedOut
        )
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerBudgetState {
    pub within_budget: bool,
    pub exhausted_dimensions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerEligibility {
    pub eligible: bool,
    pub code: Option<SchedulerDeferredCode>,
    pub reason: String,
    pub next_eligible_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerDispatchView {
    pub dispatched_at_ms: u64,
    pub identity: ExecutionIdentity,
    pub details: SchedulerDispatchDetails,
    pub lease_expires_at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerDeferredView {
    pub deferred_at_ms: u64,
    pub details: SchedulerDeferredDetails,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerJobView {
    pub job_id: String,
    pub task_id: String,
    pub run_id: String,
    pub status: SchedulerJobStatus,
    pub queued_at_ms: u64,
    pub updated_at_ms: u64,
    pub spec: SchedulerJobSpec,
    pub dispatches: Vec<SchedulerDispatchView>,
    pub last_deferred: Option<SchedulerDeferredView>,
    pub consecutive_failures: u32,
    pub budget_state: SchedulerBudgetState,
    pub eligibility: SchedulerEligibility,
    pub state_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerReport {
    pub ok: bool,
    pub schema_version: &'static str,
    pub store_path: String,
    pub as_of_timestamp_ms: Option<u64>,
    pub evaluated_at_ms: u64,
    pub jobs: Vec<SchedulerJobView>,
    pub safety: SafetySummary,
}

impl SchedulerReport {
    pub fn job(&self, job_id: &str) -> Result<&SchedulerJobView> {
        self.jobs
            .iter()
            .find(|job| job.job_id == job_id)
            .with_context(|| format!("scheduler jobId '{job_id}' was not found"))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerMutationReport {
    pub ok: bool,
    pub schema_version: &'static str,
    pub action: String,
    pub event_ids: Vec<String>,
    pub job: SchedulerJobView,
    pub safety: SafetySummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerDispatchReport {
    pub ok: bool,
    pub schema_version: &'static str,
    pub dispatched: bool,
    pub decision: Option<RouteDecision>,
    pub event_ids: Vec<String>,
    pub job: SchedulerJobView,
    pub safety: SafetySummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerTickReport {
    pub ok: bool,
    pub schema_version: &'static str,
    pub evaluated_at_ms: u64,
    pub event_ids: Vec<String>,
    pub recovery_required: Vec<String>,
    pub automatically_paused: Vec<String>,
    pub safety: SafetySummary,
}

#[derive(Debug, Clone)]
pub struct SchedulerStore {
    events: ObservabilityStore,
}

impl SchedulerStore {
    pub fn from_environment(override_path: Option<PathBuf>) -> Result<Self> {
        Ok(Self {
            events: ObservabilityStore::from_environment(override_path)?,
        })
    }

    pub fn new(path: PathBuf) -> Self {
        Self {
            events: ObservabilityStore::new(path),
        }
    }

    pub fn path(&self) -> &Path {
        self.events.path()
    }

    pub fn report(&self) -> Result<SchedulerReport> {
        self.report_at(now_ms()?)
    }

    pub fn report_at(&self, evaluated_at_ms: u64) -> Result<SchedulerReport> {
        project_scheduler(
            &self.events.load_verified()?,
            self.events.path(),
            evaluated_at_ms,
        )
    }

    pub fn enqueue(&self, job: SchedulerJobSpec) -> Result<SchedulerMutationReport> {
        validate_job_spec(&job)?;
        let event_id = generate_id("evt");
        let event_id_for_build = event_id.clone();
        let job_for_build = job.clone();
        self.events.append_checked(move |events| {
            let runs = project_agent_runs(events, Path::new(""))?;
            let run = runs.run(&job_for_build.run_id)?;
            if run.status != RunLifecycleStatus::Running {
                bail!("scheduler can only enqueue an active Run");
            }
            if !run.attempts.is_empty() {
                bail!("scheduler job must be enqueued before the Run starts an Attempt");
            }
            let scheduler = project_scheduler(events, Path::new(""), now_ms()?)?;
            if scheduler
                .jobs
                .iter()
                .any(|existing| existing.run_id == job_for_build.run_id)
            {
                bail!("runId '{}' is already scheduled", job_for_build.run_id);
            }
            for dependency in &job_for_build.dependencies {
                scheduler.job(dependency)?;
            }
            let mut event = scheduler_event(
                event_id_for_build,
                now_ms()?,
                ObservabilityEventType::SchedulerJobQueued,
                ObservabilityStatus::Started,
                &run.task_id,
                &run.run_id,
                None,
            );
            event.details.scheduler_job = Some(job_for_build);
            Ok(vec![event])
        })?;
        self.mutation("enqueue", &job.job_id, vec![event_id])
    }

    pub fn pause(
        &self,
        job_id: &str,
        reason: &str,
        automatic: bool,
    ) -> Result<SchedulerMutationReport> {
        self.state_change(
            job_id,
            "pause",
            reason,
            automatic,
            ObservabilityEventType::SchedulerJobPaused,
            ObservabilityStatus::Degraded,
        )
    }

    pub fn resume(&self, job_id: &str, reason: &str) -> Result<SchedulerMutationReport> {
        self.state_change(
            job_id,
            "resume",
            reason,
            false,
            ObservabilityEventType::SchedulerJobResumed,
            ObservabilityStatus::Running,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn dispatch(
        &self,
        scheduler_policy: &SchedulerPolicy,
        route_policy: &RoutePolicy,
        homes: &[RouteHomeState],
        job_id: Option<&str>,
    ) -> Result<SchedulerDispatchReport> {
        self.dispatch_at(scheduler_policy, route_policy, homes, job_id, now_ms()?)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_at(
        &self,
        scheduler_policy: &SchedulerPolicy,
        route_policy: &RoutePolicy,
        homes: &[RouteHomeState],
        job_id: Option<&str>,
        timestamp_ms: u64,
    ) -> Result<SchedulerDispatchReport> {
        scheduler_policy.validate()?;
        let engine = RouteEngine::new(route_policy)?;
        let result_slot = RefCell::new(None::<(bool, Option<RouteDecision>, String, Vec<String>)>);
        self.events.append_checked(|events| {
            let scheduler = project_scheduler(events, Path::new(""), timestamp_ms)?;
            let runs = project_agent_runs(events, Path::new(""))?;
            let job = select_job(&scheduler, job_id)?;
            if job.spec.lease_duration_ms > scheduler_policy.max_lease_ms {
                bail!(
                    "job leaseDurationMs {} exceeds scheduler policy maxLeaseMs {}",
                    job.spec.lease_duration_ms,
                    scheduler_policy.max_lease_ms
                );
            }
            let mut transaction = Vec::new();
            if !job.eligibility.eligible {
                let event_id = generate_id("evt");
                let deferred = deferred_event(
                    event_id.clone(),
                    timestamp_ms,
                    job,
                    job.eligibility
                        .code
                        .unwrap_or(SchedulerDeferredCode::RunNotReady),
                    job.eligibility.reason.clone(),
                    job.eligibility.next_eligible_at_ms,
                    None,
                );
                transaction.push(deferred);
                result_slot.replace(Some((false, None, job.job_id.clone(), vec![event_id])));
                return Ok(transaction);
            }
            let active = active_counts(&runs);
            if active.total >= u64::from(scheduler_policy.max_active_jobs) {
                let event_id = generate_id("evt");
                transaction.push(deferred_event(
                    event_id.clone(),
                    timestamp_ms,
                    job,
                    SchedulerDeferredCode::GlobalConcurrency,
                    format!(
                        "global active Attempt count {}/{} reached the scheduler limit",
                        active.total, scheduler_policy.max_active_jobs
                    ),
                    None,
                    None,
                ));
                result_slot.replace(Some((false, None, job.job_id.clone(), vec![event_id])));
                return Ok(transaction);
            }

            let mut decision =
                engine.evaluate(&job.spec.route_request, events, homes, timestamp_ms)?;
            let selection = choose_scheduler_candidate(
                job,
                &decision,
                scheduler_policy,
                &active,
                runs.run(&job.run_id)?,
            );
            let route_event_id = generate_id("evt");
            let mut event_ids = vec![route_event_id.clone()];
            match selection {
                Ok((candidate, mode, scheduler_rejections)) => {
                    decision.ok = true;
                    decision.selected = Some(RouteSelection {
                        candidate_id: candidate.candidate_id.clone(),
                        identity: candidate.identity.clone(),
                        total_score_basis_points: candidate
                            .total_score_basis_points
                            .unwrap_or_default(),
                        estimated_cost_microusd: candidate.estimated_cost_microusd,
                        reasons: candidate.reasons.clone(),
                    });
                    decision.decision_reason = format!(
                        "scheduler selected '{}' via {:?} preference for job '{}'",
                        candidate.candidate_id, mode, job.job_id
                    );
                    let route_action = AgentRunAction::RecordRouteDecision {
                        run_id: job.run_id.clone(),
                        decision: Box::new(decision.clone()),
                    };
                    let route_event = build_action_event(
                        &runs,
                        &route_action,
                        route_event_id.clone(),
                        timestamp_ms,
                        events,
                    )?;
                    transaction.push(route_event);

                    let events_with_route = combined(events, &transaction);
                    let routed_runs = project_agent_runs(&events_with_route, Path::new(""))?;
                    let run = routed_runs.run(&job.run_id)?;
                    let attempt_id = generate_id("attempt");
                    let transition = scheduler_transition(run, &candidate.identity)?;
                    let attempt_action = AgentRunAction::StartAttempt {
                        run_id: job.run_id.clone(),
                        attempt_id: attempt_id.clone(),
                        identity: candidate.identity.clone(),
                        transition,
                        route_reason: Some(format!(
                            "scheduler job '{}' dispatched candidate '{}'",
                            job.job_id, candidate.candidate_id
                        )),
                        route_decision_id: Some(route_event_id.clone()),
                    };
                    let attempt_event_id = generate_id("evt");
                    let attempt_event = build_action_event(
                        &routed_runs,
                        &attempt_action,
                        attempt_event_id.clone(),
                        timestamp_ms,
                        &events_with_route,
                    )?;
                    transaction.push(attempt_event);
                    event_ids.push(attempt_event_id.clone());

                    let dispatch_event_id = generate_id("evt");
                    let dispatch_id = generate_id("dispatch");
                    let lease_id = generate_id("lease");
                    let lease_expires_at_ms = timestamp_ms
                        .checked_add(job.spec.lease_duration_ms)
                        .context("scheduler lease expiry overflow")?;
                    let mut dispatch_event = scheduler_event(
                        dispatch_event_id.clone(),
                        timestamp_ms,
                        ObservabilityEventType::SchedulerDispatchDecided,
                        ObservabilityStatus::Succeeded,
                        &job.task_id,
                        &job.run_id,
                        Some(&attempt_id),
                    );
                    dispatch_event.identity = candidate.identity.clone();
                    dispatch_event.parent_event_id = Some(attempt_event_id);
                    dispatch_event.details.scheduler_dispatch = Some(SchedulerDispatchDetails {
                        job_id: job.job_id.clone(),
                        dispatch_id,
                        dispatch_ordinal: u32::try_from(job.dispatches.len() + 1)
                            .context("scheduler dispatch ordinal overflow")?,
                        attempt_id,
                        route_decision_id: route_event_id,
                        lease_id,
                        leased_at_ms: timestamp_ms,
                        lease_expires_at_ms,
                        selected_candidate_id: candidate.candidate_id.clone(),
                        selection_mode: mode,
                        scheduler_rejections,
                    });
                    transaction.push(dispatch_event);
                    event_ids.push(dispatch_event_id);
                    result_slot.replace(Some((
                        true,
                        Some(decision),
                        job.job_id.clone(),
                        event_ids,
                    )));
                }
                Err((code, reason, scheduler_rejections)) => {
                    decision.ok = false;
                    decision.selected = None;
                    decision.decision_reason = reason.clone();
                    let route_action = AgentRunAction::RecordRouteDecision {
                        run_id: job.run_id.clone(),
                        decision: Box::new(decision.clone()),
                    };
                    let route_event = build_action_event(
                        &runs,
                        &route_action,
                        route_event_id.clone(),
                        timestamp_ms,
                        events,
                    )?;
                    transaction.push(route_event);
                    let deferred_event_id = generate_id("evt");
                    let full_reason = if scheduler_rejections.is_empty() {
                        reason
                    } else {
                        format!("{reason}; {}", scheduler_rejections.join("; "))
                    };
                    transaction.push(deferred_event(
                        deferred_event_id.clone(),
                        timestamp_ms,
                        job,
                        code,
                        full_reason,
                        None,
                        Some(route_event_id),
                    ));
                    event_ids.push(deferred_event_id);
                    result_slot.replace(Some((
                        false,
                        Some(decision),
                        job.job_id.clone(),
                        event_ids,
                    )));
                }
            }
            Ok(transaction)
        })?;
        let (dispatched, decision, selected_job_id, event_ids) = result_slot
            .into_inner()
            .context("dispatch produced no result")?;
        let job = self.report_at(timestamp_ms)?.job(&selected_job_id)?.clone();
        Ok(SchedulerDispatchReport {
            ok: dispatched,
            schema_version: SCHEDULER_DISPATCH_SCHEMA_VERSION,
            dispatched,
            decision,
            event_ids,
            job,
            safety: scheduler_safety(),
        })
    }

    pub fn renew_lease(
        &self,
        policy: &SchedulerPolicy,
        job_id: &str,
        dispatch_id: &str,
        lease_id: &str,
        extension_ms: u64,
        reason: &str,
    ) -> Result<SchedulerMutationReport> {
        self.renew_lease_at(
            policy,
            job_id,
            dispatch_id,
            lease_id,
            extension_ms,
            reason,
            now_ms()?,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn renew_lease_at(
        &self,
        policy: &SchedulerPolicy,
        job_id: &str,
        dispatch_id: &str,
        lease_id: &str,
        extension_ms: u64,
        reason: &str,
        timestamp_ms: u64,
    ) -> Result<SchedulerMutationReport> {
        policy.validate()?;
        validate_reason(reason)?;
        if extension_ms == 0 || extension_ms > policy.max_lease_ms {
            bail!("lease extension must be within scheduler policy maxLeaseMs");
        }
        let event_id = generate_id("evt");
        let event_id_for_build = event_id.clone();
        let job_id = job_id.to_owned();
        let job_id_for_event = job_id.clone();
        let dispatch_id = dispatch_id.to_owned();
        let lease_id = lease_id.to_owned();
        let reason = reason.to_owned();
        self.events.append_checked(move |events| {
            let report = project_scheduler(events, Path::new(""), timestamp_ms)?;
            let job = report.job(&job_id_for_event)?;
            if job.status != SchedulerJobStatus::Running {
                bail!("scheduler lease can only be renewed for a running job");
            }
            let dispatch = job.dispatches.last().context("job has no dispatch lease")?;
            if dispatch.details.dispatch_id != dispatch_id || dispatch.details.lease_id != lease_id
            {
                bail!("lease renewal does not match the active dispatch and lease");
            }
            if timestamp_ms >= dispatch.lease_expires_at_ms {
                bail!("expired scheduler leases require recovery and cannot be renewed");
            }
            let lease_expires_at_ms = dispatch
                .lease_expires_at_ms
                .checked_add(extension_ms)
                .context("scheduler lease extension overflow")?;
            let policy_cap = timestamp_ms
                .checked_add(policy.max_lease_ms)
                .context("scheduler policy lease cap overflow")?;
            if lease_expires_at_ms > policy_cap {
                bail!("renewed lease window exceeds scheduler policy maxLeaseMs");
            }
            let attempt_cap = dispatch
                .dispatched_at_ms
                .checked_add(job.spec.attempt_timeout_ms)
                .context("scheduler attempt timeout overflow")?;
            if lease_expires_at_ms > attempt_cap {
                bail!("lease extension exceeds the job attempt timeout");
            }
            let runs = project_agent_runs(events, Path::new(""))?;
            let run = runs.run(&job.run_id)?;
            if duration_budget_deadline(job, run, dispatch.dispatched_at_ms)
                .is_some_and(|deadline| lease_expires_at_ms > deadline)
            {
                bail!("lease extension exceeds the remaining duration budget");
            }
            let mut event = scheduler_event(
                event_id_for_build,
                timestamp_ms,
                ObservabilityEventType::SchedulerLeaseRenewed,
                ObservabilityStatus::Running,
                &job.task_id,
                &job.run_id,
                Some(&dispatch.details.attempt_id),
            );
            event.identity = dispatch.identity.clone();
            event.details.scheduler_lease = Some(SchedulerLeaseDetails {
                job_id: job.job_id.clone(),
                dispatch_id: dispatch.details.dispatch_id.clone(),
                lease_id: dispatch.details.lease_id.clone(),
                previous_expires_at_ms: dispatch.lease_expires_at_ms,
                lease_expires_at_ms,
                reason,
            });
            Ok(vec![event])
        })?;
        self.mutation("renew_lease", &job_id, vec![event_id])
    }

    pub fn tick(&self) -> Result<SchedulerTickReport> {
        self.tick_at(now_ms()?)
    }

    pub fn tick_at(&self, timestamp_ms: u64) -> Result<SchedulerTickReport> {
        let event_ids = RefCell::new(Vec::new());
        let recovery = RefCell::new(Vec::new());
        let paused = RefCell::new(Vec::new());
        self.events.append_checked_optional(|events| {
            let report = project_scheduler(events, Path::new(""), timestamp_ms)?;
            let mut transaction = Vec::new();
            for job in &report.jobs {
                if job.status == SchedulerJobStatus::Running {
                    let dispatch = job
                        .dispatches
                        .last()
                        .context("running job has no dispatch")?;
                    if dispatch.lease_expires_at_ms <= timestamp_ms {
                        let event_id = generate_id("evt");
                        let mut event = scheduler_event(
                            event_id.clone(),
                            timestamp_ms,
                            ObservabilityEventType::SchedulerRecoveryRequired,
                            ObservabilityStatus::Degraded,
                            &job.task_id,
                            &job.run_id,
                            Some(&dispatch.details.attempt_id),
                        );
                        event.identity = dispatch.identity.clone();
                        event.details.scheduler_state_change = Some(SchedulerStateChangeDetails {
                            job_id: job.job_id.clone(),
                            reason: format!(
                                "lease '{}' expired at {}",
                                dispatch.details.lease_id, dispatch.lease_expires_at_ms
                            ),
                            automatic: true,
                        });
                        transaction.push(event);
                        event_ids.borrow_mut().push(event_id);
                        recovery.borrow_mut().push(job.job_id.clone());
                    }
                } else if job.status == SchedulerJobStatus::NeedsAttention
                    && matches!(
                        job.eligibility.code,
                        Some(
                            SchedulerDeferredCode::FailureThreshold
                                | SchedulerDeferredCode::BudgetExhausted
                                | SchedulerDeferredCode::DispatchLimit
                                | SchedulerDeferredCode::DependencyFailed
                                | SchedulerDeferredCode::DeadlineExpired
                        )
                    )
                {
                    let event_id = generate_id("evt");
                    let mut event = scheduler_event(
                        event_id.clone(),
                        timestamp_ms,
                        ObservabilityEventType::SchedulerJobPaused,
                        ObservabilityStatus::Degraded,
                        &job.task_id,
                        &job.run_id,
                        None,
                    );
                    event.details.scheduler_state_change = Some(SchedulerStateChangeDetails {
                        job_id: job.job_id.clone(),
                        reason: job.eligibility.reason.clone(),
                        automatic: true,
                    });
                    transaction.push(event);
                    event_ids.borrow_mut().push(event_id);
                    paused.borrow_mut().push(job.job_id.clone());
                }
            }
            Ok(transaction)
        })?;
        Ok(SchedulerTickReport {
            ok: true,
            schema_version: SCHEDULER_TICK_SCHEMA_VERSION,
            evaluated_at_ms: timestamp_ms,
            event_ids: event_ids.into_inner(),
            recovery_required: recovery.into_inner(),
            automatically_paused: paused.into_inner(),
            safety: scheduler_safety(),
        })
    }

    pub fn resolve_recovery_for_retry(
        &self,
        job_id: &str,
        usage: UsageDelta,
        reason: &str,
    ) -> Result<SchedulerMutationReport> {
        validate_reason(reason)?;
        let job_id_owned = job_id.to_owned();
        let reason_owned = reason.to_owned();
        let event_ids_slot = RefCell::new(Vec::new());
        self.events.append_checked(|events| {
            let timestamp_ms = now_ms()?;
            let scheduler = project_scheduler(events, Path::new(""), timestamp_ms)?;
            let job = scheduler.job(&job_id_owned)?;
            if job.status != SchedulerJobStatus::RecoveryRequired {
                bail!("scheduler job does not require recovery");
            }
            let runs = project_agent_runs(events, Path::new(""))?;
            let run = runs.run(&job.run_id)?;
            if run.worktree.is_some() {
                bail!("worktree Run recovery requires a new Run and explicit replan");
            }
            let attempt = run
                .attempts
                .iter()
                .find(|attempt| attempt.status == AttemptLifecycleStatus::Running)
                .context("recovery job has no active Attempt")?;
            let failure_event_id = generate_id("evt");
            let failure_action = AgentRunAction::FailAttempt {
                run_id: run.run_id.clone(),
                attempt_id: attempt.attempt_id.clone(),
                thread_id: None,
                status: ObservabilityStatus::TimedOut,
                usage: usage.clone(),
                failure: FailureInfo {
                    code: "scheduler_lease_expired".to_owned(),
                    phase: Some("scheduler".to_owned()),
                    reason: reason_owned.clone(),
                    retryable: true,
                },
            };
            let failure_event = build_action_event(
                &runs,
                &failure_action,
                failure_event_id.clone(),
                timestamp_ms,
                events,
            )?;
            let resolved_event_id = generate_id("evt");
            let dispatch = job
                .dispatches
                .last()
                .context("recovery job has no dispatch")?;
            let mut resolved = scheduler_event(
                resolved_event_id.clone(),
                timestamp_ms,
                ObservabilityEventType::SchedulerRecoveryResolved,
                ObservabilityStatus::Running,
                &job.task_id,
                &job.run_id,
                Some(&attempt.attempt_id),
            );
            resolved.identity = dispatch.identity.clone();
            resolved.parent_event_id = Some(failure_event_id.clone());
            resolved.details.scheduler_state_change = Some(SchedulerStateChangeDetails {
                job_id: job.job_id.clone(),
                reason: reason_owned.clone(),
                automatic: false,
            });
            event_ids_slot
                .borrow_mut()
                .extend([failure_event_id, resolved_event_id]);
            Ok(vec![failure_event, resolved])
        })?;
        self.mutation(
            "resolve_recovery_for_retry",
            job_id,
            event_ids_slot.into_inner(),
        )
    }

    pub fn cancel(
        &self,
        job_id: &str,
        usage: UsageDelta,
        reason: &str,
    ) -> Result<SchedulerMutationReport> {
        self.terminalize(
            job_id,
            usage,
            reason,
            "scheduler_cancelled",
            ObservabilityEventType::SchedulerJobCancelled,
            ObservabilityStatus::Cancelled,
            "cancel",
        )
    }

    pub fn timeout(
        &self,
        job_id: &str,
        usage: UsageDelta,
        reason: &str,
    ) -> Result<SchedulerMutationReport> {
        self.terminalize(
            job_id,
            usage,
            reason,
            "scheduler_deadline_exceeded",
            ObservabilityEventType::SchedulerJobTimedOut,
            ObservabilityStatus::TimedOut,
            "timeout",
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn terminalize(
        &self,
        job_id: &str,
        usage: UsageDelta,
        reason: &str,
        failure_code: &str,
        scheduler_event_type: ObservabilityEventType,
        status: ObservabilityStatus,
        action_name: &str,
    ) -> Result<SchedulerMutationReport> {
        validate_reason(reason)?;
        let job_id_owned = job_id.to_owned();
        let reason_owned = reason.to_owned();
        let failure_code = failure_code.to_owned();
        let event_ids_slot = RefCell::new(Vec::new());
        self.events.append_checked(|events| {
            let timestamp_ms = now_ms()?;
            let scheduler = project_scheduler(events, Path::new(""), timestamp_ms)?;
            let job = scheduler.job(&job_id_owned)?;
            if job.status.terminal() {
                bail!("scheduler job is already terminal");
            }
            let mut transaction = Vec::new();
            let mut working_events = events.to_vec();
            let mut runs = project_agent_runs(&working_events, Path::new(""))?;
            let run = runs.run(&job.run_id)?;
            let active_attempt = run
                .attempts
                .iter()
                .find(|attempt| attempt.status == AttemptLifecycleStatus::Running)
                .cloned();
            if active_attempt.is_none() && usage != UsageDelta::default() {
                bail!("terminal usage must be zero when the job has no active Attempt");
            }
            if let Some(attempt) = active_attempt {
                let attempt_event_id = generate_id("evt");
                let attempt_action = AgentRunAction::FailAttempt {
                    run_id: run.run_id.clone(),
                    attempt_id: attempt.attempt_id.clone(),
                    thread_id: None,
                    status,
                    usage: usage.clone(),
                    failure: FailureInfo {
                        code: failure_code.clone(),
                        phase: Some("scheduler".to_owned()),
                        reason: reason_owned.clone(),
                        retryable: false,
                    },
                };
                let attempt_event = build_action_event(
                    &runs,
                    &attempt_action,
                    attempt_event_id.clone(),
                    timestamp_ms,
                    &working_events,
                )?;
                working_events.push(attempt_event.clone());
                transaction.push(attempt_event);
                event_ids_slot.borrow_mut().push(attempt_event_id);
                runs = project_agent_runs(&working_events, Path::new(""))?;
            }
            let state_event_id = generate_id("evt");
            let mut state_event = scheduler_event(
                state_event_id.clone(),
                timestamp_ms,
                scheduler_event_type,
                status,
                &job.task_id,
                &job.run_id,
                None,
            );
            state_event.details.scheduler_state_change = Some(SchedulerStateChangeDetails {
                job_id: job.job_id.clone(),
                reason: reason_owned.clone(),
                automatic: false,
            });
            working_events.push(state_event.clone());
            transaction.push(state_event);
            event_ids_slot.borrow_mut().push(state_event_id);

            let run_event_id = generate_id("evt");
            let run_action = AgentRunAction::FailRun {
                run_id: job.run_id.clone(),
                status,
                failure: FailureInfo {
                    code: failure_code.clone(),
                    phase: Some("scheduler".to_owned()),
                    reason: reason_owned.clone(),
                    retryable: false,
                },
            };
            let run_event = build_action_event(
                &runs,
                &run_action,
                run_event_id.clone(),
                timestamp_ms,
                &working_events,
            )?;
            transaction.push(run_event);
            event_ids_slot.borrow_mut().push(run_event_id);
            Ok(transaction)
        })?;
        self.mutation(action_name, job_id, event_ids_slot.into_inner())
    }

    fn state_change(
        &self,
        job_id: &str,
        action: &str,
        reason: &str,
        automatic: bool,
        event_type: ObservabilityEventType,
        status: ObservabilityStatus,
    ) -> Result<SchedulerMutationReport> {
        validate_reason(reason)?;
        let event_id = generate_id("evt");
        let event_id_for_build = event_id.clone();
        let job_id_owned = job_id.to_owned();
        let reason = reason.to_owned();
        self.events.append_checked(move |events| {
            let report = project_scheduler(events, Path::new(""), now_ms()?)?;
            let job = report.job(&job_id_owned)?;
            if job.status.terminal() {
                bail!("scheduler job is already terminal");
            }
            if event_type == ObservabilityEventType::SchedulerJobPaused
                && job.status == SchedulerJobStatus::Paused
            {
                bail!("scheduler job is already paused");
            }
            if event_type == ObservabilityEventType::SchedulerJobPaused
                && !matches!(
                    job.status,
                    SchedulerJobStatus::Queued
                        | SchedulerJobStatus::Waiting
                        | SchedulerJobStatus::Ready
                        | SchedulerJobStatus::RetryReady
                        | SchedulerJobStatus::NeedsAttention
                )
            {
                bail!("only queued, waiting, retry-ready, or attention jobs can be paused");
            }
            if event_type == ObservabilityEventType::SchedulerJobResumed
                && job.status != SchedulerJobStatus::Paused
            {
                bail!("scheduler job is not paused");
            }
            let mut event = scheduler_event(
                event_id_for_build,
                now_ms()?,
                event_type,
                status,
                &job.task_id,
                &job.run_id,
                None,
            );
            event.details.scheduler_state_change = Some(SchedulerStateChangeDetails {
                job_id: job.job_id.clone(),
                reason,
                automatic,
            });
            Ok(vec![event])
        })?;
        self.mutation(action, job_id, vec![event_id])
    }

    fn mutation(
        &self,
        action: &str,
        job_id: &str,
        event_ids: Vec<String>,
    ) -> Result<SchedulerMutationReport> {
        let job = self.report()?.job(job_id)?.clone();
        Ok(SchedulerMutationReport {
            ok: true,
            schema_version: SCHEDULER_MUTATION_SCHEMA_VERSION,
            action: action.to_owned(),
            event_ids,
            job,
            safety: scheduler_safety(),
        })
    }
}

fn project_scheduler(
    events: &[ObservabilityEvent],
    path: &Path,
    evaluated_at_ms: u64,
) -> Result<SchedulerReport> {
    let runs = project_agent_runs(events, path)?;
    let mut jobs = BTreeMap::<String, SchedulerJobView>::new();
    let mut as_of = None;
    for event in events {
        as_of = Some(as_of.map_or(event.timestamp_ms, |value: u64| {
            value.max(event.timestamp_ms)
        }));
        match event.event_type {
            ObservabilityEventType::SchedulerJobQueued => {
                let spec = event
                    .details
                    .scheduler_job
                    .clone()
                    .context("scheduler_job_queued is missing details")?;
                let task_id = event
                    .trace
                    .task_id
                    .clone()
                    .context("scheduler_job_queued is missing taskId")?;
                jobs.insert(
                    spec.job_id.clone(),
                    SchedulerJobView {
                        job_id: spec.job_id.clone(),
                        task_id,
                        run_id: spec.run_id.clone(),
                        status: SchedulerJobStatus::Queued,
                        queued_at_ms: event.timestamp_ms,
                        updated_at_ms: event.timestamp_ms,
                        spec,
                        dispatches: Vec::new(),
                        last_deferred: None,
                        consecutive_failures: 0,
                        budget_state: SchedulerBudgetState {
                            within_budget: true,
                            exhausted_dimensions: Vec::new(),
                        },
                        eligibility: ineligible(
                            SchedulerDeferredCode::RunNotReady,
                            "scheduler state has not been evaluated",
                            None,
                        ),
                        state_reason: None,
                    },
                );
            }
            ObservabilityEventType::SchedulerDispatchDecided => {
                let details = event
                    .details
                    .scheduler_dispatch
                    .clone()
                    .context("scheduler dispatch is missing details")?;
                let job = jobs
                    .get_mut(&details.job_id)
                    .context("scheduler dispatch precedes job")?;
                job.updated_at_ms = event.timestamp_ms;
                job.status = SchedulerJobStatus::Running;
                job.state_reason = None;
                job.dispatches.push(SchedulerDispatchView {
                    dispatched_at_ms: event.timestamp_ms,
                    identity: event.identity.clone(),
                    lease_expires_at_ms: details.lease_expires_at_ms,
                    details,
                });
            }
            ObservabilityEventType::SchedulerDispatchDeferred => {
                let details = event
                    .details
                    .scheduler_deferred
                    .clone()
                    .context("scheduler deferred event is missing details")?;
                let job = jobs
                    .get_mut(&details.job_id)
                    .context("scheduler deferred event precedes job")?;
                job.updated_at_ms = event.timestamp_ms;
                job.state_reason = Some(details.reason.clone());
                job.last_deferred = Some(SchedulerDeferredView {
                    deferred_at_ms: event.timestamp_ms,
                    details,
                });
            }
            ObservabilityEventType::SchedulerJobPaused
            | ObservabilityEventType::SchedulerJobResumed
            | ObservabilityEventType::SchedulerJobCancelled
            | ObservabilityEventType::SchedulerJobTimedOut
            | ObservabilityEventType::SchedulerRecoveryRequired
            | ObservabilityEventType::SchedulerRecoveryResolved => {
                let change = event
                    .details
                    .scheduler_state_change
                    .as_ref()
                    .context("scheduler state event is missing details")?;
                let job = jobs
                    .get_mut(&change.job_id)
                    .context("scheduler state event precedes job")?;
                job.updated_at_ms = event.timestamp_ms;
                job.state_reason = Some(change.reason.clone());
                job.status = match event.event_type {
                    ObservabilityEventType::SchedulerJobPaused => SchedulerJobStatus::Paused,
                    ObservabilityEventType::SchedulerJobResumed
                    | ObservabilityEventType::SchedulerRecoveryResolved => {
                        SchedulerJobStatus::Queued
                    }
                    ObservabilityEventType::SchedulerJobCancelled => SchedulerJobStatus::Cancelled,
                    ObservabilityEventType::SchedulerJobTimedOut => SchedulerJobStatus::TimedOut,
                    ObservabilityEventType::SchedulerRecoveryRequired => {
                        SchedulerJobStatus::RecoveryRequired
                    }
                    _ => unreachable!(),
                };
            }
            ObservabilityEventType::SchedulerLeaseRenewed => {
                let lease = event
                    .details
                    .scheduler_lease
                    .as_ref()
                    .context("scheduler lease event is missing details")?;
                let job = jobs
                    .get_mut(&lease.job_id)
                    .context("scheduler lease event precedes job")?;
                let dispatch = job
                    .dispatches
                    .iter_mut()
                    .find(|dispatch| dispatch.details.dispatch_id == lease.dispatch_id)
                    .context("scheduler lease references unknown dispatch")?;
                dispatch.lease_expires_at_ms = lease.lease_expires_at_ms;
                job.updated_at_ms = event.timestamp_ms;
            }
            _ => {}
        }
    }

    for job in jobs.values_mut() {
        reconcile_job(job, &runs)?;
    }
    let snapshot = jobs.clone();
    for job in jobs.values_mut() {
        evaluate_job(job, &snapshot, &runs, evaluated_at_ms)?;
    }
    let mut jobs = jobs.into_values().collect::<Vec<_>>();
    jobs.sort_by(|left, right| {
        priority_rank(right.spec.priority)
            .cmp(&priority_rank(left.spec.priority))
            .then(left.queued_at_ms.cmp(&right.queued_at_ms))
            .then(left.job_id.cmp(&right.job_id))
    });
    Ok(SchedulerReport {
        ok: true,
        schema_version: SCHEDULER_REPORT_SCHEMA_VERSION,
        store_path: path.to_string_lossy().into_owned(),
        as_of_timestamp_ms: as_of,
        evaluated_at_ms,
        jobs,
        safety: scheduler_safety(),
    })
}

fn reconcile_job(job: &mut SchedulerJobView, runs: &AgentRunReport) -> Result<()> {
    let run = runs.run(&job.run_id)?;
    job.consecutive_failures = run
        .attempts
        .iter()
        .rev()
        .take_while(|attempt| attempt.status != AttemptLifecycleStatus::Succeeded)
        .filter(|attempt| {
            matches!(
                attempt.status,
                AttemptLifecycleStatus::Failed
                    | AttemptLifecycleStatus::Cancelled
                    | AttemptLifecycleStatus::TimedOut
            )
        })
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    job.budget_state = scheduler_budget_state(job, run);
    if job.status.terminal() {
        return Ok(());
    }
    match run.status {
        RunLifecycleStatus::Succeeded => {
            job.status = SchedulerJobStatus::Succeeded;
            return Ok(());
        }
        RunLifecycleStatus::Failed => {
            job.status = SchedulerJobStatus::Failed;
            return Ok(());
        }
        RunLifecycleStatus::Cancelled => {
            job.status = SchedulerJobStatus::Cancelled;
            return Ok(());
        }
        RunLifecycleStatus::TimedOut => {
            job.status = SchedulerJobStatus::TimedOut;
            return Ok(());
        }
        RunLifecycleStatus::Running => {}
    }
    if matches!(
        job.status,
        SchedulerJobStatus::Paused | SchedulerJobStatus::RecoveryRequired
    ) {
        return Ok(());
    }
    let latest = job.dispatches.last().and_then(|dispatch| {
        run.attempts
            .iter()
            .find(|attempt| attempt.attempt_id == dispatch.details.attempt_id)
    });
    job.status = match latest.map(|attempt| attempt.status) {
        Some(AttemptLifecycleStatus::Running) => SchedulerJobStatus::Running,
        Some(AttemptLifecycleStatus::Succeeded) => SchedulerJobStatus::Verifying,
        Some(
            AttemptLifecycleStatus::Failed
            | AttemptLifecycleStatus::Cancelled
            | AttemptLifecycleStatus::TimedOut,
        ) => SchedulerJobStatus::RetryReady,
        None => SchedulerJobStatus::Queued,
    };
    Ok(())
}

fn evaluate_job(
    job: &mut SchedulerJobView,
    jobs: &BTreeMap<String, SchedulerJobView>,
    runs: &AgentRunReport,
    timestamp_ms: u64,
) -> Result<()> {
    if job.status.terminal() {
        job.eligibility = ineligible(
            SchedulerDeferredCode::RunNotReady,
            "scheduler job is terminal",
            None,
        );
        return Ok(());
    }
    if job.status == SchedulerJobStatus::Paused {
        job.eligibility = ineligible(
            SchedulerDeferredCode::FailureThreshold,
            job.state_reason
                .clone()
                .unwrap_or_else(|| "scheduler job is paused".to_owned()),
            None,
        );
        return Ok(());
    }
    if job.status == SchedulerJobStatus::RecoveryRequired {
        job.eligibility = ineligible(
            SchedulerDeferredCode::RecoveryRequired,
            job.state_reason
                .clone()
                .unwrap_or_else(|| "scheduler lease requires recovery".to_owned()),
            None,
        );
        return Ok(());
    }
    if job.status == SchedulerJobStatus::Running {
        job.eligibility = ineligible(
            SchedulerDeferredCode::ActiveAttempt,
            "scheduler job has an active Attempt",
            job.dispatches
                .last()
                .map(|dispatch| dispatch.lease_expires_at_ms),
        );
        return Ok(());
    }
    if job.status == SchedulerJobStatus::Verifying {
        job.eligibility = ineligible(
            SchedulerDeferredCode::RunNotReady,
            "latest Attempt succeeded and awaits verification or Run completion",
            None,
        );
        return Ok(());
    }
    if let Some(not_before) = job.spec.not_before_ms {
        if timestamp_ms < not_before {
            job.status = SchedulerJobStatus::Waiting;
            job.eligibility = ineligible(
                SchedulerDeferredCode::NotBefore,
                format!("job is scheduled for timestamp {not_before}"),
                Some(not_before),
            );
            return Ok(());
        }
    }
    if let Some(deadline) = job.spec.deadline_ms {
        if timestamp_ms >= deadline {
            job.status = SchedulerJobStatus::NeedsAttention;
            job.eligibility = ineligible(
                SchedulerDeferredCode::DeadlineExpired,
                format!("job deadline {deadline} has expired"),
                None,
            );
            return Ok(());
        }
    }
    for dependency in &job.spec.dependencies {
        let dependency = jobs
            .get(dependency)
            .with_context(|| format!("scheduler dependency '{}' disappeared", dependency))?;
        if dependency.status == SchedulerJobStatus::Succeeded {
            continue;
        }
        if dependency.status.terminal() {
            job.status = SchedulerJobStatus::NeedsAttention;
            job.eligibility = ineligible(
                SchedulerDeferredCode::DependencyFailed,
                format!(
                    "dependency '{}' is terminal with status {:?}",
                    dependency.job_id, dependency.status
                ),
                None,
            );
        } else {
            job.status = SchedulerJobStatus::Waiting;
            job.eligibility = ineligible(
                SchedulerDeferredCode::DependencyPending,
                format!("dependency '{}' has not succeeded", dependency.job_id),
                None,
            );
        }
        return Ok(());
    }
    let run = runs.run(&job.run_id)?;
    if run.status != RunLifecycleStatus::Running {
        job.status = SchedulerJobStatus::NeedsAttention;
        job.eligibility = ineligible(
            SchedulerDeferredCode::RunNotReady,
            "linked Run is not active",
            None,
        );
        return Ok(());
    }
    if run
        .attempts
        .iter()
        .any(|attempt| attempt.status == AttemptLifecycleStatus::Running)
    {
        job.status = SchedulerJobStatus::Running;
        job.eligibility = ineligible(
            SchedulerDeferredCode::ActiveAttempt,
            "linked Run has an active Attempt",
            None,
        );
        return Ok(());
    }
    if run.worktree.is_some() && !run.attempts.is_empty() {
        job.status = SchedulerJobStatus::NeedsAttention;
        job.eligibility = ineligible(
            SchedulerDeferredCode::RunNotReady,
            "worktree Run retries require a new Run and explicit replan",
            None,
        );
        return Ok(());
    }
    if !job.budget_state.within_budget {
        job.status = SchedulerJobStatus::NeedsAttention;
        job.eligibility = ineligible(
            SchedulerDeferredCode::BudgetExhausted,
            format!(
                "job budget exhausted: {}",
                job.budget_state.exhausted_dimensions.join(",")
            ),
            None,
        );
        return Ok(());
    }
    if job.dispatches.len() >= job.spec.max_dispatches as usize {
        job.status = SchedulerJobStatus::NeedsAttention;
        job.eligibility = ineligible(
            SchedulerDeferredCode::DispatchLimit,
            format!("dispatch limit {} reached", job.spec.max_dispatches),
            None,
        );
        return Ok(());
    }
    if job.consecutive_failures >= job.spec.max_consecutive_failures {
        job.status = SchedulerJobStatus::NeedsAttention;
        job.eligibility = ineligible(
            SchedulerDeferredCode::FailureThreshold,
            format!(
                "{} consecutive failures reached threshold {}",
                job.consecutive_failures, job.spec.max_consecutive_failures
            ),
            None,
        );
        return Ok(());
    }
    job.status = if job.dispatches.is_empty() {
        SchedulerJobStatus::Ready
    } else {
        SchedulerJobStatus::RetryReady
    };
    job.eligibility = SchedulerEligibility {
        eligible: true,
        code: None,
        reason: "dependencies, time window, Run state, and budgets allow dispatch".to_owned(),
        next_eligible_at_ms: None,
    };
    Ok(())
}

fn scheduler_budget_state(
    job: &SchedulerJobView,
    run: &crate::AgentRunView,
) -> SchedulerBudgetState {
    let mut exhausted = run.budget_state.exhausted_dimensions.clone();
    if let Some(budget) = &job.spec.budget {
        if budget
            .max_total_tokens
            .is_some_and(|limit| run.consumption.total_tokens >= limit)
        {
            exhausted.push("job_tokens".to_owned());
        }
        if budget
            .max_duration_ms
            .is_some_and(|limit| run.consumption.duration_ms >= limit)
        {
            exhausted.push("job_duration".to_owned());
        }
        if budget
            .max_cost_microusd
            .is_some_and(|limit| run.consumption.estimated_cost_microusd >= limit)
        {
            exhausted.push("job_cost".to_owned());
        }
        if budget
            .max_attempts
            .is_some_and(|limit| run.attempts.len() >= limit as usize)
        {
            exhausted.push("job_attempts".to_owned());
        }
    }
    exhausted.sort();
    exhausted.dedup();
    SchedulerBudgetState {
        within_budget: exhausted.is_empty(),
        exhausted_dimensions: exhausted,
    }
}

fn select_job<'a>(
    report: &'a SchedulerReport,
    requested: Option<&str>,
) -> Result<&'a SchedulerJobView> {
    if let Some(job_id) = requested {
        return report.job(job_id);
    }
    report
        .jobs
        .iter()
        .find(|job| job.eligibility.eligible)
        .or_else(|| report.jobs.iter().find(|job| !job.status.terminal()))
        .context("scheduler queue is empty")
}

#[derive(Debug, Default)]
struct ActiveCounts {
    total: u64,
    by_home: BTreeMap<String, u64>,
    by_model: BTreeMap<String, u64>,
}

type SchedulerCandidateChoice = (
    RouteCandidateEvaluation,
    SchedulerSelectionMode,
    Vec<String>,
);
type SchedulerCandidateRejection = (SchedulerDeferredCode, String, Vec<String>);

fn active_counts(runs: &AgentRunReport) -> ActiveCounts {
    let mut counts = ActiveCounts::default();
    for attempt in runs
        .runs
        .iter()
        .flat_map(|run| &run.attempts)
        .filter(|attempt| attempt.status == AttemptLifecycleStatus::Running)
    {
        counts.total = counts.total.saturating_add(1);
        if let Some(home) = &attempt.identity.home_id {
            *counts.by_home.entry(home.to_ascii_lowercase()).or_default() += 1;
        }
        if let Some(model) = &attempt.identity.model {
            *counts
                .by_model
                .entry(model.to_ascii_lowercase())
                .or_default() += 1;
        }
    }
    counts
}

fn choose_scheduler_candidate(
    job: &SchedulerJobView,
    decision: &RouteDecision,
    policy: &SchedulerPolicy,
    active: &ActiveCounts,
    run: &crate::AgentRunView,
) -> std::result::Result<SchedulerCandidateChoice, SchedulerCandidateRejection> {
    let mut rejections = Vec::new();
    let mut ordered = Vec::<(&RouteCandidateEvaluation, SchedulerSelectionMode)>::new();
    if job.spec.candidate_preference.is_empty() {
        ordered.extend(
            decision
                .candidates
                .iter()
                .map(|candidate| (candidate, SchedulerSelectionMode::Dynamic)),
        );
    } else {
        for (index, candidate_id) in job.spec.candidate_preference.iter().enumerate() {
            match decision
                .candidates
                .iter()
                .find(|candidate| candidate.candidate_id == *candidate_id)
            {
                Some(candidate) => ordered.push((
                    candidate,
                    if index == 0 {
                        SchedulerSelectionMode::Preferred
                    } else {
                        SchedulerSelectionMode::Fallback
                    },
                )),
                None => rejections.push(format!(
                    "preferred candidate '{}' is absent from route policy",
                    candidate_id
                )),
            }
        }
    }
    let last = run.attempts.last();
    let mut concurrency_code = None;
    let mut budget_rejected = false;
    for (candidate, mode) in ordered {
        if !candidate.eligible {
            rejections.push(format!(
                "{}: {}",
                candidate.candidate_id,
                candidate.rejection_reasons.join(" | ")
            ));
            continue;
        }
        if let Some(last) = last {
            if same_execution_identity(&last.identity, &candidate.identity) {
                if last
                    .failure
                    .as_ref()
                    .is_some_and(|failure| !failure.retryable)
                {
                    rejections.push(format!(
                        "{}: latest failure is non-retryable for the same identity",
                        candidate.candidate_id
                    ));
                    continue;
                }
                if let Some(failure) = last
                    .failure
                    .as_ref()
                    .filter(|failure| requires_identity_migration(&failure.code))
                {
                    rejections.push(format!(
                        "{}: latest '{}' failure requires a different identity",
                        candidate.candidate_id, failure.code
                    ));
                    continue;
                }
            }
        }
        if let Some(reason) = projected_budget_rejection(job, run, candidate) {
            budget_rejected = true;
            rejections.push(format!("{}: {reason}", candidate.candidate_id));
            continue;
        }
        let home = candidate.identity.home_id.as_deref().unwrap_or_default();
        let home_active = active
            .by_home
            .get(&home.to_ascii_lowercase())
            .copied()
            .unwrap_or_default();
        let home_limit = policy.home_limit(home);
        if home_active >= u64::from(home_limit) {
            concurrency_code = Some(SchedulerDeferredCode::HomeConcurrency);
            rejections.push(format!(
                "{}: Home '{}' active load {}/{} reached scheduler limit",
                candidate.candidate_id, home, home_active, home_limit
            ));
            continue;
        }
        let model = candidate.identity.model.as_deref().unwrap_or_default();
        let model_active = active
            .by_model
            .get(&model.to_ascii_lowercase())
            .copied()
            .unwrap_or_default();
        let model_limit = policy.model_limit(model);
        if model_active >= u64::from(model_limit) {
            concurrency_code = Some(SchedulerDeferredCode::ModelConcurrency);
            rejections.push(format!(
                "{}: model '{}' active load {}/{} reached scheduler limit",
                candidate.candidate_id, model, model_active, model_limit
            ));
            continue;
        }
        return Ok((candidate.clone(), mode, rejections));
    }
    Err((
        if budget_rejected {
            SchedulerDeferredCode::BudgetExhausted
        } else {
            concurrency_code.unwrap_or(SchedulerDeferredCode::NoEligibleRoute)
        },
        if decision.selected.is_none() {
            decision.decision_reason.clone()
        } else {
            "scheduler constraints rejected every eligible route candidate".to_owned()
        },
        rejections,
    ))
}

fn projected_budget_rejection(
    job: &SchedulerJobView,
    run: &crate::AgentRunView,
    candidate: &RouteCandidateEvaluation,
) -> Option<String> {
    let estimated_tokens = job
        .spec
        .route_request
        .estimated_context_tokens
        .saturating_add(job.spec.route_request.estimated_output_tokens);
    let projected_tokens = run
        .consumption
        .total_tokens
        .saturating_add(estimated_tokens);
    let projected_cost = run
        .consumption
        .estimated_cost_microusd
        .saturating_add(candidate.estimated_cost_microusd);
    let projected_attempts = run.attempts.len().saturating_add(1);
    for (scope, budget) in [
        ("Run", run.budget.as_ref()),
        ("job", job.spec.budget.as_ref()),
    ] {
        let Some(budget) = budget else {
            continue;
        };
        if budget
            .max_total_tokens
            .is_some_and(|limit| projected_tokens > limit)
        {
            return Some(format!(
                "{scope} projected token use {projected_tokens} exceeds its limit"
            ));
        }
        if budget
            .max_cost_microusd
            .is_some_and(|limit| projected_cost > limit)
        {
            return Some(format!(
                "{scope} projected cost {projected_cost} microusd exceeds its limit"
            ));
        }
        if budget
            .max_attempts
            .is_some_and(|limit| projected_attempts > limit as usize)
        {
            return Some(format!(
                "{scope} projected Attempt count {projected_attempts} exceeds its limit"
            ));
        }
        if budget.max_duration_ms.is_some_and(|limit| {
            run.consumption
                .duration_ms
                .saturating_add(job.spec.lease_duration_ms)
                > limit
        }) {
            return Some(format!(
                "{scope} remaining duration cannot cover the initial lease"
            ));
        }
    }
    None
}

fn duration_budget_deadline(
    job: &SchedulerJobView,
    run: &crate::AgentRunView,
    dispatched_at_ms: u64,
) -> Option<u64> {
    [run.budget.as_ref(), job.spec.budget.as_ref()]
        .into_iter()
        .flatten()
        .filter_map(|budget| {
            budget
                .max_duration_ms
                .map(|limit| limit.saturating_sub(run.consumption.duration_ms))
        })
        .min()
        .and_then(|remaining| dispatched_at_ms.checked_add(remaining))
}

fn requires_identity_migration(code: &str) -> bool {
    let code = code.trim().to_ascii_lowercase();
    code.contains("rate_limit")
        || code.contains("quota")
        || code.contains("auth_invalid")
        || code.contains("home_offline")
        || code.contains("provider_unavailable")
}

fn scheduler_transition(
    run: &crate::AgentRunView,
    identity: &ExecutionIdentity,
) -> Result<AttemptTransition> {
    let Some(previous) = run.attempts.last() else {
        return Ok(AttemptTransition {
            kind: AttemptTransitionKind::Initial,
            from_attempt_id: None,
        });
    };
    if previous.status == AttemptLifecycleStatus::Running {
        bail!("cannot dispatch while a previous Attempt is active");
    }
    let kind = if same_execution_identity(&previous.identity, identity) {
        if !previous
            .failure
            .as_ref()
            .is_some_and(|failure| failure.retryable)
        {
            bail!("same-identity scheduler retry requires a retryable failure");
        }
        AttemptTransitionKind::Retry
    } else {
        AttemptTransitionKind::Migration
    };
    Ok(AttemptTransition {
        kind,
        from_attempt_id: Some(previous.attempt_id.clone()),
    })
}

fn same_execution_identity(left: &ExecutionIdentity, right: &ExecutionIdentity) -> bool {
    left.home_id == right.home_id
        && left.account_id == right.account_id
        && left.model == right.model
}

fn deferred_event(
    event_id: String,
    timestamp_ms: u64,
    job: &SchedulerJobView,
    code: SchedulerDeferredCode,
    reason: String,
    next_eligible_at_ms: Option<u64>,
    route_decision_id: Option<String>,
) -> ObservabilityEvent {
    let mut event = scheduler_event(
        event_id,
        timestamp_ms,
        ObservabilityEventType::SchedulerDispatchDeferred,
        ObservabilityStatus::Degraded,
        &job.task_id,
        &job.run_id,
        None,
    );
    event.details.scheduler_deferred = Some(SchedulerDeferredDetails {
        job_id: job.job_id.clone(),
        decision_id: generate_id("schedule-decision"),
        code,
        reason,
        next_eligible_at_ms,
        route_decision_id,
    });
    event
}

fn scheduler_event(
    event_id: String,
    timestamp_ms: u64,
    event_type: ObservabilityEventType,
    status: ObservabilityStatus,
    task_id: &str,
    run_id: &str,
    attempt_id: Option<&str>,
) -> ObservabilityEvent {
    ObservabilityEvent {
        schema_version: OBSERVABILITY_EVENT_SCHEMA_VERSION.to_owned(),
        event_id,
        timestamp_ms,
        event_type,
        status,
        trace: TraceContext {
            task_id: Some(task_id.to_owned()),
            run_id: Some(run_id.to_owned()),
            attempt_id: attempt_id.map(ToOwned::to_owned),
            ..TraceContext::default()
        },
        identity: ExecutionIdentity::default(),
        usage: UsageDelta::default(),
        parent_event_id: None,
        tool_name: None,
        artifact_kind: None,
        verification_kind: None,
        failure: None,
        health: None,
        details: EventDetails::default(),
        safety: EventSafety::default(),
    }
}

fn combined(
    existing: &[ObservabilityEvent],
    transaction: &[ObservabilityEvent],
) -> Vec<ObservabilityEvent> {
    let mut combined = Vec::with_capacity(existing.len() + transaction.len());
    combined.extend_from_slice(existing);
    combined.extend_from_slice(transaction);
    combined
}

fn ineligible(
    code: SchedulerDeferredCode,
    reason: impl Into<String>,
    next_eligible_at_ms: Option<u64>,
) -> SchedulerEligibility {
    SchedulerEligibility {
        eligible: false,
        code: Some(code),
        reason: reason.into(),
        next_eligible_at_ms,
    }
}

fn priority_rank(priority: SchedulerPriority) -> u8 {
    match priority {
        SchedulerPriority::Background => 0,
        SchedulerPriority::Low => 1,
        SchedulerPriority::Normal => 2,
        SchedulerPriority::High => 3,
        SchedulerPriority::Critical => 4,
    }
}

fn validate_job_spec(job: &SchedulerJobSpec) -> Result<()> {
    let event = scheduler_event(
        "evt-scheduler-validation".to_owned(),
        0,
        ObservabilityEventType::SchedulerJobQueued,
        ObservabilityStatus::Started,
        "task-validation",
        &job.run_id,
        None,
    );
    let mut event = event;
    event.details.scheduler_job = Some(job.clone());
    event.validate()
}

fn validate_limits(name: &str, limits: &[SchedulerConcurrencyLimit]) -> Result<()> {
    if limits.len() > 256 {
        bail!("{name} cannot contain more than 256 entries");
    }
    let mut keys = BTreeSet::new();
    for limit in limits {
        validate_label(name, &limit.key)?;
        if limit.max_active == 0 {
            bail!("{name} maxActive must be positive");
        }
        if !keys.insert(limit.key.to_ascii_lowercase()) {
            bail!("{name} repeats key '{}'", limit.key);
        }
    }
    Ok(())
}

fn lookup_limit(limits: &[SchedulerConcurrencyLimit], key: &str, default: u16) -> u16 {
    limits
        .iter()
        .find(|limit| limit.key.eq_ignore_ascii_case(key))
        .map_or(default, |limit| limit.max_active)
}

fn validate_identifier(name: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._:-/@".contains(character))
    {
        bail!("{name} contains unsupported characters");
    }
    Ok(())
}

fn validate_label(name: &str, value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 256
        || value
            .chars()
            .any(|character| matches!(character, '\r' | '\n'))
    {
        bail!("{name} is empty or too long");
    }
    Ok(())
}

fn validate_reason(reason: &str) -> Result<()> {
    let value = reason.trim();
    let lower = value.to_ascii_lowercase();
    if value.is_empty()
        || value.len() > 96
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '-')
        })
        || lower.starts_with("sk-")
        || lower.starts_with("ghp_")
        || lower.starts_with("github_pat_")
        || lower.starts_with("xox")
        || lower.starts_with("akia")
        || lower.starts_with("aiza")
        || lower.starts_with("eyj")
        || [
            "password",
            "secret",
            "token",
            "cookie",
            "credential",
            "authorization",
            "api_key",
            "apikey",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
    {
        bail!("scheduler reason must be a safe 1..=96 byte operational reason code");
    }
    Ok(())
}

fn scheduler_safety() -> SafetySummary {
    SafetySummary {
        secrets_redacted: true,
        includes_secrets: false,
        reads_auth_contents: false,
        includes_local_paths: true,
    }
}

fn default_max_active_jobs() -> u16 {
    8
}

fn default_home_concurrency() -> u16 {
    1
}

fn default_model_concurrency() -> u16 {
    2
}

fn default_max_lease_ms() -> u64 {
    86_400_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AgentRunStore, RouteCandidateProfile, RouteRequest, RouteScoreWeights, RouteTaskKind,
        RunBudget, ROUTE_POLICY_SCHEMA_VERSION, ROUTE_REQUEST_SCHEMA_VERSION,
        SCHEDULER_JOB_SCHEMA_VERSION,
    };
    use std::thread;
    use std::time::Duration;
    use tempfile::TempDir;

    fn candidate(id: &str, home: &str, model: &str) -> RouteCandidateProfile {
        RouteCandidateProfile {
            candidate_id: id.to_owned(),
            enabled: true,
            home_id: home.to_owned(),
            home_alias: Some(format!("@{home}")),
            account_id: Some(format!("account-{home}")),
            provider: Some("test".to_owned()),
            model: model.to_owned(),
            specialties: vec!["coding".to_owned()],
            capabilities: vec!["coding".to_owned()],
            security_domains: Vec::new(),
            max_context_tokens: 128_000,
            model_strength_basis_points: 8_000,
            input_cost_microusd_per_million_tokens: 100,
            output_cost_microusd_per_million_tokens: 200,
            max_concurrent_runs: 10,
            baseline_duration_ms: Some(1_000),
        }
    }

    fn route_policy() -> RoutePolicy {
        let mut preferred = candidate("preferred", "home-preferred", "model-a");
        preferred.input_cost_microusd_per_million_tokens = 1_000_000;
        preferred.output_cost_microusd_per_million_tokens = 2_000_000;
        RoutePolicy {
            schema_version: ROUTE_POLICY_SCHEMA_VERSION.to_owned(),
            policy_id: "scheduler-test-route".to_owned(),
            revision: 1,
            weights: RouteScoreWeights::default(),
            task_rules: Vec::new(),
            candidates: vec![preferred, candidate("fallback", "home-fallback", "model-b")],
            history_prior_success_basis_points: 5_000,
            history_prior_attempts: 2,
            unknown_quota_basis_points: 5_000,
            unknown_health_basis_points: 5_000,
        }
    }

    fn scheduler_policy(max_active: u16, home_limit: u16) -> SchedulerPolicy {
        SchedulerPolicy {
            schema_version: SCHEDULER_POLICY_SCHEMA_VERSION.to_owned(),
            policy_id: "scheduler-test".to_owned(),
            revision: 1,
            max_active_jobs: max_active,
            default_home_concurrency: home_limit,
            default_model_concurrency: 10,
            home_concurrency: Vec::new(),
            model_concurrency: Vec::new(),
            max_lease_ms: 10_000,
        }
    }

    fn homes(preferred_available: bool) -> Vec<RouteHomeState> {
        vec![
            RouteHomeState {
                home_id: "home-preferred".to_owned(),
                home_alias: Some("@home-preferred".to_owned()),
                available: preferred_available,
            },
            RouteHomeState {
                home_id: "home-fallback".to_owned(),
                home_alias: Some("@home-fallback".to_owned()),
                available: true,
            },
        ]
    }

    fn create_run(path: &Path, suffix: &str, budget: Option<RunBudget>) -> (String, String) {
        let store = AgentRunStore::new(path.to_path_buf());
        let task_id = format!("task-{suffix}");
        let run_id = format!("run-{suffix}");
        store
            .apply(AgentRunAction::CreateTask {
                task_id: task_id.clone(),
                label: format!("Scheduler test {suffix}"),
                kind: Some("coding".to_owned()),
            })
            .expect("create task");
        store
            .apply(AgentRunAction::StartRun {
                task_id: task_id.clone(),
                run_id: run_id.clone(),
                budget,
            })
            .expect("start run");
        (task_id, run_id)
    }

    fn job(job_id: &str, run_id: &str, priority: SchedulerPriority) -> SchedulerJobSpec {
        SchedulerJobSpec {
            schema_version: SCHEDULER_JOB_SCHEMA_VERSION.to_owned(),
            job_id: job_id.to_owned(),
            run_id: run_id.to_owned(),
            priority,
            dependencies: Vec::new(),
            not_before_ms: None,
            deadline_ms: None,
            attempt_timeout_ms: 10_000,
            lease_duration_ms: 100,
            max_dispatches: 4,
            max_consecutive_failures: 2,
            budget: None,
            route_request: RouteRequest {
                schema_version: ROUTE_REQUEST_SCHEMA_VERSION.to_owned(),
                request_id: format!("request-{job_id}"),
                task_kind: RouteTaskKind::SimpleCode,
                estimated_context_tokens: 2_000,
                estimated_output_tokens: 500,
                required_capabilities: vec!["coding".to_owned()],
                preferred_specialties: Vec::new(),
                sensitive_directory: false,
                required_security_domain: None,
                locked_home: None,
                locked_model: None,
                max_estimated_cost_microusd: None,
                allow_degraded: false,
            },
            candidate_preference: vec!["preferred".to_owned(), "fallback".to_owned()],
        }
    }

    #[test]
    fn priority_time_and_dependencies_are_projected_deterministically() {
        let temp = TempDir::new().expect("temp");
        let path = temp.path().join("events.jsonl");
        let (_, low_run) = create_run(&path, "low", None);
        let (_, high_run) = create_run(&path, "high", None);
        let (_, dependent_run) = create_run(&path, "dependent", None);
        let store = SchedulerStore::new(path);
        store
            .enqueue(job("job-low", &low_run, SchedulerPriority::Low))
            .expect("enqueue low");
        let mut high = job("job-high", &high_run, SchedulerPriority::Critical);
        high.not_before_ms = Some(now_ms().expect("time") + 60_000);
        store.enqueue(high).expect("enqueue high");
        let mut dependent = job("job-dependent", &dependent_run, SchedulerPriority::High);
        dependent.dependencies = vec!["job-low".to_owned()];
        store.enqueue(dependent).expect("enqueue dependency");

        let report = store.report().expect("report");
        assert_eq!(report.jobs[0].job_id, "job-high");
        assert_eq!(
            report.job("job-high").expect("high").eligibility.code,
            Some(SchedulerDeferredCode::NotBefore)
        );
        assert_eq!(
            report
                .job("job-dependent")
                .expect("dependent")
                .eligibility
                .code,
            Some(SchedulerDeferredCode::DependencyPending)
        );
        assert!(report.job("job-low").expect("low").eligibility.eligible);
    }

    #[test]
    fn unavailable_preference_falls_back_and_records_why() {
        let temp = TempDir::new().expect("temp");
        let path = temp.path().join("events.jsonl");
        let (_, run_id) = create_run(&path, "fallback", None);
        let store = SchedulerStore::new(path);
        store
            .enqueue(job("job-fallback", &run_id, SchedulerPriority::Normal))
            .expect("enqueue");

        let report = store
            .dispatch(
                &scheduler_policy(4, 4),
                &route_policy(),
                &homes(false),
                Some("job-fallback"),
            )
            .expect("dispatch");
        assert!(report.dispatched);
        let dispatch = report.job.dispatches.last().expect("dispatch view");
        assert_eq!(
            dispatch.details.selection_mode,
            SchedulerSelectionMode::Fallback
        );
        assert_eq!(dispatch.details.selected_candidate_id, "fallback");
        assert!(dispatch
            .details
            .scheduler_rejections
            .iter()
            .any(|reason| reason.contains("unavailable")));
    }

    #[test]
    fn projected_cost_uses_a_cheaper_fallback() {
        let temp = TempDir::new().expect("temp");
        let path = temp.path().join("events.jsonl");
        let (_, run_id) = create_run(&path, "budget-fallback", None);
        let store = SchedulerStore::new(path);
        let mut spec = job("job-budget-fallback", &run_id, SchedulerPriority::Normal);
        spec.budget = Some(RunBudget {
            max_total_tokens: None,
            max_duration_ms: None,
            max_cost_microusd: Some(100),
            max_attempts: None,
        });
        store.enqueue(spec).expect("enqueue");

        let report = store
            .dispatch(
                &scheduler_policy(4, 4),
                &route_policy(),
                &homes(true),
                Some("job-budget-fallback"),
            )
            .expect("dispatch");
        assert!(report.dispatched);
        let dispatch = report.job.dispatches.last().expect("dispatch view");
        assert_eq!(
            dispatch.details.selection_mode,
            SchedulerSelectionMode::Fallback
        );
        assert!(dispatch
            .details
            .scheduler_rejections
            .iter()
            .any(|reason| reason.contains("projected cost")));
    }

    #[test]
    fn rate_limited_identity_migrates_on_the_next_dispatch() {
        let temp = TempDir::new().expect("temp");
        let path = temp.path().join("events.jsonl");
        let (_, run_id) = create_run(&path, "rate-limit", None);
        let store = SchedulerStore::new(path.clone());
        store
            .enqueue(job("job-rate-limit", &run_id, SchedulerPriority::Normal))
            .expect("enqueue");
        let first = store
            .dispatch(
                &scheduler_policy(4, 4),
                &route_policy(),
                &homes(true),
                Some("job-rate-limit"),
            )
            .expect("first dispatch");
        let attempt_id = first.job.dispatches[0].details.attempt_id.clone();
        AgentRunStore::new(path)
            .apply(AgentRunAction::FailAttempt {
                run_id,
                attempt_id,
                thread_id: None,
                status: ObservabilityStatus::Failed,
                usage: UsageDelta::default(),
                failure: FailureInfo {
                    code: "rate_limited".to_owned(),
                    phase: Some("provider".to_owned()),
                    reason: "provider requested a backoff".to_owned(),
                    retryable: true,
                },
            })
            .expect("fail attempt");

        let retried = store
            .dispatch(
                &scheduler_policy(4, 4),
                &route_policy(),
                &homes(true),
                Some("job-rate-limit"),
            )
            .expect("migration dispatch");
        assert!(retried.dispatched);
        let dispatch = retried.job.dispatches.last().expect("dispatch view");
        assert_eq!(dispatch.details.selected_candidate_id, "fallback");
        assert!(dispatch
            .details
            .scheduler_rejections
            .iter()
            .any(|reason| reason.contains("requires a different identity")));
    }

    #[test]
    fn concurrency_gate_defers_second_home_claim() {
        let temp = TempDir::new().expect("temp");
        let path = temp.path().join("events.jsonl");
        let (_, run_one) = create_run(&path, "one", None);
        let (_, run_two) = create_run(&path, "two", None);
        let store = SchedulerStore::new(path);
        let mut first = job("job-one", &run_one, SchedulerPriority::High);
        first.candidate_preference = vec!["preferred".to_owned()];
        let mut second = job("job-two", &run_two, SchedulerPriority::Normal);
        second.candidate_preference = vec!["preferred".to_owned()];
        store.enqueue(first).expect("enqueue first");
        store.enqueue(second).expect("enqueue second");
        let policy = scheduler_policy(4, 1);
        store
            .dispatch(&policy, &route_policy(), &homes(true), Some("job-one"))
            .expect("dispatch first");
        let deferred = store
            .dispatch(&policy, &route_policy(), &homes(true), Some("job-two"))
            .expect("defer second");
        assert!(!deferred.dispatched);
        assert_eq!(
            deferred
                .job
                .last_deferred
                .as_ref()
                .map(|value| value.details.code),
            Some(SchedulerDeferredCode::HomeConcurrency)
        );
    }

    #[test]
    fn expired_lease_requires_recovery_before_retry() {
        let temp = TempDir::new().expect("temp");
        let path = temp.path().join("events.jsonl");
        let (_, run_id) = create_run(&path, "recovery", None);
        let store = SchedulerStore::new(path.clone());
        let mut spec = job("job-recovery", &run_id, SchedulerPriority::Normal);
        spec.lease_duration_ms = 1;
        store.enqueue(spec).expect("enqueue");
        store
            .dispatch(
                &scheduler_policy(4, 4),
                &route_policy(),
                &homes(true),
                Some("job-recovery"),
            )
            .expect("dispatch");
        thread::sleep(Duration::from_millis(3));
        let tick = store.tick().expect("tick");
        assert_eq!(tick.recovery_required, ["job-recovery"]);
        assert_eq!(
            store
                .report()
                .expect("report")
                .job("job-recovery")
                .expect("job")
                .status,
            SchedulerJobStatus::RecoveryRequired
        );

        store
            .resolve_recovery_for_retry(
                "job-recovery",
                UsageDelta {
                    duration_ms: 3,
                    ..UsageDelta::default()
                },
                "worker_heartbeat_lost",
            )
            .expect("resolve");
        let retried = store
            .dispatch(
                &scheduler_policy(4, 4),
                &route_policy(),
                &homes(true),
                Some("job-recovery"),
            )
            .expect("retry");
        assert!(retried.dispatched);
        assert_eq!(retried.job.dispatches.len(), 2);
    }

    #[test]
    fn parallel_dispatchers_only_claim_a_job_once_and_empty_tick_is_ok() {
        let temp = TempDir::new().expect("temp");
        let path = temp.path().join("events.jsonl");
        let (_, run_id) = create_run(&path, "atomic", None);
        let store = SchedulerStore::new(path.clone());
        let mut spec = job("job-atomic", &run_id, SchedulerPriority::Critical);
        spec.lease_duration_ms = 10_000;
        store.enqueue(spec).expect("enqueue");
        let mut handles = Vec::new();
        for _ in 0..2 {
            let store = SchedulerStore::new(path.clone());
            let policy = scheduler_policy(4, 4);
            let route = route_policy();
            let homes = homes(true);
            handles.push(thread::spawn(move || {
                store
                    .dispatch(&policy, &route, &homes, Some("job-atomic"))
                    .expect("parallel dispatch")
                    .dispatched
            }));
        }
        let claimed = handles
            .into_iter()
            .map(|handle| handle.join().expect("join"))
            .filter(|dispatched| *dispatched)
            .count();
        assert_eq!(claimed, 1);
        assert_eq!(
            store
                .report()
                .expect("report")
                .job("job-atomic")
                .expect("job")
                .dispatches
                .len(),
            1
        );
        let tick = store.tick().expect("empty tick");
        assert!(tick.event_ids.is_empty());
    }

    #[test]
    fn cancellation_closes_active_attempt_and_run() {
        let temp = TempDir::new().expect("temp");
        let path = temp.path().join("events.jsonl");
        let (_, run_id) = create_run(&path, "cancel", None);
        let store = SchedulerStore::new(path.clone());
        store
            .enqueue(job("job-cancel", &run_id, SchedulerPriority::Normal))
            .expect("enqueue");
        store
            .dispatch(
                &scheduler_policy(4, 4),
                &route_policy(),
                &homes(true),
                Some("job-cancel"),
            )
            .expect("dispatch");
        let mutation = store
            .cancel(
                "job-cancel",
                UsageDelta {
                    input_tokens: 10,
                    output_tokens: 2,
                    duration_ms: 5,
                    estimated_cost_microusd: 1,
                    ..UsageDelta::default()
                },
                "operator_cancelled",
            )
            .expect("cancel");
        assert_eq!(mutation.job.status, SchedulerJobStatus::Cancelled);
        let runs = AgentRunStore::new(path).report().expect("runs");
        assert_eq!(
            runs.run(&run_id).expect("run").status,
            RunLifecycleStatus::Cancelled
        );
    }

    #[test]
    fn running_jobs_cannot_be_paused_and_reasons_are_codes() {
        let temp = TempDir::new().expect("temp");
        let path = temp.path().join("events.jsonl");
        let (_, run_id) = create_run(&path, "pause-guard", None);
        let store = SchedulerStore::new(path);
        store
            .enqueue(job("job-pause-guard", &run_id, SchedulerPriority::Normal))
            .expect("enqueue");
        store
            .dispatch(
                &scheduler_policy(4, 4),
                &route_policy(),
                &homes(true),
                Some("job-pause-guard"),
            )
            .expect("dispatch");
        assert!(store
            .pause("job-pause-guard", "maintenance", false)
            .expect_err("running pause must fail")
            .to_string()
            .contains("can be paused"));
        assert!(validate_reason("password=demo-secret").is_err());
        assert!(validate_reason("copy the raw response").is_err());
        validate_reason("capacity_restored").expect("safe reason code");
    }

    #[test]
    fn renew_requires_the_active_unexpired_lease_and_bounded_window() {
        let temp = TempDir::new().expect("temp");
        let path = temp.path().join("events.jsonl");
        let (_, run_id) = create_run(&path, "renew", None);
        let store = SchedulerStore::new(path);
        let spec = job("job-renew", &run_id, SchedulerPriority::Normal);
        store.enqueue(spec).expect("enqueue");
        let dispatched_at = now_ms().expect("time");
        let dispatched = store
            .dispatch_at(
                &scheduler_policy(4, 4),
                &route_policy(),
                &homes(true),
                Some("job-renew"),
                dispatched_at,
            )
            .expect("dispatch");
        let details = dispatched.job.dispatches[0].details.clone();
        assert!(store
            .renew_lease_at(
                &scheduler_policy(4, 4),
                "job-renew",
                "wrong-dispatch",
                &details.lease_id,
                10,
                "worker_heartbeat",
                dispatched_at + 10,
            )
            .is_err());
        assert!(store
            .renew_lease_at(
                &scheduler_policy(4, 4),
                "job-renew",
                &details.dispatch_id,
                &details.lease_id,
                10,
                "worker_heartbeat",
                details.lease_expires_at_ms,
            )
            .expect_err("expired renewal must fail")
            .to_string()
            .contains("expired"));
        let mut narrow = scheduler_policy(4, 4);
        narrow.max_lease_ms = 100;
        assert!(store
            .renew_lease_at(
                &narrow,
                "job-renew",
                &details.dispatch_id,
                &details.lease_id,
                50,
                "worker_heartbeat",
                dispatched_at + 10,
            )
            .expect_err("window must be bounded")
            .to_string()
            .contains("maxLeaseMs"));
    }

    #[test]
    fn duration_budget_and_case_insensitive_home_limits_are_hard_gates() {
        let temp = TempDir::new().expect("temp");
        let path = temp.path().join("events.jsonl");
        let (_, budget_run) = create_run(
            &path,
            "duration",
            Some(RunBudget {
                max_total_tokens: None,
                max_duration_ms: Some(50),
                max_cost_microusd: None,
                max_attempts: None,
            }),
        );
        let scheduler = SchedulerStore::new(path.clone());
        scheduler
            .enqueue(job("job-duration", &budget_run, SchedulerPriority::Normal))
            .expect("enqueue duration");
        let deferred = scheduler
            .dispatch(
                &scheduler_policy(4, 4),
                &route_policy(),
                &homes(true),
                Some("job-duration"),
            )
            .expect("duration defer");
        assert!(!deferred.dispatched);
        assert_eq!(
            deferred
                .job
                .last_deferred
                .as_ref()
                .map(|value| value.details.code),
            Some(SchedulerDeferredCode::BudgetExhausted)
        );

        let runs = AgentRunStore::new(path.clone());
        let (_, active_run) = create_run(&path, "case-active", None);
        runs.apply(AgentRunAction::StartAttempt {
            run_id: active_run,
            attempt_id: "attempt-case-active".to_owned(),
            identity: ExecutionIdentity {
                home_id: Some("home-preferred".to_owned()),
                model: Some("model-a".to_owned()),
                ..ExecutionIdentity::default()
            },
            transition: AttemptTransition {
                kind: AttemptTransitionKind::Initial,
                from_attempt_id: None,
            },
            route_reason: None,
            route_decision_id: None,
        })
        .expect("active attempt");
        let (_, case_run) = create_run(&path, "case-scheduled", None);
        let mut case_job = job("job-case-limit", &case_run, SchedulerPriority::Normal);
        case_job.candidate_preference = vec!["preferred".to_owned()];
        scheduler.enqueue(case_job).expect("enqueue case");
        let mut route = route_policy();
        route.candidates[0].home_id = "HOME-PREFERRED".to_owned();
        let case_homes = vec![RouteHomeState {
            home_id: "HOME-PREFERRED".to_owned(),
            home_alias: Some("@home-preferred".to_owned()),
            available: true,
        }];
        let case_deferred = scheduler
            .dispatch(
                &scheduler_policy(8, 1),
                &route,
                &case_homes,
                Some("job-case-limit"),
            )
            .expect("case defer");
        assert!(!case_deferred.dispatched);
        assert_eq!(
            case_deferred
                .job
                .last_deferred
                .as_ref()
                .map(|value| value.details.code),
            Some(SchedulerDeferredCode::HomeConcurrency)
        );
    }

    #[test]
    fn truncated_scheduler_transaction_is_rejected() {
        let temp = TempDir::new().expect("temp");
        let path = temp.path().join("events.jsonl");
        let (_, run_id) = create_run(&path, "truncated", None);
        let store = SchedulerStore::new(path.clone());
        store
            .enqueue(job("job-truncated", &run_id, SchedulerPriority::Normal))
            .expect("enqueue");
        store
            .dispatch(
                &scheduler_policy(4, 4),
                &route_policy(),
                &homes(true),
                Some("job-truncated"),
            )
            .expect("dispatch");
        let mut events = ObservabilityStore::new(path).load().expect("events");
        assert_eq!(
            events.pop().expect("dispatch event").event_type,
            ObservabilityEventType::SchedulerDispatchDecided
        );
        let partial = temp.path().join("partial.jsonl");
        let mut contents = String::new();
        for event in events {
            contents.push_str(&serde_json::to_string(&event).expect("event JSON"));
            contents.push('\n');
        }
        fs::write(&partial, contents).expect("partial store");
        assert!(ObservabilityStore::new(partial)
            .load_verified()
            .expect_err("partial dispatch must fail")
            .to_string()
            .contains("no atomic scheduler dispatch"));
    }

    #[test]
    fn published_scheduler_examples_parse_and_validate() {
        parse_scheduler_job(include_str!("../../../examples/scheduler-job.example.json"))
            .expect("scheduler job example");
        let policy: SchedulerPolicy = serde_json::from_str(include_str!(
            "../../../examples/scheduler-policy.example.json"
        ))
        .expect("scheduler policy example JSON");
        policy.validate().expect("scheduler policy example");
    }
}
