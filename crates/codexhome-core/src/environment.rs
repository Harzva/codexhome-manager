use crate::{RegistryEntry, SafetySummary};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const ACCOUNT_PROFILE_SCHEMA_VERSION: &str = "codexhome.account-profile.v1";
pub const LEGACY_HOME_SCHEMA_VERSION: &str = "codexhome.legacy-home.v1";
pub const EXPERT_PACK_SCHEMA_VERSION: &str = "codexhome.expert-pack.v1";
pub const PROJECT_BINDING_SCHEMA_VERSION: &str = "codexhome.project-binding.v1";
pub const SKILL_REGISTRY_SCHEMA_VERSION: &str = "codexhome.skill-registry.v1";
pub const EFFECTIVE_RUNTIME_SCHEMA_VERSION: &str = "codexhome.effective-runtime.v1";
pub const RUNTIME_CONTEXT_SNAPSHOT_SCHEMA_VERSION: &str = "codexhome.runtime-context-snapshot.v1";
pub const RUNTIME_CONTEXT_REPORT_SCHEMA_VERSION: &str = "codexhome.runtime-context-report.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountProfile {
    pub schema_version: String,
    pub id: String,
    pub label: String,
    pub home_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_host: Option<String>,
    #[serde(default)]
    pub has_auth: bool,
    #[serde(default)]
    pub has_config: bool,
}

