use crate::{
    expand_home, inspect_home_path, stable_id, user_home, CodexHomeSummary, SafetySummary,
};
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

pub const REGISTRY_SCHEMA_VERSION: &str = "codexhome.registry.v1";
pub const REGISTRY_REPORT_SCHEMA_VERSION: &str = "codexhome.registry-report.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    pub schema_version: String,
    pub revision: u64,
    pub homes: Vec<RegistryEntry>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            schema_version: REGISTRY_SCHEMA_VERSION.to_owned(),
            revision: 0,
            homes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryEntry {
    pub id: String,
    pub alias: String,
    pub label: String,
    pub path: String,
    pub specialties: Vec<String>,
    pub origin: RegistryOrigin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<String>,
}

impl RegistryEntry {
    pub fn new(
        alias: &str,
        label: Option<&str>,
        path: &Path,
        specialties: Vec<String>,
        origin: RegistryOrigin,
        derived_from: Option<String>,
    ) -> Result<Self> {
        let alias = normalize_alias(alias)?;
        let specialties = normalize_specialties(specialties)?;
        let path = normalize_absolute_path(path)?;
        let label = label
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| alias.trim_start_matches('@').replace('-', " "));
        if label.chars().count() > 80 {
            bail!("Home label must be 80 characters or fewer");
        }
        Ok(Self {
            id: stable_id(&path),
            alias,
            label,
            path: path.to_string_lossy().into_owned(),
            specialties,
            origin,
            derived_from,
        })
    }

    pub fn path_buf(&self) -> PathBuf {
        PathBuf::from(&self.path)
    }

    pub fn matches(&self, query: &str) -> bool {
        self.id == query
            || self.alias.eq_ignore_ascii_case(query)
            || self.label.eq_ignore_ascii_case(query)
            || self.path == query
            || (!query.starts_with('@')
                && self
                    .alias
                    .trim_start_matches('@')
                    .eq_ignore_ascii_case(query))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RegistryOrigin {
    Created,
    Imported,
    Cloned,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryReport {
    pub ok: bool,
    pub schema_version: &'static str,
    pub registry_path: String,
    pub revision: u64,
    pub homes: Vec<RegisteredHomeView>,
    pub safety: SafetySummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredHomeView {
    #[serde(flatten)]
    pub entry: RegistryEntry,
    pub available: bool,
    pub summary: Option<CodexHomeSummary>,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RegistryStore {
    path: PathBuf,
}

impl RegistryStore {
    pub fn from_environment(override_path: Option<PathBuf>) -> Result<Self> {
        let home = user_home().context("cannot discover the current user home directory")?;
        let path = override_path
            .or_else(|| env::var_os("CODEXHOME_REGISTRY").map(PathBuf::from))
            .unwrap_or_else(|| home.join(".codexhome/registry.json"));
        Ok(Self::new(expand_home(&path, &home)))
    }

    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Registry> {
        load_registry(&self.path)
    }

    pub fn report(&self) -> Result<RegistryReport> {
        let registry = self.load()?;
        let homes = registry
            .homes
            .iter()
            .cloned()
            .map(|entry| {
                let path = entry.path_buf();
                match inspect_home_path(&path, "registry") {
                    Ok((summary, warning)) => {
                        let issues = warning.map(|item| vec![item.message]).unwrap_or_default();
                        RegisteredHomeView {
                            entry,
                            available: true,
                            summary: Some(summary),
                            issues,
                        }
                    }
                    Err(error) => RegisteredHomeView {
                        entry,
                        available: false,
                        summary: None,
                        issues: vec![error.to_string()],
                    },
                }
            })
            .collect();
        Ok(RegistryReport {
            ok: true,
            schema_version: REGISTRY_REPORT_SCHEMA_VERSION,
            registry_path: self.path.to_string_lossy().into_owned(),
            revision: registry.revision,
            homes,
            safety: SafetySummary {
                secrets_redacted: true,
                includes_secrets: false,
                reads_auth_contents: false,
                includes_local_paths: true,
            },
        })
    }

    pub fn resolve(&self, query: &str) -> Result<RegistryEntry> {
        let registry = self.load()?;
        let matches = registry
            .homes
            .iter()
            .filter(|entry| entry.matches(query))
            .cloned()
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [entry] => Ok(entry.clone()),
            [] => bail!(
                "registered Home '{query}' was not found; run `codexhome registry list` or import it first"
            ),
            _ => {
                let aliases = matches
                    .iter()
                    .map(|entry| entry.alias.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                bail!("registered Home '{query}' is ambiguous; use one of: {aliases}")
            }
        }
    }

    pub fn preview_add(&self, entry: &RegistryEntry) -> Result<()> {
        let registry = self.load()?;
        validate_candidate(&registry, entry)
    }

    pub fn add(&self, entry: RegistryEntry) -> Result<Registry> {
        let parent = registry_parent(&self.path)?;
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create registry directory {}", parent.display()))?;
        secure_directory(parent)?;
        let lock_path = lock_path(&self.path)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("failed to open registry lock {}", lock_path.display()))?;
        secure_file(&lock)?;
        lock.lock_exclusive()
            .with_context(|| format!("failed to lock registry {}", self.path.display()))?;

        let mut registry = self.load()?;
        validate_candidate(&registry, &entry)?;
        registry.homes.push(entry);
        registry
            .homes
            .sort_by(|left, right| left.alias.cmp(&right.alias));
        registry.revision = registry
            .revision
            .checked_add(1)
            .context("registry revision overflow")?;
        write_registry(&self.path, &registry)?;
        FileExt::unlock(&lock).context("failed to unlock registry")?;
        Ok(registry)
    }
}

pub fn normalize_alias(value: &str) -> Result<String> {
    let value = value.trim();
    let body = value
        .strip_prefix('@')
        .unwrap_or(value)
        .to_ascii_lowercase();
    let valid = (2..=32).contains(&body.len())
        && !body.starts_with('@')
        && body
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_lowercase())
        && body.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        });
    if !valid {
        bail!(
            "alias must be @ plus 2-32 lowercase ASCII letters, digits, or hyphens and start with a letter"
        );
    }
    Ok(format!("@{body}"))
}

