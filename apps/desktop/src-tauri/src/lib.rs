use codexhome_core::{
    discover, DiscoveryOptions, HomeManager, ObservabilityFilter, ObservabilityStore,
    ObservabilitySummary, RegistryReport, RegistryStore,
};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HomeRequest {
    alias: String,
    label: Option<String>,
    path: String,
    #[serde(default)]
    specialties: Vec<String>,
    #[serde(default)]
    dry_run: bool,
    source: Option<String>,
    #[serde(default)]
    copy_capabilities: bool,
}

fn store() -> Result<RegistryStore, String> {
    RegistryStore::from_environment(None).map_err(|error| format!("{error:#}"))
}

#[tauri::command]
fn registry_list() -> Result<RegistryReport, String> {
    store()?.report().map_err(|error| format!("{error:#}"))
}

#[tauri::command]
fn discover_homes() -> Result<codexhome_core::DiscoveryReport, String> {
    let options =
        DiscoveryOptions::from_environment(Vec::new()).map_err(|error| format!("{error:#}"))?;
    Ok(discover(&options))
}

#[tauri::command]
fn observability_summary() -> Result<ObservabilitySummary, String> {
    ObservabilityStore::from_environment(None)
        .and_then(|store| store.summary(&ObservabilityFilter::default()))
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
fn create_home(request: HomeRequest) -> Result<codexhome_core::HomeMutationResult, String> {
    HomeManager::new(store()?)
        .create(
            &request.alias,
            request.label.as_deref(),
            &PathBuf::from(request.path),
            request.specialties,
            request.dry_run,
        )
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
fn import_home(request: HomeRequest) -> Result<codexhome_core::HomeMutationResult, String> {
    HomeManager::new(store()?)
        .import(
            &PathBuf::from(request.path),
            &request.alias,
            request.label.as_deref(),
            request.specialties,
            request.dry_run,
        )
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
fn clone_home(request: HomeRequest) -> Result<codexhome_core::HomeMutationResult, String> {
    let source = request
        .source
        .as_deref()
        .ok_or_else(|| "clone source is required".to_owned())?;
    HomeManager::new(store()?)
        .clone_home(
            source,
            &request.alias,
            request.label.as_deref(),
            &PathBuf::from(request.path),
            request.specialties,
            request.copy_capabilities,
            request.dry_run,
        )
        .map_err(|error| format!("{error:#}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            registry_list,
            discover_homes,
            observability_summary,
            create_home,
            import_home,
            clone_home
        ])
        .run(tauri::generate_context!())
        .expect("failed to run CodexHome Manager desktop");
}
