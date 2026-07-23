use crate::{
    user_home, ExecutionIdentity, HealthSnapshot, ObservabilityEvent, ObservabilityEventType,
    ObservabilityStatus, SafetySummary,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub const ROUTE_REQUEST_SCHEMA_VERSION: &str = "codexhome.route-request.v1";
pub const ROUTE_POLICY_SCHEMA_VERSION: &str = "codexhome.route-policy.v1";
pub const ROUTE_DECISION_SCHEMA_VERSION: &str = "codexhome.route-decision.v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RouteTaskKind {
    SimpleCode,
    Architecture,
    LongDocument,
    Batch,
    Retrieval,
    Visual,
    General,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteRequest {
    pub schema_version: String,
    pub request_id: String,
    pub task_kind: RouteTaskKind,
    pub estimated_context_tokens: u64,
    pub estimated_output_tokens: u64,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    #[serde(default)]
    pub preferred_specialties: Vec<String>,
    #[serde(default)]
    pub sensitive_directory: bool,
    pub required_security_domain: Option<String>,
    pub locked_home: Option<String>,
    pub locked_model: Option<String>,
    pub max_estimated_cost_microusd: Option<u64>,
    #[serde(default)]
    pub allow_degraded: bool,
}

impl RouteRequest {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != ROUTE_REQUEST_SCHEMA_VERSION {
            bail!(
                "route request uses unsupported schemaVersion '{}'",
                self.schema_version
            );
        }
        validate_identifier("requestId", &self.request_id)?;
        if self.estimated_context_tokens == 0 {
            bail!("estimatedContextTokens must be positive");
        }
        if self.sensitive_directory && self.locked_home.is_none() {
            bail!("sensitiveDirectory requires lockedHome");
        }
        if self.max_estimated_cost_microusd == Some(0) {
            bail!("maxEstimatedCostMicrousd must be positive");
        }
        validate_optional_label("requiredSecurityDomain", &self.required_security_domain)?;
        validate_optional_label("lockedHome", &self.locked_home)?;
        validate_optional_label("lockedModel", &self.locked_model)?;
        validate_labels("requiredCapabilities", &self.required_capabilities)?;
        validate_labels("preferredSpecialties", &self.preferred_specialties)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoutePolicy {
    pub schema_version: String,
    pub policy_id: String,
    pub revision: u64,
    #[serde(default)]
    pub weights: RouteScoreWeights,
    #[serde(default)]
    pub task_rules: Vec<RouteTaskRule>,
    pub candidates: Vec<RouteCandidateProfile>,
    #[serde(default = "default_history_prior")]
    pub history_prior_success_basis_points: u16,
    #[serde(default = "default_history_prior_weight")]
    pub history_prior_attempts: u16,
    #[serde(default = "default_unknown_quota")]
    pub unknown_quota_basis_points: u16,
    #[serde(default = "default_unknown_health")]
    pub unknown_health_basis_points: u16,
}

impl RoutePolicy {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != ROUTE_POLICY_SCHEMA_VERSION {
            bail!(
                "route policy uses unsupported schemaVersion '{}'",
                self.schema_version
            );
        }
        validate_identifier("policyId", &self.policy_id)?;
        self.weights.validate("weights")?;
        validate_basis_points(
            "historyPriorSuccessBasisPoints",
            self.history_prior_success_basis_points,
        )?;
        validate_basis_points("unknownQuotaBasisPoints", self.unknown_quota_basis_points)?;
        validate_basis_points("unknownHealthBasisPoints", self.unknown_health_basis_points)?;
        if self.candidates.is_empty() {
            bail!("route policy must define at least one candidate");
        }
        if self.candidates.len() > 256 {
            bail!("route policy cannot define more than 256 candidates");
        }
        let mut candidate_ids = BTreeSet::new();
        for candidate in &self.candidates {
            candidate.validate()?;
            if !candidate_ids.insert(candidate.candidate_id.as_str()) {
                bail!("duplicate candidateId '{}'", candidate.candidate_id);
            }
        }
        let mut task_kinds = BTreeSet::new();
        for rule in &self.task_rules {
            rule.validate()?;
            if !task_kinds.insert(rule.task_kind) {
                bail!("duplicate task rule for {:?}", rule.task_kind);
            }
        }
        Ok(())
    }

    fn rule(&self, task_kind: RouteTaskKind) -> Option<&RouteTaskRule> {
        self.task_rules
            .iter()
            .find(|rule| rule.task_kind == task_kind)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteTaskRule {
    pub task_kind: RouteTaskKind,
    #[serde(default)]
    pub preferred_specialties: Vec<String>,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    pub minimum_model_strength_basis_points: Option<u16>,
    pub weights: Option<RouteScoreWeights>,
}

impl RouteTaskRule {
    fn validate(&self) -> Result<()> {
        validate_labels("taskRule.preferredSpecialties", &self.preferred_specialties)?;
        validate_labels("taskRule.requiredCapabilities", &self.required_capabilities)?;
        if let Some(value) = self.minimum_model_strength_basis_points {
            validate_basis_points("minimumModelStrengthBasisPoints", value)?;
        }
        if let Some(weights) = &self.weights {
            weights.validate("taskRule.weights")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteCandidateProfile {
    pub candidate_id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub home_id: String,
    pub home_alias: Option<String>,
    pub account_id: Option<String>,
    pub provider: Option<String>,
    pub model: String,
    #[serde(default)]
    pub specialties: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub security_domains: Vec<String>,
    pub max_context_tokens: u64,
    pub model_strength_basis_points: u16,
    pub input_cost_microusd_per_million_tokens: u64,
    pub output_cost_microusd_per_million_tokens: u64,
    #[serde(default = "default_max_concurrent_runs")]
    pub max_concurrent_runs: u16,
    pub baseline_duration_ms: Option<u64>,
}

impl RouteCandidateProfile {
    fn validate(&self) -> Result<()> {
        validate_identifier("candidateId", &self.candidate_id)?;
        validate_identifier("homeId", &self.home_id)?;
        validate_optional_label("homeAlias", &self.home_alias)?;
        if let Some(account_id) = &self.account_id {
            validate_identifier("accountId", account_id)?;
        }
        validate_optional_label("provider", &self.provider)?;
        validate_label("model", &self.model)?;
        validate_labels("specialties", &self.specialties)?;
        validate_labels("capabilities", &self.capabilities)?;
        validate_labels("securityDomains", &self.security_domains)?;
        if self.max_context_tokens == 0 {
            bail!(
                "candidate '{}' maxContextTokens must be positive",
                self.candidate_id
            );
        }
        validate_basis_points("modelStrengthBasisPoints", self.model_strength_basis_points)?;
        if self.max_concurrent_runs == 0 {
            bail!(
                "candidate '{}' maxConcurrentRuns must be positive",
                self.candidate_id
            );
        }
        if self.baseline_duration_ms == Some(0) {
            bail!(
                "candidate '{}' baselineDurationMs must be positive",
                self.candidate_id
            );
        }
        Ok(())
    }

    pub fn identity(&self) -> ExecutionIdentity {
        ExecutionIdentity {
            home_id: Some(self.home_id.clone()),
            home_alias: self.home_alias.clone(),
            account_id: self.account_id.clone(),
            provider: self.provider.clone(),
            model: Some(self.model.clone()),
            agent_role: None,
        }
    }

    fn matches_home(&self, value: &str) -> bool {
        self.home_id.eq_ignore_ascii_case(value)
            || self
                .home_alias
                .as_deref()
                .is_some_and(|alias| alias.eq_ignore_ascii_case(value))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteScoreWeights {
    pub task_fit: u16,
    pub model_strength: u16,
    pub context_headroom: u16,
    pub quota: u16,
    pub historical_success: u16,
    pub speed: u16,
    pub cost: u16,
    pub load: u16,
    pub health: u16,
}

impl Default for RouteScoreWeights {
    fn default() -> Self {
        Self {
            task_fit: 1_800,
            model_strength: 1_400,
            context_headroom: 900,
            quota: 1_000,
            historical_success: 1_500,
            speed: 900,
            cost: 1_000,
            load: 900,
            health: 600,
        }
    }
}

impl RouteScoreWeights {
    fn validate(&self, name: &str) -> Result<()> {
        let total = self.values().into_iter().map(u32::from).sum::<u32>();
        if total != 10_000 {
            bail!("{name} must sum to 10000 basis points; found {total}");
        }
        Ok(())
    }

    fn values(&self) -> [u16; 9] {
        [
            self.task_fit,
            self.model_strength,
            self.context_headroom,
            self.quota,
            self.historical_success,
            self.speed,
            self.cost,
            self.load,
            self.health,
        ]
    }
}

#[derive(Debug, Clone)]
pub struct RouteHomeState {
    pub home_id: String,
    pub home_alias: Option<String>,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteDecision {
    pub ok: bool,
    pub schema_version: &'static str,
    pub request_id: String,
    pub task_kind: RouteTaskKind,
    pub policy_id: String,
    pub policy_revision: u64,
    pub evaluated_at_timestamp_ms: u64,
    pub observed_event_count: usize,
    pub request: RouteRequest,
    pub selected: Option<RouteSelection>,
    pub evaluated_candidates: usize,
    pub eligible_candidates: usize,
    pub candidates: Vec<RouteCandidateEvaluation>,
    pub decision_reason: String,
    pub locks: RouteLocks,
    pub safety: SafetySummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteSelection {
    pub candidate_id: String,
    pub identity: ExecutionIdentity,
    pub total_score_basis_points: u16,
    pub estimated_cost_microusd: u64,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteLocks {
    pub home: Option<String>,
    pub model: Option<String>,
    pub sensitive_directory: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteCandidateEvaluation {
    pub candidate_id: String,
    pub identity: ExecutionIdentity,
    pub eligible: bool,
    pub total_score_basis_points: Option<u16>,
    pub estimated_cost_microusd: u64,
    pub historical_attempts: u64,
    pub historical_success_basis_points: u16,
    pub active_attempts: u64,
    pub quota_remaining_basis_points: Option<u16>,
    pub components: Vec<RouteScoreComponent>,
    pub reasons: Vec<String>,
    pub rejection_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteScoreComponent {
    pub name: String,
    pub score_basis_points: u16,
    pub weight_basis_points: u16,
    pub weighted_points: u16,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct RoutePolicyStore {
    path: PathBuf,
}

impl RoutePolicyStore {
    pub fn from_environment(override_path: Option<PathBuf>) -> Result<Self> {
        let home = user_home().context("cannot discover the current user home directory")?;
        let path = override_path
            .or_else(|| env::var_os("CODEXHOME_ROUTE_POLICY").map(PathBuf::from))
            .unwrap_or_else(|| home.join(".codexhome/router-policy.json"));
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<RoutePolicy> {
        let contents = fs::read_to_string(&self.path)
            .with_context(|| format!("failed to read route policy {}", self.path.display()))?;
        parse_route_policy(&contents)
    }
}

pub fn parse_route_request(input: &str) -> Result<RouteRequest> {
    let request: RouteRequest =
        serde_json::from_str(input).context("failed to parse route request JSON")?;
    request.validate()?;
    Ok(request)
}

pub fn parse_route_policy(input: &str) -> Result<RoutePolicy> {
    let policy: RoutePolicy =
        serde_json::from_str(input).context("failed to parse route policy JSON")?;
    policy.validate()?;
    Ok(policy)
}

#[derive(Debug)]
pub struct RouteEngine<'a> {
    policy: &'a RoutePolicy,
}

impl<'a> RouteEngine<'a> {
    pub fn new(policy: &'a RoutePolicy) -> Result<Self> {
        policy.validate()?;
        Ok(Self { policy })
    }

    pub fn evaluate(
        &self,
        request: &RouteRequest,
        events: &[ObservabilityEvent],
        homes: &[RouteHomeState],
        now_ms: u64,
    ) -> Result<RouteDecision> {
        request.validate()?;
        let rule = self.policy.rule(request.task_kind);
        let weights = rule
            .and_then(|rule| rule.weights.as_ref())
            .unwrap_or(&self.policy.weights);
        let required_capabilities = normalized_union(
            &request.required_capabilities,
            rule.map_or(&[], |rule| rule.required_capabilities.as_slice()),
        );
        let preferred_specialties = normalized_union(
            &request.preferred_specialties,
            rule.map_or(&[], |rule| rule.preferred_specialties.as_slice()),
        );

        let mut drafts = self
            .policy
            .candidates
            .iter()
            .map(|candidate| {
                self.evaluate_constraints(
                    candidate,
                    request,
                    rule,
                    &required_capabilities,
                    events,
                    homes,
                    now_ms,
                )
            })
            .collect::<Result<Vec<_>>>()?;

        let minimum_cost = drafts
            .iter()
            .filter(|draft| draft.rejection_reasons.is_empty())
            .map(|draft| draft.estimated_cost_microusd)
            .min();
        let minimum_duration = drafts
            .iter()
            .filter(|draft| draft.rejection_reasons.is_empty())
            .filter_map(|draft| draft.expected_duration_ms)
            .min();

        for draft in &mut drafts {
            if draft.rejection_reasons.is_empty() {
                score_candidate(
                    draft,
                    weights,
                    &preferred_specialties,
                    minimum_cost,
                    minimum_duration,
                );
            }
        }

        let mut candidates = drafts
            .into_iter()
            .map(CandidateDraft::finish)
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .total_score_basis_points
                .cmp(&left.total_score_basis_points)
                .then(left.candidate_id.cmp(&right.candidate_id))
        });
        let selected = candidates
            .iter()
            .find(|candidate| candidate.eligible)
            .map(|candidate| RouteSelection {
                candidate_id: candidate.candidate_id.clone(),
                identity: candidate.identity.clone(),
                total_score_basis_points: candidate.total_score_basis_points.unwrap_or(0),
                estimated_cost_microusd: candidate.estimated_cost_microusd,
                reasons: candidate.reasons.clone(),
            });
        let eligible_candidates = candidates
            .iter()
            .filter(|candidate| candidate.eligible)
            .count();
        let decision_reason = selected.as_ref().map_or_else(
            || {
                let rejected = candidates
                    .iter()
                    .flat_map(|candidate| candidate.rejection_reasons.iter())
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join("; ");
                format!("no eligible candidate: {rejected}")
            },
            |selection| {
                format!(
                    "selected '{}' with explainable score {}/10000 from {} eligible candidate(s)",
                    selection.candidate_id, selection.total_score_basis_points, eligible_candidates
                )
            },
        );
        Ok(RouteDecision {
            ok: selected.is_some(),
            schema_version: ROUTE_DECISION_SCHEMA_VERSION,
            request_id: request.request_id.clone(),
            task_kind: request.task_kind,
            policy_id: self.policy.policy_id.clone(),
            policy_revision: self.policy.revision,
            evaluated_at_timestamp_ms: now_ms,
            observed_event_count: events.len(),
            request: request.clone(),
            selected,
            evaluated_candidates: candidates.len(),
            eligible_candidates,
            candidates,
            decision_reason,
            locks: RouteLocks {
                home: request.locked_home.clone(),
                model: request.locked_model.clone(),
                sensitive_directory: request.sensitive_directory,
            },
            safety: SafetySummary {
                secrets_redacted: true,
                includes_secrets: false,
                reads_auth_contents: false,
                includes_local_paths: false,
            },
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn evaluate_constraints<'b>(
        &self,
        candidate: &'b RouteCandidateProfile,
        request: &RouteRequest,
        rule: Option<&RouteTaskRule>,
        required_capabilities: &BTreeSet<String>,
        events: &[ObservabilityEvent],
        homes: &[RouteHomeState],
        now_ms: u64,
    ) -> Result<CandidateDraft<'b>> {
        let runtime = CandidateRuntime::from_events(candidate, events);
        let estimated_cost_microusd = estimated_cost(candidate, request)?;
        let mut rejection_reasons = Vec::new();
        if !candidate.enabled {
            rejection_reasons.push("candidate is disabled".to_owned());
        }
        match home_state(candidate, homes) {
            Some(home) if home.available => {}
            Some(_) => rejection_reasons.push("Home is currently unavailable".to_owned()),
            None => rejection_reasons.push("Home is not present in the active registry".to_owned()),
        }
        if request
            .locked_home
            .as_deref()
            .is_some_and(|locked| !candidate.matches_home(locked))
        {
            rejection_reasons.push(format!(
                "user locked Home to '{}'",
                request.locked_home.as_deref().unwrap_or_default()
            ));
        }
        if request
            .locked_model
            .as_deref()
            .is_some_and(|locked| !candidate.model.eq_ignore_ascii_case(locked))
        {
            rejection_reasons.push(format!(
                "user locked model to '{}'",
                request.locked_model.as_deref().unwrap_or_default()
            ));
        }
        if let Some(domain) = &request.required_security_domain {
            if !contains_label(&candidate.security_domains, domain) {
                rejection_reasons.push(format!("missing security domain '{domain}'"));
            }
        }
        for capability in required_capabilities {
            if !contains_label(&candidate.capabilities, capability) {
                rejection_reasons.push(format!("missing required capability '{capability}'"));
            }
        }
        let total_context = request
            .estimated_context_tokens
            .checked_add(request.estimated_output_tokens)
            .context("estimated route tokens overflow")?;
        if total_context > candidate.max_context_tokens {
            rejection_reasons.push(format!(
                "requires {total_context} context tokens but candidate supports {}",
                candidate.max_context_tokens
            ));
        }
        if rule
            .and_then(|rule| rule.minimum_model_strength_basis_points)
            .is_some_and(|minimum| candidate.model_strength_basis_points < minimum)
        {
            rejection_reasons.push(format!(
                "model strength {} is below task minimum {}",
                candidate.model_strength_basis_points,
                rule.and_then(|rule| rule.minimum_model_strength_basis_points)
                    .unwrap_or_default()
            ));
        }
        if request
            .max_estimated_cost_microusd
            .is_some_and(|maximum| estimated_cost_microusd > maximum)
        {
            rejection_reasons.push(format!(
                "estimated cost {estimated_cost_microusd} exceeds request budget {}",
                request.max_estimated_cost_microusd.unwrap_or_default()
            ));
        }
        if runtime.active_attempts >= u64::from(candidate.max_concurrent_runs) {
            rejection_reasons.push(format!(
                "Home load {}/{} has reached its concurrency limit",
                runtime.active_attempts, candidate.max_concurrent_runs
            ));
        }
        if let Some(health) = &runtime.latest_health {
            if !health.snapshot.service_reachable {
                rejection_reasons.push("service is unreachable".to_owned());
            }
            if !request.allow_degraded
                && health.status == ObservabilityStatus::Unhealthy
                && health.snapshot.service_reachable
            {
                rejection_reasons.push("Home health is unhealthy".to_owned());
            }
        }
        if runtime
            .latest_auth
            .is_some_and(|(_, auth_valid)| !auth_valid)
        {
            rejection_reasons.push("authentication is invalid".to_owned());
        }
        if runtime
            .latest_quota
            .is_some_and(|(_, quota_basis_points)| quota_basis_points == 0)
        {
            rejection_reasons.push("account quota is exhausted".to_owned());
        }
        if runtime
            .latest_rate_limit
            .is_some_and(|(_, reset)| reset.is_none_or(|reset| reset > now_ms))
        {
            rejection_reasons.push("account is actively rate limited".to_owned());
        }

        let historical_success_basis_points = smoothed_success(
            runtime.successes,
            runtime.terminal_attempts,
            self.policy.history_prior_success_basis_points,
            self.policy.history_prior_attempts,
        );
        Ok(CandidateDraft {
            candidate,
            estimated_cost_microusd,
            historical_attempts: runtime.terminal_attempts,
            historical_success_basis_points,
            active_attempts: runtime.active_attempts,
            quota_remaining_basis_points: runtime
                .latest_quota
                .map(|(_, quota_basis_points)| quota_basis_points),
            quota_score_basis_points: runtime
                .latest_quota
                .map(|(_, quota_basis_points)| quota_basis_points)
                .unwrap_or(self.policy.unknown_quota_basis_points),
            health_score_basis_points: health_score(
                runtime.latest_health.as_ref(),
                self.policy.unknown_health_basis_points,
            ),
            expected_duration_ms: runtime
                .average_duration_ms()
                .or(candidate.baseline_duration_ms),
            context_score_basis_points: context_headroom(candidate, total_context),
            rejection_reasons,
            components: Vec::new(),
            reasons: Vec::new(),
            total_score_basis_points: None,
        })
    }
}

#[derive(Debug)]
struct CandidateDraft<'a> {
    candidate: &'a RouteCandidateProfile,
    estimated_cost_microusd: u64,
    historical_attempts: u64,
    historical_success_basis_points: u16,
    active_attempts: u64,
    quota_remaining_basis_points: Option<u16>,
    quota_score_basis_points: u16,
    health_score_basis_points: u16,
    expected_duration_ms: Option<u64>,
    context_score_basis_points: u16,
    rejection_reasons: Vec<String>,
    components: Vec<RouteScoreComponent>,
    reasons: Vec<String>,
    total_score_basis_points: Option<u16>,
}

impl CandidateDraft<'_> {
    fn finish(self) -> RouteCandidateEvaluation {
        RouteCandidateEvaluation {
            candidate_id: self.candidate.candidate_id.clone(),
            identity: self.candidate.identity(),
            eligible: self.rejection_reasons.is_empty(),
            total_score_basis_points: self.total_score_basis_points,
            estimated_cost_microusd: self.estimated_cost_microusd,
            historical_attempts: self.historical_attempts,
            historical_success_basis_points: self.historical_success_basis_points,
            active_attempts: self.active_attempts,
            quota_remaining_basis_points: self.quota_remaining_basis_points,
            components: self.components,
            reasons: self.reasons,
            rejection_reasons: self.rejection_reasons,
        }
    }
}

fn score_candidate(
    draft: &mut CandidateDraft<'_>,
    weights: &RouteScoreWeights,
    preferred_specialties: &BTreeSet<String>,
    minimum_cost: Option<u64>,
    minimum_duration: Option<u64>,
) {
    let task_fit = ratio_matches(&draft.candidate.specialties, preferred_specialties);
    let speed = relative_inverse_score(draft.expected_duration_ms, minimum_duration);
    let cost = relative_inverse_score(Some(draft.estimated_cost_microusd), minimum_cost);
    let load = load_score(draft.active_attempts, draft.candidate.max_concurrent_runs);
    let scores = [
        (
            "task_fit",
            task_fit,
            weights.task_fit,
            format!("matched task specialties at {task_fit}/10000"),
        ),
        (
            "model_strength",
            draft.candidate.model_strength_basis_points,
            weights.model_strength,
            format!(
                "declared model strength is {}/10000",
                draft.candidate.model_strength_basis_points
            ),
        ),
        (
            "context_headroom",
            draft.context_score_basis_points,
            weights.context_headroom,
            format!(
                "context headroom is {}/10000",
                draft.context_score_basis_points
            ),
        ),
        (
            "quota",
            draft.quota_score_basis_points,
            weights.quota,
            format!(
                "latest quota score is {}/10000",
                draft.quota_score_basis_points
            ),
        ),
        (
            "historical_success",
            draft.historical_success_basis_points,
            weights.historical_success,
            format!(
                "smoothed success is {}/10000 from {} terminal attempt(s)",
                draft.historical_success_basis_points, draft.historical_attempts
            ),
        ),
        (
            "speed",
            speed,
            weights.speed,
            match draft.expected_duration_ms {
                Some(duration) => {
                    format!("expected duration is {duration}ms; relative speed score {speed}/10000")
                }
                None => "no duration evidence; neutral speed score 5000/10000".to_owned(),
            },
        ),
        (
            "cost",
            cost,
            weights.cost,
            format!(
                "estimated cost is {} microUSD; relative cost score {cost}/10000",
                draft.estimated_cost_microusd
            ),
        ),
        (
            "load",
            load,
            weights.load,
            format!(
                "active load is {}/{}; load score {load}/10000",
                draft.active_attempts, draft.candidate.max_concurrent_runs
            ),
        ),
        (
            "health",
            draft.health_score_basis_points,
            weights.health,
            format!(
                "latest health score is {}/10000",
                draft.health_score_basis_points
            ),
        ),
    ];
    let mut total = 0_u64;
    for (name, score, weight, reason) in scores {
        let weighted = u64::from(score) * u64::from(weight) / 10_000;
        total = total.saturating_add(weighted);
        draft.components.push(RouteScoreComponent {
            name: name.to_owned(),
            score_basis_points: score,
            weight_basis_points: weight,
            weighted_points: u16::try_from(weighted).unwrap_or(u16::MAX),
            reason: reason.clone(),
        });
        draft.reasons.push(reason);
    }
    draft.total_score_basis_points = Some(u16::try_from(total).unwrap_or(10_000).min(10_000));
}

#[derive(Debug, Default)]
struct CandidateRuntime {
    starts: BTreeSet<String>,
    terminals: BTreeSet<String>,
    terminal_attempts: u64,
    successes: u64,
    total_duration_ms: u64,
    active_attempts: u64,
    latest_health: Option<RuntimeHealth>,
    latest_quota: Option<(u64, u16)>,
    latest_auth: Option<(u64, bool)>,
    latest_rate_limit: Option<(u64, Option<u64>)>,
}

#[derive(Debug)]
struct RuntimeHealth {
    timestamp_ms: u64,
    status: ObservabilityStatus,
    snapshot: HealthSnapshot,
}

impl CandidateRuntime {
    fn from_events(candidate: &RouteCandidateProfile, events: &[ObservabilityEvent]) -> Self {
        let mut runtime = Self::default();
        for event in events {
            if event.event_type == ObservabilityEventType::AttemptStarted
                && identity_matches_candidate(&event.identity, candidate)
            {
                if let Some(attempt_id) = &event.trace.attempt_id {
                    runtime.starts.insert(attempt_id.clone());
                }
            }
            if matches!(
                event.event_type,
                ObservabilityEventType::AttemptCompleted | ObservabilityEventType::AttemptFailed
            ) {
                if let Some(attempt_id) = &event.trace.attempt_id {
                    if runtime.starts.contains(attempt_id)
                        && runtime.terminals.insert(attempt_id.clone())
                    {
                        runtime.terminal_attempts = runtime.terminal_attempts.saturating_add(1);
                        if event.event_type == ObservabilityEventType::AttemptCompleted
                            && event.status == ObservabilityStatus::Succeeded
                        {
                            runtime.successes = runtime.successes.saturating_add(1);
                        }
                        runtime.total_duration_ms = runtime
                            .total_duration_ms
                            .saturating_add(event.usage.duration_ms);
                    }
                }
            }
            if matches!(
                event.event_type,
                ObservabilityEventType::QuotaSnapshot
                    | ObservabilityEventType::RateLimited
                    | ObservabilityEventType::AuthInvalid
                    | ObservabilityEventType::HomeHealth
            ) && health_identity_matches_candidate(&event.identity, candidate)
            {
                if let Some(snapshot) = &event.health {
                    let replace = runtime
                        .latest_health
                        .as_ref()
                        .is_none_or(|health| event.timestamp_ms >= health.timestamp_ms);
                    if replace {
                        runtime.latest_health = Some(RuntimeHealth {
                            timestamp_ms: event.timestamp_ms,
                            status: event.status,
                            snapshot: snapshot.clone(),
                        });
                    }
                    if let Some(quota) = snapshot.quota_remaining_basis_points {
                        if runtime
                            .latest_quota
                            .is_none_or(|(timestamp, _)| event.timestamp_ms >= timestamp)
                        {
                            runtime.latest_quota = Some((event.timestamp_ms, quota));
                        }
                    }
                    let auth_valid = if event.event_type == ObservabilityEventType::AuthInvalid {
                        Some(false)
                    } else {
                        snapshot.auth_valid
                    };
                    if let Some(auth_valid) = auth_valid {
                        if runtime
                            .latest_auth
                            .is_none_or(|(timestamp, _)| event.timestamp_ms >= timestamp)
                        {
                            runtime.latest_auth = Some((event.timestamp_ms, auth_valid));
                        }
                    }
                    if event.event_type == ObservabilityEventType::RateLimited
                        && runtime
                            .latest_rate_limit
                            .is_none_or(|(timestamp, _)| event.timestamp_ms >= timestamp)
                    {
                        runtime.latest_rate_limit =
                            Some((event.timestamp_ms, snapshot.rate_limit_reset_at_ms));
                    }
                }
            }
        }
        runtime.active_attempts =
            u64::try_from(runtime.starts.difference(&runtime.terminals).count())
                .unwrap_or(u64::MAX);
        runtime
    }

    fn average_duration_ms(&self) -> Option<u64> {
        (self.terminal_attempts > 0).then(|| self.total_duration_ms / self.terminal_attempts)
    }
}

fn identity_matches_candidate(
    identity: &ExecutionIdentity,
    candidate: &RouteCandidateProfile,
) -> bool {
    identity
        .home_id
        .as_deref()
        .is_some_and(|home| candidate.matches_home(home))
        && identity.account_id == candidate.account_id
        && identity
            .model
            .as_deref()
            .is_some_and(|model| model.eq_ignore_ascii_case(&candidate.model))
}

fn health_identity_matches_candidate(
    identity: &ExecutionIdentity,
    candidate: &RouteCandidateProfile,
) -> bool {
    identity
        .home_id
        .as_deref()
        .is_some_and(|home| candidate.matches_home(home))
        && (identity.account_id.is_none() || identity.account_id == candidate.account_id)
        && identity
            .model
            .as_deref()
            .is_none_or(|model| model.eq_ignore_ascii_case(&candidate.model))
}

fn home_state<'a>(
    candidate: &RouteCandidateProfile,
    homes: &'a [RouteHomeState],
) -> Option<&'a RouteHomeState> {
    homes.iter().find(|home| {
        home.home_id.eq_ignore_ascii_case(&candidate.home_id)
            || candidate
                .home_alias
                .as_deref()
                .zip(home.home_alias.as_deref())
                .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
    })
}

fn estimated_cost(candidate: &RouteCandidateProfile, request: &RouteRequest) -> Result<u64> {
    let input = u128::from(request.estimated_context_tokens)
        .checked_mul(u128::from(candidate.input_cost_microusd_per_million_tokens))
        .context("input cost overflow")?;
    let output = u128::from(request.estimated_output_tokens)
        .checked_mul(u128::from(
            candidate.output_cost_microusd_per_million_tokens,
        ))
        .context("output cost overflow")?;
    let total = input.checked_add(output).context("route cost overflow")?;
    let rounded = total.saturating_add(999_999) / 1_000_000;
    u64::try_from(rounded).context("estimated route cost does not fit in u64")
}

fn context_headroom(candidate: &RouteCandidateProfile, required: u64) -> u16 {
    if required >= candidate.max_context_tokens {
        return 0;
    }
    ratio_basis_points(
        candidate.max_context_tokens - required,
        candidate.max_context_tokens,
    )
}

fn smoothed_success(successes: u64, attempts: u64, prior: u16, prior_weight: u16) -> u16 {
    let numerator = u128::from(successes)
        .saturating_mul(10_000)
        .saturating_add(u128::from(prior).saturating_mul(u128::from(prior_weight)));
    let denominator = u128::from(attempts).saturating_add(u128::from(prior_weight));
    let smoothed = numerator
        .checked_div(denominator)
        .unwrap_or(u128::from(prior));
    u16::try_from(smoothed.min(10_000)).unwrap_or(10_000)
}

fn health_score(health: Option<&RuntimeHealth>, unknown: u16) -> u16 {
    let Some(health) = health else {
        return unknown;
    };
    if !health.snapshot.service_reachable || health.snapshot.auth_valid == Some(false) {
        return 0;
    }
    match health.status {
        ObservabilityStatus::Healthy | ObservabilityStatus::Succeeded => 10_000,
        ObservabilityStatus::Degraded => 5_000,
        ObservabilityStatus::Unhealthy | ObservabilityStatus::Failed => 2_000,
        _ => 7_500,
    }
}

fn load_score(active: u64, capacity: u16) -> u16 {
    10_000_u16.saturating_sub(ratio_basis_points(active, u64::from(capacity)))
}

fn ratio_matches(values: &[String], expected: &BTreeSet<String>) -> u16 {
    if expected.is_empty() {
        return 5_000;
    }
    let actual = values
        .iter()
        .map(|value| normalize_label(value))
        .collect::<BTreeSet<_>>();
    let matches = expected.intersection(&actual).count();
    ratio_basis_points(
        u64::try_from(matches).unwrap_or(u64::MAX),
        u64::try_from(expected.len()).unwrap_or(u64::MAX),
    )
}

fn relative_inverse_score(value: Option<u64>, minimum: Option<u64>) -> u16 {
    match (value, minimum) {
        (None, _) => 5_000,
        (Some(0), _) => 10_000,
        (Some(_), Some(0)) => 0,
        (Some(value), Some(minimum)) => ratio_basis_points(minimum, value),
        (Some(_), None) => 5_000,
    }
}

fn ratio_basis_points(numerator: u64, denominator: u64) -> u16 {
    if denominator == 0 {
        return 0;
    }
    let value = u128::from(numerator).saturating_mul(10_000) / u128::from(denominator);
    u16::try_from(value.min(10_000)).unwrap_or(10_000)
}

fn normalized_union(left: &[String], right: &[String]) -> BTreeSet<String> {
    left.iter()
        .chain(right)
        .map(|value| normalize_label(value))
        .collect()
}

fn contains_label(values: &[String], expected: &str) -> bool {
    let expected = normalize_label(expected);
    values
        .iter()
        .any(|value| normalize_label(value) == expected)
}

fn normalize_label(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn validate_basis_points(name: &str, value: u16) -> Result<()> {
    if value > 10_000 {
        bail!("{name} must be between 0 and 10000");
    }
    Ok(())
}

fn validate_identifier(name: &str, value: &str) -> Result<()> {
    let valid = !value.is_empty()
        && value.len() <= 160
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '.' | '_' | ':' | '/' | '@' | '-')
        });
    if !valid {
        bail!("{name} contains unsupported characters or length");
    }
    Ok(())
}

fn validate_label(name: &str, value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        bail!("{name} must be a non-empty label no longer than 256 bytes");
    }
    Ok(())
}

fn validate_optional_label(name: &str, value: &Option<String>) -> Result<()> {
    if let Some(value) = value {
        validate_label(name, value)?;
    }
    Ok(())
}

fn validate_labels(name: &str, values: &[String]) -> Result<()> {
    if values.len() > 64 {
        bail!("{name} cannot contain more than 64 labels");
    }
    let mut unique = BTreeSet::new();
    for value in values {
        validate_label(name, value)?;
        if !unique.insert(normalize_label(value)) {
            bail!("{name} contains duplicate label '{value}'");
        }
    }
    Ok(())
}

fn default_true() -> bool {
    true
}

fn default_max_concurrent_runs() -> u16 {
    1
}

fn default_history_prior() -> u16 {
    5_000
}

fn default_history_prior_weight() -> u16 {
    4
}

fn default_unknown_quota() -> u16 {
    5_000
}

fn default_unknown_health() -> u16 {
    7_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EventDetails, EventSafety, TraceContext, UsageDelta, OBSERVABILITY_EVENT_SCHEMA_VERSION,
    };

    fn candidate(
        id: &str,
        home: &str,
        model: &str,
        specialties: &[&str],
        strength: u16,
        context: u64,
        input_cost: u64,
    ) -> RouteCandidateProfile {
        RouteCandidateProfile {
            candidate_id: id.to_owned(),
            enabled: true,
            home_id: home.to_owned(),
            home_alias: Some(format!("@{home}")),
            account_id: Some(format!("account-{home}")),
            provider: Some("test".to_owned()),
            model: model.to_owned(),
            specialties: specialties
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            capabilities: vec!["coding".to_owned(), "long-context".to_owned()],
            security_domains: vec!["local-sensitive".to_owned()],
            max_context_tokens: context,
            model_strength_basis_points: strength,
            input_cost_microusd_per_million_tokens: input_cost,
            output_cost_microusd_per_million_tokens: input_cost * 2,
            max_concurrent_runs: 2,
            baseline_duration_ms: Some(input_cost.max(1)),
        }
    }

    fn policy() -> RoutePolicy {
        RoutePolicy {
            schema_version: ROUTE_POLICY_SCHEMA_VERSION.to_owned(),
            policy_id: "test-policy".to_owned(),
            revision: 1,
            weights: RouteScoreWeights::default(),
            task_rules: vec![
                RouteTaskRule {
                    task_kind: RouteTaskKind::SimpleCode,
                    preferred_specialties: vec!["spark".to_owned()],
                    required_capabilities: vec!["coding".to_owned()],
                    minimum_model_strength_basis_points: None,
                    weights: None,
                },
                RouteTaskRule {
                    task_kind: RouteTaskKind::Architecture,
                    preferred_specialties: vec!["architecture".to_owned()],
                    required_capabilities: vec!["coding".to_owned()],
                    minimum_model_strength_basis_points: Some(8_000),
                    weights: None,
                },
            ],
            candidates: vec![
                candidate(
                    "spark",
                    "spark",
                    "spark-model",
                    &["spark", "coding"],
                    6_000,
                    128_000,
                    100,
                ),
                candidate(
                    "primary",
                    "primary",
                    "strong-model",
                    &["architecture", "coding"],
                    9_500,
                    512_000,
                    900,
                ),
            ],
            history_prior_success_basis_points: 5_000,
            history_prior_attempts: 4,
            unknown_quota_basis_points: 5_000,
            unknown_health_basis_points: 7_000,
        }
    }

    fn homes() -> Vec<RouteHomeState> {
        vec![
            RouteHomeState {
                home_id: "spark".to_owned(),
                home_alias: Some("@spark".to_owned()),
                available: true,
            },
            RouteHomeState {
                home_id: "primary".to_owned(),
                home_alias: Some("@primary".to_owned()),
                available: true,
            },
        ]
    }

    fn request(kind: RouteTaskKind) -> RouteRequest {
        RouteRequest {
            schema_version: ROUTE_REQUEST_SCHEMA_VERSION.to_owned(),
            request_id: "request-1".to_owned(),
            task_kind: kind,
            estimated_context_tokens: 20_000,
            estimated_output_tokens: 2_000,
            required_capabilities: Vec::new(),
            preferred_specialties: Vec::new(),
            sensitive_directory: false,
            required_security_domain: None,
            locked_home: None,
            locked_model: None,
            max_estimated_cost_microusd: None,
            allow_degraded: false,
        }
    }

    #[test]
    fn simple_code_prefers_spark_and_architecture_requires_strong_model() {
        let policy = policy();
        let engine = RouteEngine::new(&policy).expect("engine");
        let simple = engine
            .evaluate(&request(RouteTaskKind::SimpleCode), &[], &homes(), 1)
            .expect("simple route");
        assert_eq!(
            simple
                .selected
                .as_ref()
                .map(|value| value.candidate_id.as_str()),
            Some("spark")
        );

        let architecture = engine
            .evaluate(&request(RouteTaskKind::Architecture), &[], &homes(), 1)
            .expect("architecture route");
        assert_eq!(
            architecture
                .selected
                .as_ref()
                .map(|value| value.candidate_id.as_str()),
            Some("primary")
        );
        assert!(architecture.candidates.iter().any(|candidate| {
            candidate.candidate_id == "spark"
                && candidate
                    .rejection_reasons
                    .iter()
                    .any(|reason| reason.contains("model strength"))
        }));
    }

    #[test]
    fn locks_and_sensitive_directory_are_hard_constraints() {
        let policy = policy();
        let engine = RouteEngine::new(&policy).expect("engine");
        let mut request = request(RouteTaskKind::SimpleCode);
        request.sensitive_directory = true;
        request.locked_home = Some("@primary".to_owned());
        request.required_security_domain = Some("local-sensitive".to_owned());
        let decision = engine
            .evaluate(&request, &[], &homes(), 1)
            .expect("locked route");
        assert_eq!(
            decision
                .selected
                .as_ref()
                .map(|value| value.candidate_id.as_str()),
            Some("primary")
        );
        assert!(decision.candidates.iter().any(|candidate| {
            candidate.candidate_id == "spark"
                && candidate
                    .rejection_reasons
                    .iter()
                    .any(|reason| reason.contains("locked Home"))
        }));
    }

    #[test]
    fn exhausted_quota_rejects_candidate_and_history_changes_score() {
        let policy = policy();
        let engine = RouteEngine::new(&policy).expect("engine");
        let mut later_home_health = health_event(&policy.candidates[0], 4, None);
        later_home_health.event_type = ObservabilityEventType::HomeHealth;
        let events = vec![
            attempt_event(
                "start-primary",
                ObservabilityEventType::AttemptStarted,
                ObservabilityStatus::Running,
                "attempt-primary",
                &policy.candidates[1],
                1,
            ),
            attempt_event(
                "end-primary",
                ObservabilityEventType::AttemptCompleted,
                ObservabilityStatus::Succeeded,
                "attempt-primary",
                &policy.candidates[1],
                2,
            ),
            health_event(&policy.candidates[0], 3, Some(0)),
            later_home_health,
        ];
        let decision = engine
            .evaluate(&request(RouteTaskKind::SimpleCode), &events, &homes(), 5)
            .expect("route");
        assert_eq!(
            decision
                .selected
                .as_ref()
                .map(|value| value.candidate_id.as_str()),
            Some("primary")
        );
        let spark = decision
            .candidates
            .iter()
            .find(|candidate| candidate.candidate_id == "spark")
            .expect("spark");
        assert!(spark
            .rejection_reasons
            .iter()
            .any(|reason| reason.contains("quota")));
    }

    #[test]
    fn long_document_and_batch_choose_context_and_cost_specialists() {
        let mut policy = policy();
        policy.task_rules.extend([
            RouteTaskRule {
                task_kind: RouteTaskKind::LongDocument,
                preferred_specialties: vec!["long-context".to_owned()],
                required_capabilities: vec!["long-context".to_owned()],
                minimum_model_strength_basis_points: None,
                weights: None,
            },
            RouteTaskRule {
                task_kind: RouteTaskKind::Batch,
                preferred_specialties: vec!["low-cost".to_owned()],
                required_capabilities: Vec::new(),
                minimum_model_strength_basis_points: None,
                weights: Some(RouteScoreWeights {
                    task_fit: 1_500,
                    model_strength: 500,
                    context_headroom: 500,
                    quota: 1_000,
                    historical_success: 1_000,
                    speed: 1_000,
                    cost: 3_500,
                    load: 700,
                    health: 300,
                }),
            },
        ]);
        policy.candidates[0].specialties.push("low-cost".to_owned());
        policy.candidates.push(candidate(
            "long",
            "long",
            "long-model",
            &["long-context", "documents"],
            8_200,
            1_000_000,
            600,
        ));
        let mut homes = homes();
        homes.push(RouteHomeState {
            home_id: "long".to_owned(),
            home_alias: Some("@long".to_owned()),
            available: true,
        });
        let engine = RouteEngine::new(&policy).expect("engine");

        let mut long_request = request(RouteTaskKind::LongDocument);
        long_request.estimated_context_tokens = 700_000;
        let long_decision = engine
            .evaluate(&long_request, &[], &homes, 1)
            .expect("long route");
        assert_eq!(
            long_decision
                .selected
                .as_ref()
                .map(|value| value.candidate_id.as_str()),
            Some("long")
        );

        let batch_decision = engine
            .evaluate(&request(RouteTaskKind::Batch), &[], &homes, 1)
            .expect("batch route");
        assert_eq!(
            batch_decision
                .selected
                .as_ref()
                .map(|value| value.candidate_id.as_str()),
            Some("spark")
        );
    }

    #[test]
    fn active_load_is_a_hard_capacity_constraint() {
        let mut policy = policy();
        policy.candidates[0].max_concurrent_runs = 1;
        let events = vec![attempt_event(
            "start-spark",
            ObservabilityEventType::AttemptStarted,
            ObservabilityStatus::Running,
            "attempt-spark",
            &policy.candidates[0],
            1,
        )];
        let engine = RouteEngine::new(&policy).expect("engine");
        let decision = engine
            .evaluate(&request(RouteTaskKind::SimpleCode), &events, &homes(), 2)
            .expect("route");
        let spark = decision
            .candidates
            .iter()
            .find(|candidate| candidate.candidate_id == "spark")
            .expect("spark");
        assert!(spark
            .rejection_reasons
            .iter()
            .any(|reason| reason.contains("concurrency limit")));
        assert_ne!(
            decision
                .selected
                .as_ref()
                .map(|value| value.candidate_id.as_str()),
            Some("spark")
        );
    }

    fn attempt_event(
        event_id: &str,
        event_type: ObservabilityEventType,
        status: ObservabilityStatus,
        attempt_id: &str,
        candidate: &RouteCandidateProfile,
        timestamp_ms: u64,
    ) -> ObservabilityEvent {
        ObservabilityEvent {
            schema_version: OBSERVABILITY_EVENT_SCHEMA_VERSION.to_owned(),
            event_id: event_id.to_owned(),
            timestamp_ms,
            event_type,
            status,
            trace: TraceContext {
                attempt_id: Some(attempt_id.to_owned()),
                ..TraceContext::default()
            },
            identity: candidate.identity(),
            usage: UsageDelta {
                duration_ms: 100,
                ..UsageDelta::default()
            },
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

    fn health_event(
        candidate: &RouteCandidateProfile,
        timestamp_ms: u64,
        quota: Option<u16>,
    ) -> ObservabilityEvent {
        ObservabilityEvent {
            schema_version: OBSERVABILITY_EVENT_SCHEMA_VERSION.to_owned(),
            event_id: format!("health-{timestamp_ms}"),
            timestamp_ms,
            event_type: ObservabilityEventType::QuotaSnapshot,
            status: ObservabilityStatus::Healthy,
            trace: crate::TraceContext::default(),
            identity: candidate.identity(),
            usage: UsageDelta::default(),
            parent_event_id: None,
            tool_name: None,
            artifact_kind: None,
            verification_kind: None,
            failure: None,
            health: Some(HealthSnapshot {
                service_reachable: true,
                auth_valid: Some(true),
                quota_remaining_basis_points: quota,
                rate_limit_reset_at_ms: None,
                detail_code: None,
            }),
            details: EventDetails::default(),
            safety: EventSafety::default(),
        }
    }
}