pub fn normalize_specialties(values: Vec<String>) -> Result<Vec<String>> {
    let mut normalized = BTreeSet::new();
    for value in values {
        let value = value
            .trim()
            .to_lowercase()
            .replace(char::is_whitespace, "-");
        if value.is_empty() {
            bail!("specialty cannot be empty");
        }
        if value.chars().count() > 48 {
            bail!("specialty '{value}' must be 48 characters or fewer");
        }
        if !value
            .chars()
            .all(|character| character.is_alphanumeric() || matches!(character, '-' | '_'))
        {
            bail!(
                "specialty '{value}' may contain only letters, numbers, hyphens, underscores, or spaces"
            );
        }
        normalized.insert(value);
    }
    Ok(normalized.into_iter().collect())
}

pub(crate) fn normalize_absolute_path(path: &Path) -> Result<PathBuf> {
    let home = user_home().context("cannot discover the current user home directory")?;
    let path = expand_home(path, &home);
    if !path.is_absolute() {
        bail!("Home path must be absolute: {}", path.display());
    }
    if let Ok(canonical) = path.canonicalize() {
        return Ok(canonical);
    }
    let mut clean = PathBuf::new();
    for component in path.components() {
        use std::path::Component;
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !clean.pop() {
                    bail!("Home path escapes its filesystem root: {}", path.display());
                }
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                clean.push(component.as_os_str());
            }
        }
    }
    let mut cursor = clean.as_path();
    let mut missing = Vec::new();
    while !cursor.exists() {
        let name = cursor.file_name().with_context(|| {
            format!("cannot resolve Home path ancestor for {}", clean.display())
        })?;
        missing.push(name.to_os_string());
        cursor = cursor.parent().with_context(|| {
            format!("cannot resolve Home path ancestor for {}", clean.display())
        })?;
    }
    let mut resolved = cursor
        .canonicalize()
        .with_context(|| format!("failed to resolve Home path ancestor {}", cursor.display()))?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

fn load_registry(path: &Path) -> Result<Registry> {
    if !path.exists() {
        return Ok(Registry::default());
    }
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read registry {}", path.display()))?;
    let registry = serde_json::from_str::<Registry>(&content)
        .with_context(|| format!("registry {} is not valid JSON", path.display()))?;
    validate_registry(&registry)?;
    Ok(registry)
}

fn validate_registry(registry: &Registry) -> Result<()> {
    if registry.schema_version != REGISTRY_SCHEMA_VERSION {
        bail!(
            "unsupported registry schema '{}'; expected '{}'",
            registry.schema_version,
            REGISTRY_SCHEMA_VERSION
        );
    }
    let mut aliases = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for entry in &registry.homes {
        if normalize_alias(&entry.alias)? != entry.alias {
            bail!("registry alias is not canonical: {}", entry.alias);
        }
        if normalize_specialties(entry.specialties.clone())? != entry.specialties {
            bail!("registry specialties are not canonical for {}", entry.alias);
        }
        if entry.label.trim().is_empty() || entry.label.chars().count() > 80 {
            bail!("registry label is invalid for {}", entry.alias);
        }
        let normalized_path = normalize_absolute_path(Path::new(&entry.path))?;
        if stable_id(&normalized_path) != entry.id {
            bail!(
                "registry Home ID does not match its path for {}",
                entry.alias
            );
        }
        if !aliases.insert(entry.alias.to_ascii_lowercase()) {
            bail!("registry contains duplicate alias {}", entry.alias);
        }
        if !ids.insert(&entry.id) {
            bail!("registry contains duplicate Home ID {}", entry.id);
        }
        if !paths.insert(path_comparison_key(&entry.path)) {
            bail!("registry contains duplicate Home path {}", entry.path);
        }
    }
    Ok(())
}