impl AccountProfile {
    pub fn validate(&self) -> Result<()> {
        validate_schema(
            "AccountProfile",
            &self.schema_version,
            ACCOUNT_PROFILE_SCHEMA_VERSION,
        )?;
        validate_id("accountProfile.id", &self.id)?;
        validate_label("accountProfile.label", &self.label)?;
        validate_absolute_path("accountProfile.homePath", &self.home_path)?;
        validate_optional_label("accountProfile.provider", self.provider.as_deref())?;
        validate_optional_label("accountProfile.model", self.model.as_deref())?;
        if let Some(host) = self.endpoint_host.as_deref() {
            validate_endpoint_host(host)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyHome {
    pub schema_version: String,
    pub entry: RegistryEntry,
    #[serde(default)]
    pub account_profile_id: Option<String>,
    #[serde(default)]
    pub expert_pack_ids: Vec<String>,
}

impl LegacyHome {
    pub fn from_registry_entry(entry: RegistryEntry) -> Self {
        Self {
            schema_version: LEGACY_HOME_SCHEMA_VERSION.to_owned(),
            account_profile_id: Some(format!("account-{}", entry.id)),
            expert_pack_ids: Vec::new(),
            entry,
        }
    }

    pub fn validate(&self) -> Result<()> {
        validate_schema(
            "LegacyHome",
            &self.schema_version,
            LEGACY_HOME_SCHEMA_VERSION,
        )?;
        if let Some(id) = self.account_profile_id.as_deref() {
            validate_id("legacyHome.accountProfileId", id)?;
        }
        validate_unique_ids("legacyHome.expertPackIds", &self.expert_pack_ids)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillRequirement {
    pub skill_id: String,
    #[serde(default = "default_true")]
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpertPack {
    pub schema_version: String,
    pub id: String,
    pub label: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub skills: Vec<SkillRequirement>,
    #[serde(default)]
    pub agents_files: Vec<String>,
    #[serde(default)]
    pub rule_files: Vec<String>,
    #[serde(default)]
    pub mcp_server_ids: Vec<String>,
    #[serde(default)]
    pub requires_hard_isolation: bool,
    #[serde(default)]
    pub context: ContextFootprint,
}

impl ExpertPack {
    pub fn validate(&self) -> Result<()> {
        validate_schema(
            "ExpertPack",
            &self.schema_version,
            EXPERT_PACK_SCHEMA_VERSION,
        )?;
        validate_id("expertPack.id", &self.id)?;
        validate_label("expertPack.label", &self.label)?;
        validate_version(&self.version)?;
        validate_unique_labels("expertPack.capabilities", &self.capabilities)?;
        validate_skill_requirements(&self.skills)?;
        validate_unique_paths("expertPack.agentsFiles", &self.agents_files)?;
        validate_unique_paths("expertPack.ruleFiles", &self.rule_files)?;
        reject_sensitive_paths("expertPack.agentsFiles", &self.agents_files)?;
        reject_sensitive_paths("expertPack.ruleFiles", &self.rule_files)?;
        validate_unique_ids("expertPack.mcpServerIds", &self.mcp_server_ids)?;
        self.context.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectHomeLayer {
    #[serde(default)]
    pub skills: Vec<SkillRequirement>,
    #[serde(default)]
    pub agents_files: Vec<String>,
    #[serde(default)]
    pub rule_files: Vec<String>,
    #[serde(default)]
    pub mcp_server_ids: Vec<String>,
    #[serde(default)]
    pub context: ContextFootprint,
}

impl ProjectHomeLayer {
    pub fn validate(&self, project_root: &Path) -> Result<()> {
        let project_root_text = project_root
            .to_str()
            .context("projectRoot must be valid UTF-8")?;
        validate_absolute_path("projectRoot", project_root_text)?;
        validate_skill_requirements(&self.skills)?;
        validate_project_paths("projectHome.agentsFiles", &self.agents_files, project_root)?;
        validate_project_paths("projectHome.ruleFiles", &self.rule_files, project_root)?;
        validate_unique_ids("projectHome.mcpServerIds", &self.mcp_server_ids)?;
        self.context.validate()
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
            && self.agents_files.is_empty()
            && self.rule_files.is_empty()
            && self.mcp_server_ids.is_empty()
            && self.context.total_tokens() == 0
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeIsolation {
    #[default]
    SharedAccountHome,
    Hard,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBinding {
    pub schema_version: String,
    pub id: String,
    pub project_id: String,
    pub project_root: String,
    pub account_profile_id: String,
    #[serde(default)]
    pub expert_pack_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_home: Option<ProjectHomeLayer>,
    #[serde(default)]
    pub isolation: RuntimeIsolation,
}

impl ProjectBinding {
    pub fn validate(&self) -> Result<()> {
        validate_schema(
            "ProjectBinding",
            &self.schema_version,
            PROJECT_BINDING_SCHEMA_VERSION,
        )?;
        validate_id("projectBinding.id", &self.id)?;
        validate_id("projectBinding.projectId", &self.project_id)?;
        validate_id("projectBinding.accountProfileId", &self.account_profile_id)?;
        validate_absolute_path("projectBinding.projectRoot", &self.project_root)?;
        validate_unique_ids("projectBinding.expertPackIds", &self.expert_pack_ids)?;
        if let Some(project_home) = &self.project_home {
            project_home.validate(Path::new(&self.project_root))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextFootprint {
    #[serde(default)]
    pub skill_catalog_tokens: u64,
    #[serde(default)]
    pub skill_body_tokens: u64,
    #[serde(default)]
    pub agents_tokens: u64,
    #[serde(default)]
    pub rules_tokens: u64,
    #[serde(default)]
    pub tool_schema_tokens: u64,
}

impl ContextFootprint {
    pub fn total_tokens(&self) -> u64 {
        self.skill_catalog_tokens
            .saturating_add(self.skill_body_tokens)
            .saturating_add(self.agents_tokens)
            .saturating_add(self.rules_tokens)
            .saturating_add(self.tool_schema_tokens)
    }

    pub fn add_assign(&mut self, other: &Self) {
        self.skill_catalog_tokens = self
            .skill_catalog_tokens
            .saturating_add(other.skill_catalog_tokens);
        self.skill_body_tokens = self
            .skill_body_tokens
            .saturating_add(other.skill_body_tokens);
        self.agents_tokens = self.agents_tokens.saturating_add(other.agents_tokens);
        self.rules_tokens = self.rules_tokens.saturating_add(other.rules_tokens);
        self.tool_schema_tokens = self
            .tool_schema_tokens
            .saturating_add(other.tool_schema_tokens);
    }

    pub fn validate(&self) -> Result<()> {
        if self.total_tokens() > 50_000_000 {
            bail!("context footprint exceeds the 50M-token safety limit");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeContextPressure {
    Healthy,
    Notice,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeContextSnapshot {
    pub schema_version: String,
    pub run_id: String,
    pub attempt_id: String,
    pub context_window_tokens: u64,
    pub skill_catalog_tokens: u64,
    pub loaded_skill_body_tokens: u64,
    pub agents_tokens: u64,
    pub rules_tokens: u64,
    pub tool_schema_tokens: u64,
    pub conversation_history_tokens: u64,
    pub cached_reuse_tokens: u64,
    #[serde(default)]
    pub loaded_skill_ids: Vec<String>,
}

impl RuntimeContextSnapshot {
    pub fn active_context_tokens(&self) -> u64 {
        self.skill_catalog_tokens
            .saturating_add(self.loaded_skill_body_tokens)
            .saturating_add(self.agents_tokens)
            .saturating_add(self.rules_tokens)
            .saturating_add(self.tool_schema_tokens)
            .saturating_add(self.conversation_history_tokens)
    }

    pub fn usage_basis_points(&self) -> u16 {
        if self.context_window_tokens == 0 {
            return 0;
        }
        let points = self
            .active_context_tokens()
            .saturating_mul(10_000)
            .checked_div(self.context_window_tokens)
            .unwrap_or(u64::MAX)
            .min(10_000);
        points as u16
    }

    pub fn pressure(&self) -> RuntimeContextPressure {
        match self.usage_basis_points() {
            0..=99 => RuntimeContextPressure::Healthy,
            100..=200 => RuntimeContextPressure::Notice,
            201..=500 => RuntimeContextPressure::Warning,
            _ => RuntimeContextPressure::Critical,
        }
    }

    pub fn validate(&self) -> Result<()> {
        validate_schema(
            "RuntimeContextSnapshot",
            &self.schema_version,
            RUNTIME_CONTEXT_SNAPSHOT_SCHEMA_VERSION,
        )?;
        validate_id("runtimeContext.runId", &self.run_id)?;
        validate_id("runtimeContext.attemptId", &self.attempt_id)?;
        if self.context_window_tokens == 0 {
            bail!("runtimeContext.contextWindowTokens must be greater than zero");
        }
        if self.active_context_tokens() > self.context_window_tokens {
            bail!("runtime context exceeds its declared context window");
        }
        if self.cached_reuse_tokens > self.active_context_tokens() {
            bail!("runtimeContext.cachedReuseTokens exceeds active context");
        }
        validate_unique_ids("runtimeContext.loadedSkillIds", &self.loaded_skill_ids)
    }

    pub fn report(&self) -> Result<RuntimeContextReport> {
        self.validate()?;
        Ok(RuntimeContextReport {
            ok: true,
            schema_version: RUNTIME_CONTEXT_REPORT_SCHEMA_VERSION,
            snapshot: self.clone(),
            active_context_tokens: self.active_context_tokens(),
            usage_basis_points: self.usage_basis_points(),
            pressure: self.pressure(),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeContextReport {
    pub ok: bool,
    pub schema_version: &'static str,
    pub snapshot: RuntimeContextSnapshot,
    pub active_context_tokens: u64,
    pub usage_basis_points: u16,
    pub pressure: RuntimeContextPressure,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillDefinition {
    pub id: String,
    pub label: String,
    pub version: String,
    pub source_path: String,
    pub digest_sha256: String,
    #[serde(default)]
    pub catalog_tokens: u64,
    #[serde(default)]
    pub body_tokens: u64,
}

impl SkillDefinition {
    pub fn validate(&self) -> Result<()> {
        validate_id("skill.id", &self.id)?;
        validate_label("skill.label", &self.label)?;
        validate_version(&self.version)?;
        validate_absolute_path("skill.sourcePath", &self.source_path)?;
        if self.digest_sha256.len() != 64
            || !self
                .digest_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("skill.digestSha256 must contain exactly 64 hexadecimal characters");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillRegistry {
    pub schema_version: String,
    pub revision: u64,
    #[serde(default)]
    pub skills: Vec<SkillDefinition>,
}

impl SkillRegistry {
    pub fn validate(&self) -> Result<()> {
        validate_schema(
            "SkillRegistry",
            &self.schema_version,
            SKILL_REGISTRY_SCHEMA_VERSION,
        )?;
        let mut ids = BTreeSet::new();
        for skill in &self.skills {
            skill.validate()?;
            if !ids.insert(skill.id.to_ascii_lowercase()) {
                bail!("Skill Registry contains duplicate skill ID '{}'", skill.id);
            }
        }
        Ok(())
    }

    pub fn resolve(&self, id: &str) -> Result<&SkillDefinition> {
        self.skills
            .iter()
            .find(|skill| skill.id.eq_ignore_ascii_case(id))
            .with_context(|| format!("Skill Registry does not contain '{id}'"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveAccountLayer {
    pub id: String,
    pub label: String,
    pub home_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_host: Option<String>,
    pub has_auth: bool,
    pub has_config: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveExpertLayer {
    pub id: String,
    pub label: String,
    pub version: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveProjectLayer {
    pub id: String,
    pub project_id: String,
    pub project_root: String,
    pub has_project_home: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveSkill {
    pub id: String,
    pub label: String,
    pub version: String,
    pub source_path: String,
    pub target_path: String,
    pub digest_sha256: String,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveRuntimeManifest {
    pub ok: bool,
    pub schema_version: String,
    pub manifest_id: String,
    pub account: EffectiveAccountLayer,
    pub experts: Vec<EffectiveExpertLayer>,
    pub project: EffectiveProjectLayer,
    pub isolation: RuntimeIsolation,
    pub runtime_root: String,
    pub codex_home: String,
    pub skills: Vec<EffectiveSkill>,
    pub agents_files: Vec<String>,
    pub rule_files: Vec<String>,
    pub mcp_server_ids: Vec<String>,
    pub context: ContextFootprint,
    pub safety: SafetySummary,
}

pub struct EnvironmentResolver;

impl EnvironmentResolver {
    pub fn resolve(
        binding: &ProjectBinding,
        accounts: &[AccountProfile],
        expert_packs: &[ExpertPack],
        skill_registry: &SkillRegistry,
        runtime_root: &Path,
    ) -> Result<EffectiveRuntimeManifest> {
        binding.validate()?;
        skill_registry.validate()?;
        if !runtime_root.is_absolute() {
            bail!("runtimeRoot must be absolute");
        }

        let account = accounts
            .iter()
            .find(|item| item.id == binding.account_profile_id)
            .with_context(|| {
                format!(
                    "Account Profile '{}' was not provided",
                    binding.account_profile_id
                )
            })?;
        account.validate()?;

        let packs_by_id = expert_packs
            .iter()
            .map(|pack| (pack.id.as_str(), pack))
            .collect::<BTreeMap<_, _>>();
        let mut selected_packs = Vec::new();
        for id in &binding.expert_pack_ids {
            let pack = packs_by_id
                .get(id.as_str())
                .with_context(|| format!("Expert Pack '{id}' was not provided"))?;
            pack.validate()?;
            selected_packs.push(*pack);
        }

        let isolation = if binding.isolation == RuntimeIsolation::Hard
            || selected_packs
                .iter()
                .any(|pack| pack.requires_hard_isolation)
        {
            RuntimeIsolation::Hard
        } else {
            RuntimeIsolation::SharedAccountHome
        };

        let seed = serde_json::to_vec(&(
            binding,
            account,
            &selected_packs,
            skill_registry.revision,
            isolation,
        ))
        .context("failed to encode runtime manifest identity")?;
        let digest = Sha256::digest(seed);
        let manifest_id = format!("runtime-{}", hex_prefix(&digest, 16));
        let runtime_home = runtime_root.join(&binding.project_id).join(&manifest_id);
        let codex_home = match isolation {
            RuntimeIsolation::SharedAccountHome => PathBuf::from(&account.home_path),
            RuntimeIsolation::Hard => runtime_home.clone(),
        };
        let skill_target_root = match isolation {
            RuntimeIsolation::SharedAccountHome => {
                PathBuf::from(&binding.project_root).join(".agents/skills")
            }
            RuntimeIsolation::Hard => runtime_home.join("skills"),
        };

        let mut requested_skills = BTreeMap::<String, bool>::new();
        let mut context = ContextFootprint::default();
        let mut agents_files = Vec::new();
        let mut rule_files = Vec::new();
        let mut mcp_server_ids = Vec::new();
        for pack in &selected_packs {
            merge_requirements(&mut requested_skills, &pack.skills);
            merge_unique(&mut agents_files, &pack.agents_files);
            merge_unique(&mut rule_files, &pack.rule_files);
            merge_unique(&mut mcp_server_ids, &pack.mcp_server_ids);
            context.add_assign(&pack.context);
        }
        if let Some(project_home) = &binding.project_home {
            merge_requirements(&mut requested_skills, &project_home.skills);
            merge_unique(&mut agents_files, &project_home.agents_files);
            merge_unique(&mut rule_files, &project_home.rule_files);
            merge_unique(&mut mcp_server_ids, &project_home.mcp_server_ids);
            context.add_assign(&project_home.context);
        }

        let mut skills = Vec::new();
        for (skill_id, required) in requested_skills {
            match skill_registry.resolve(&skill_id) {
                Ok(skill) => {
                    context.skill_catalog_tokens = context
                        .skill_catalog_tokens
                        .saturating_add(skill.catalog_tokens);
                    context.skill_body_tokens =
                        context.skill_body_tokens.saturating_add(skill.body_tokens);
                    skills.push(EffectiveSkill {
                        id: skill.id.clone(),
                        label: skill.label.clone(),
                        version: skill.version.clone(),
                        source_path: skill.source_path.clone(),
                        target_path: skill_target_root
                            .join(&skill.id)
                            .to_string_lossy()
                            .into_owned(),
                        digest_sha256: skill.digest_sha256.clone(),
                        required,
                    });
                }
                Err(error) if required => return Err(error),
                Err(_) => {}
            }
        }
        context.validate()?;

        Ok(EffectiveRuntimeManifest {
            ok: true,
            schema_version: EFFECTIVE_RUNTIME_SCHEMA_VERSION.to_owned(),
            manifest_id,
            account: EffectiveAccountLayer {
                id: account.id.clone(),
                label: account.label.clone(),
                home_path: account.home_path.clone(),
                provider: account.provider.clone(),
                model: account.model.clone(),
                endpoint_host: account.endpoint_host.clone(),
                has_auth: account.has_auth,
                has_config: account.has_config,
            },
            experts: selected_packs
                .iter()
                .map(|pack| EffectiveExpertLayer {
                    id: pack.id.clone(),
                    label: pack.label.clone(),
                    version: pack.version.clone(),
                    capabilities: pack.capabilities.clone(),
                })
                .collect(),
            project: EffectiveProjectLayer {
                id: binding.id.clone(),
                project_id: binding.project_id.clone(),
                project_root: binding.project_root.clone(),
                has_project_home: binding
                    .project_home
                    .as_ref()
                    .is_some_and(|layer| !layer.is_empty()),
            },
            isolation,
            runtime_root: runtime_home.to_string_lossy().into_owned(),
            codex_home: codex_home.to_string_lossy().into_owned(),
            skills,
            agents_files,
            rule_files,
            mcp_server_ids,
            context,
            safety: SafetySummary {
                secrets_redacted: true,
                includes_secrets: false,
                reads_auth_contents: false,
                includes_local_paths: true,
            },
        })
    }
}

fn default_true() -> bool {
    true
}

fn merge_requirements(target: &mut BTreeMap<String, bool>, values: &[SkillRequirement]) {
    for value in values {
        target
            .entry(value.skill_id.clone())
            .and_modify(|required| *required |= value.required)
            .or_insert(value.required);
    }
}

fn merge_unique(target: &mut Vec<String>, values: &[String]) {
    for value in values {
        if !target.contains(value) {
            target.push(value.clone());
        }
    }
    target.sort();
}

fn validate_skill_requirements(values: &[SkillRequirement]) -> Result<()> {
    let mut ids = BTreeSet::new();
    for value in values {
        validate_id("skillRequirement.skillId", &value.skill_id)?;
        if !ids.insert(value.skill_id.to_ascii_lowercase()) {
            bail!(
                "skill requirements contain duplicate ID '{}'",
                value.skill_id
            );
        }
    }
    Ok(())
}

fn validate_unique_ids(name: &str, values: &[String]) -> Result<()> {
    let mut normalized = BTreeSet::new();
    for value in values {
        validate_id(name, value)?;
        if !normalized.insert(value.to_ascii_lowercase()) {
            bail!("{name} contains duplicate value '{value}'");
        }
    }
    Ok(())
}

fn validate_unique_labels(name: &str, values: &[String]) -> Result<()> {
    let mut normalized = BTreeSet::new();
    for value in values {
        validate_label(name, value)?;
        if !normalized.insert(value.to_ascii_lowercase()) {
            bail!("{name} contains duplicate value '{value}'");
        }
    }
    Ok(())
}

fn validate_unique_paths(name: &str, values: &[String]) -> Result<()> {
    let mut normalized = BTreeSet::new();
    for value in values {
        validate_absolute_path(name, value)?;
        if !normalized.insert(value) {
            bail!("{name} contains duplicate path '{value}'");
        }
    }
    Ok(())
}

fn validate_project_paths(name: &str, values: &[String], project_root: &Path) -> Result<()> {
    validate_unique_paths(name, values)?;
    reject_sensitive_paths(name, values)?;
    for value in values {
        if !Path::new(value).starts_with(project_root) {
            bail!("{name} path '{value}' must stay inside projectRoot");
        }
    }
    Ok(())
}

fn reject_sensitive_paths(name: &str, values: &[String]) -> Result<()> {
    for value in values {
        let path = Path::new(value);
        let sensitive = path.components().any(|component| {
            let value = component.as_os_str().to_string_lossy().to_ascii_lowercase();
            matches!(
                value.as_str(),
                "auth.json" | ".env" | "credentials" | "cookies" | "token.json"
            )
        });
        if sensitive {
            bail!("{name} must not reference credential or secret-bearing path '{value}'");
        }
    }
    Ok(())
}

fn validate_schema(name: &str, value: &str, expected: &str) -> Result<()> {
    if value != expected {
        bail!("{name}.schemaVersion must be '{expected}'");
    }
    Ok(())
}

fn validate_id(name: &str, value: &str) -> Result<()> {
    if value.len() < 2
        || value.len() > 96
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        bail!("{name} must contain 2..=96 safe identifier characters");
    }
    Ok(())
}

fn validate_label(name: &str, value: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 160 || trimmed.contains(['\0', '\r', '\n']) {
        bail!("{name} must contain 1..=160 single-line bytes");
    }
    Ok(())
}

fn validate_optional_label(name: &str, value: Option<&str>) -> Result<()> {
    if let Some(value) = value {
        validate_label(name, value)?;
    }
    Ok(())
}

fn validate_version(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        bail!("version must contain 1..=64 safe version characters");
    }
    Ok(())
}

fn validate_absolute_path(name: &str, value: &str) -> Result<()> {
    let path = Path::new(value);
    if !path.is_absolute()
        || value.contains('\0')
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        bail!("{name} must be a normalized absolute local path without '.' or '..'");
    }
    Ok(())
}

fn validate_endpoint_host(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 253
        || value.contains(['/', '\\', '@', ':'])
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        bail!("accountProfile.endpointHost must be a hostname without scheme, port, or path");
    }
    Ok(())
}

fn hex_prefix(bytes: &[u8], length: usize) -> String {
    let mut result = String::with_capacity(length);
    for byte in bytes {
        result.push_str(&format!("{byte:02x}"));
        if result.len() >= length {
            result.truncate(length);
            break;
        }
    }
    result
}

pub fn compute_skill_digest(root: &Path) -> Result<String> {
    if !root.is_dir() {
        bail!("Skill source must be a directory: {}", root.display());
    }
    let mut entries = Vec::<PathBuf>::new();
    collect_skill_entries(root, root, &mut entries)?;
    entries.sort();
    let mut hasher = Sha256::new();
    for relative in entries {
        let path = root.join(&relative);
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("failed to inspect Skill entry {}", path.display()))?;
        let relative_text = relative.to_string_lossy();
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(&path)
                .with_context(|| format!("failed to read Skill symlink {}", path.display()))?;
            hasher.update(b"symlink\0");
            hasher.update(relative_text.as_bytes());
            hasher.update(b"\0");
            hasher.update(target.to_string_lossy().as_bytes());
            hasher.update(b"\0");
        } else if metadata.is_dir() {
            hasher.update(b"directory\0");
            hasher.update(relative_text.as_bytes());
            hasher.update(b"\0");
        } else if metadata.is_file() {
            let contents = fs::read(&path)
                .with_context(|| format!("failed to read Skill file {}", path.display()))?;
            hasher.update(b"file\0");
            hasher.update(relative_text.as_bytes());
            hasher.update(b"\0");
            hasher.update((contents.len() as u64).to_le_bytes());
            hasher.update(&contents);
        }
    }
    Ok(hex_prefix(&hasher.finalize(), 64))
}

fn collect_skill_entries(root: &Path, current: &Path, entries: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(current)
        .with_context(|| format!("failed to read Skill directory {}", current.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .context("Skill entry escaped its source root")?
            .to_path_buf();
        entries.push(relative);
        if entry.file_type()?.is_dir() {
            collect_skill_entries(root, &path, entries)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(id: &str, source: &str) -> SkillDefinition {
        SkillDefinition {
            id: id.to_owned(),
            label: id.to_owned(),
            version: "1.0.0".to_owned(),
            source_path: source.to_owned(),
            digest_sha256: "a".repeat(64),
            catalog_tokens: 100,
            body_tokens: 400,
        }
    }

    fn account() -> AccountProfile {
        AccountProfile {
            schema_version: ACCOUNT_PROFILE_SCHEMA_VERSION.to_owned(),
            id: "account-main".to_owned(),
            label: "Main".to_owned(),
            home_path: "/tmp/account-main".to_owned(),
            provider: Some("openai".to_owned()),
            model: Some("gpt-test".to_owned()),
            endpoint_host: Some("api.example.com".to_owned()),
            has_auth: true,
            has_config: true,
        }
    }

    #[test]
    fn resolves_three_layers_into_one_deterministic_manifest() {
        let binding = ProjectBinding {
            schema_version: PROJECT_BINDING_SCHEMA_VERSION.to_owned(),
            id: "binding-paper".to_owned(),
            project_id: "paper-crop".to_owned(),
            project_root: "/tmp/paper-crop".to_owned(),
            account_profile_id: "account-main".to_owned(),
            expert_pack_ids: vec!["expert-research".to_owned()],
            project_home: Some(ProjectHomeLayer {
                skills: vec![SkillRequirement {
                    skill_id: "project-marker".to_owned(),
                    required: true,
                }],
                ..ProjectHomeLayer {
                    skills: Vec::new(),
                    agents_files: Vec::new(),
                    rule_files: Vec::new(),
                    mcp_server_ids: Vec::new(),
                    context: ContextFootprint::default(),
                }
            }),
            isolation: RuntimeIsolation::SharedAccountHome,
        };
        let pack = ExpertPack {
            schema_version: EXPERT_PACK_SCHEMA_VERSION.to_owned(),
            id: "expert-research".to_owned(),
            label: "Research".to_owned(),
            version: "1.0.0".to_owned(),
            description: String::new(),
            capabilities: vec!["papers".to_owned()],
            skills: vec![SkillRequirement {
                skill_id: "arxiv".to_owned(),
                required: true,
            }],
            agents_files: Vec::new(),
            rule_files: Vec::new(),
            mcp_server_ids: vec!["notebook".to_owned()],
            requires_hard_isolation: true,
            context: ContextFootprint {
                agents_tokens: 200,
                ..ContextFootprint::default()
            },
        };
        let registry = SkillRegistry {
            schema_version: SKILL_REGISTRY_SCHEMA_VERSION.to_owned(),
            revision: 1,
            skills: vec![
                skill("arxiv", "/tmp/skills/arxiv"),
                skill(
                    "project-marker",
                    "/tmp/paper-crop/.agents/source/project-marker",
                ),
            ],
        };

        let first = EnvironmentResolver::resolve(
            &binding,
            &[account()],
            std::slice::from_ref(&pack),
            &registry,
            Path::new("/tmp/runtimes"),
        )
        .expect("resolve");
        let second = EnvironmentResolver::resolve(
            &binding,
            &[account()],
            &[pack],
            &registry,
            Path::new("/tmp/runtimes"),
        )
        .expect("resolve");

        assert_eq!(first.manifest_id, second.manifest_id);
        assert_eq!(first.isolation, RuntimeIsolation::Hard);
        assert_eq!(first.skills.len(), 2);
        assert_eq!(first.context.total_tokens(), 1_200);
        assert!(first.codex_home.contains(&first.manifest_id));
        assert!(!first.safety.reads_auth_contents);
    }

    #[test]
    fn project_home_rejects_auth_material() {
        let layer = ProjectHomeLayer {
            skills: Vec::new(),
            agents_files: vec!["/tmp/project/auth.json".to_owned()],
            rule_files: Vec::new(),
            mcp_server_ids: Vec::new(),
            context: ContextFootprint::default(),
        };
        let error = layer
            .validate(Path::new("/tmp/project"))
            .expect_err("auth must be rejected");
        assert!(error.to_string().contains("auth.json"));
    }

    #[test]
    fn project_home_rejects_lexical_path_escape() {
        let layer = ProjectHomeLayer {
            skills: Vec::new(),
            agents_files: vec!["/tmp/project/../outside/AGENTS.md".to_owned()],
            rule_files: Vec::new(),
            mcp_server_ids: Vec::new(),
            context: ContextFootprint::default(),
        };
        let error = layer
            .validate(Path::new("/tmp/project"))
            .expect_err("path traversal must be rejected");
        assert!(error.to_string().contains("normalized absolute"));
    }

    #[test]
    fn missing_required_skill_fails_closed() {
        let binding = ProjectBinding {
            schema_version: PROJECT_BINDING_SCHEMA_VERSION.to_owned(),
            id: "binding-test".to_owned(),
            project_id: "project-test".to_owned(),
            project_root: "/tmp/project".to_owned(),
            account_profile_id: "account-main".to_owned(),
            expert_pack_ids: Vec::new(),
            project_home: Some(ProjectHomeLayer {
                skills: vec![SkillRequirement {
                    skill_id: "missing-skill".to_owned(),
                    required: true,
                }],
                agents_files: Vec::new(),
                rule_files: Vec::new(),
                mcp_server_ids: Vec::new(),
                context: ContextFootprint::default(),
            }),
            isolation: RuntimeIsolation::Hard,
        };
        let registry = SkillRegistry {
            schema_version: SKILL_REGISTRY_SCHEMA_VERSION.to_owned(),
            revision: 0,
            skills: Vec::new(),
        };
        let error = EnvironmentResolver::resolve(
            &binding,
            &[account()],
            &[],
            &registry,
            Path::new("/tmp/runtimes"),
        )
        .expect_err("required skill");
        assert!(error.to_string().contains("missing-skill"));
    }

    #[test]
    fn runtime_context_keeps_loaded_bodies_and_cache_attribution_separate() {
        let snapshot = RuntimeContextSnapshot {
            schema_version: RUNTIME_CONTEXT_SNAPSHOT_SCHEMA_VERSION.to_owned(),
            run_id: "run-test".to_owned(),
            attempt_id: "attempt-test".to_owned(),
            context_window_tokens: 200_000,
            skill_catalog_tokens: 800,
            loaded_skill_body_tokens: 1_200,
            agents_tokens: 400,
            rules_tokens: 300,
            tool_schema_tokens: 600,
            conversation_history_tokens: 700,
            cached_reuse_tokens: 1_500,
            loaded_skill_ids: vec!["research".to_owned()],
        };
        snapshot.validate().expect("valid context snapshot");
        assert_eq!(snapshot.active_context_tokens(), 4_000);
        assert_eq!(snapshot.usage_basis_points(), 200);
        assert_eq!(snapshot.pressure(), RuntimeContextPressure::Notice);
    }
}