fn validate_candidate(registry: &Registry, candidate: &RegistryEntry) -> Result<()> {
    validate_registry(registry)?;
    if let Some(existing) = registry
        .homes
        .iter()
        .find(|entry| entry.alias.eq_ignore_ascii_case(&candidate.alias))
    {
        bail!(
            "alias {} is already assigned to {} ({})",
            candidate.alias,
            existing.label,
            existing.path
        );
    }
    if let Some(existing) = registry.homes.iter().find(|entry| {
        entry.id == candidate.id
            || path_comparison_key(&entry.path) == path_comparison_key(&candidate.path)
    }) {
        bail!(
            "Home path {} is already registered as {}",
            candidate.path,
            existing.alias
        );
    }
    Ok(())
}

fn write_registry(path: &Path, registry: &Registry) -> Result<()> {
    let parent = registry_parent(path)?;
    let mut temp = NamedTempFile::new_in(parent).with_context(|| {
        format!(
            "failed to create registry transaction in {}",
            parent.display()
        )
    })?;
    let content = serde_json::to_vec_pretty(registry).context("failed to serialize registry")?;
    temp.write_all(&content)
        .context("failed to write registry transaction")?;
    temp.write_all(b"\n")
        .context("failed to finish registry transaction")?;
    temp.as_file()
        .sync_all()
        .context("failed to sync registry transaction")?;
    secure_file(temp.as_file())?;
    temp.persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to atomically replace registry {}", path.display()))?;
    Ok(())
}

fn registry_parent(path: &Path) -> Result<&Path> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("registry path must include a parent directory")
}

fn lock_path(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .context("registry filename is not valid UTF-8")?;
    Ok(registry_parent(path)?.join(format!(".{name}.lock")))
}

#[cfg(windows)]
fn path_comparison_key(path: &str) -> String {
    path.to_ascii_lowercase()
}

#[cfg(not(windows))]
fn path_comparison_key(path: &str) -> String {
    path.to_owned()
}

#[cfg(unix)]
fn secure_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to secure {}", path.display()))
}

#[cfg(not(unix))]
fn secure_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn secure_file(file: &File) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .context("failed to secure registry file")
}

#[cfg(not(unix))]
fn secure_file(_file: &File) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn aliases_and_unicode_specialties_are_normalized() {
        assert_eq!(normalize_alias("Frontend").expect("alias"), "@frontend");
        assert_eq!(
            normalize_specialties(vec!["UI Design".into(), "前端".into(), "ui-design".into()])
                .expect("specialties"),
            ["ui-design", "前端"]
        );
        assert!(normalize_alias("@1invalid").is_err());
        assert!(normalize_alias("@@frontend").is_err());
    }

    #[test]
    fn registry_rejects_duplicate_aliases_and_paths() {
        let temp = TempDir::new().expect("temp");
        let store = RegistryStore::new(temp.path().join("registry.json"));
        let first = RegistryEntry::new(
            "@frontend",
            None,
            &temp.path().join("one"),
            vec!["ui".into()],
            RegistryOrigin::Imported,
            None,
        )
        .expect("entry");
        store.add(first.clone()).expect("add first");
        let duplicate_alias = RegistryEntry::new(
            "@frontend",
            None,
            &temp.path().join("two"),
            vec![],
            RegistryOrigin::Imported,
            None,
        )
        .expect("entry");
        assert!(store.add(duplicate_alias).is_err());
        let duplicate_path = RegistryEntry::new(
            "@reviewer",
            None,
            &first.path_buf(),
            vec![],
            RegistryOrigin::Imported,
            None,
        )
        .expect("entry");
        assert!(store.add(duplicate_path).is_err());
    }

    #[test]
    fn registry_round_trip_is_atomic_and_resolvable() {
        let temp = TempDir::new().expect("temp");
        let store = RegistryStore::new(temp.path().join("state/registry.json"));
        let entry = RegistryEntry::new(
            "research",
            Some("Research Family"),
            &temp.path().join("research"),
            vec!["papers".into()],
            RegistryOrigin::Created,
            None,
        )
        .expect("entry");
        let result = store.add(entry).expect("add");
        assert_eq!(result.revision, 1);
        assert_eq!(
            store.resolve("@research").expect("resolve").label,
            "Research Family"
        );
        assert_eq!(
            store.resolve("research").expect("resolve").alias,
            "@research"
        );
    }
}
