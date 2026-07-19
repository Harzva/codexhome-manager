import { invoke } from "@tauri-apps/api/core";
import "./styles.css";
import type { HomeView, MutationResult, RegistryReport } from "./types";

type ViewMode = "homes" | "create" | "import" | "clone";

const mockReport: RegistryReport = {
  ok: true,
  schemaVersion: "codexhome.registry-report.v1",
  registryPath: "~/.codexhome/registry.json",
  revision: 7,
  homes: [
    { id: "8e771a9d23ac", alias: "@frontend", label: "Frontend Family", path: "/demo/homes/frontend", specialties: ["ui", "figma", "browser"], origin: "created", available: true, summary: { provider: "OpenAI", model: "gpt-5.4", skillCount: 18, mcpServerCount: 3, ruleCount: 2, hookCount: 1 }, issues: [] },
    { id: "ac1801e7ff41", alias: "@research", label: "Research House", path: "/demo/homes/research", specialties: ["papers", "data", "citations"], origin: "imported", available: true, summary: { provider: "Inroi", model: "gpt-5.4", skillCount: 12, mcpServerCount: 4, ruleCount: 1, hookCount: 0 }, issues: [] },
    { id: "911b42aa03e1", alias: "@reviewer", label: "Review Family", path: "/demo/homes/reviewer", specialties: ["review", "security"], origin: "cloned", available: false, summary: null, issues: ["Home directory is unavailable"] },
  ],
};

let report = mockReport;
let mode: ViewMode = "homes";
let query = "";
let preview: MutationResult | null = null;
let busy = false;
let notice = "";

const isTauri = () => "__TAURI_INTERNALS__" in window;
const safe = (value: unknown) => String(value).replace(/[&<>"']/g, (character) => ({
  "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
})[character]!);

async function call<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  if (!isTauri()) throw new Error("Desktop bridge is unavailable in browser preview");
  return invoke<T>(command, args);
}

async function refresh() {
  busy = true;
  notice = "";
  render();
  try {
    report = await call<RegistryReport>("registry_list");
  } catch (error) {
    if (isTauri()) notice = String(error);
  } finally {
    busy = false;
    render();
  }
}

function filteredHomes(): HomeView[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return report.homes;
  return report.homes.filter((home) =>
    [home.alias, home.label, home.path, ...home.specialties].some((value) => value.toLowerCase().includes(needle)),
  );
}

function icon(name: string) {
  const icons: Record<string, string> = {
    homes: "⌂", create: "+", import: "↙", clone: "⧉", refresh: "↻", search: "⌕",
  };
  return `<span class="icon" aria-hidden="true">${icons[name]}</span>`;
}

function homeCard(home: HomeView) {
  const summary = home.summary;
  return `<article class="home-card ${home.available ? "" : "is-missing"}">
    <div class="card-top"><div><span class="alias">${safe(home.alias)}</span><h3>${safe(home.label)}</h3></div><span class="status ${home.available ? "ready" : "missing"}">${home.available ? "Ready" : "Unavailable"}</span></div>
    <div class="tags">${(home.specialties.length ? home.specialties : ["general"]).map((tag) => `<span>${safe(tag)}</span>`).join("")}</div>
    <dl class="metrics"><div><dt>Skills</dt><dd>${summary?.skillCount ?? "—"}</dd></div><div><dt>MCP</dt><dd>${summary?.mcpServerCount ?? "—"}</dd></div><div><dt>Rules</dt><dd>${summary?.ruleCount ?? "—"}</dd></div></dl>
    <div class="card-meta"><span>${safe(summary?.provider ?? "Provider unknown")}</span><span>${safe(summary?.model ?? home.origin)}</span></div>
    <p class="path" title="${safe(home.path)}">${safe(home.path)}</p>
    ${home.issues.map((issue) => `<p class="issue">${safe(issue)}</p>`).join("")}
  </article>`;
}

function formView(kind: Exclude<ViewMode, "homes">) {
  const sourceField = kind === "clone" ? `<label>Source family<select name="source" required>${report.homes.map((home) => `<option value="${safe(home.alias)}">${safe(home.alias)} · ${safe(home.label)}</option>`).join("")}</select></label>` : "";
  const pathLabel = kind === "import" ? "Existing Home path" : "New Home path";
  return `<section class="form-shell"><div class="form-intro"><span class="eyebrow">${kind} home</span><h2>${kind === "create" ? "Start a new family" : kind === "import" ? "Welcome an existing family" : "Create a safe branch"}</h2><p>${kind === "clone" ? "Authentication, sessions, databases and provider credentials stay behind." : "Aliases make this Home addressable by people and future agents."}</p></div>
    <form id="home-form" data-kind="${kind}">
      ${sourceField}
      <div class="field-row"><label>Alias<input name="alias" placeholder="@frontend" pattern="@?[a-z][a-z0-9-]{1,31}" required /></label><label>Family label<input name="label" placeholder="Frontend Family" /></label></div>
      <label>${pathLabel}<input name="path" placeholder="/absolute/path/to/codex-home" required /></label>
      <label>Specialties<input name="specialties" placeholder="ui, browser, figma" /><small>Comma-separated; Unicode labels are supported.</small></label>
      ${kind === "clone" ? `<label class="check"><input type="checkbox" name="copyCapabilities" /><span><strong>Copy capabilities</strong><small>Skills, Rules and Hooks only; secret filenames and symlinks are skipped.</small></span></label>` : ""}
      <div class="form-actions"><button type="button" class="button ghost" data-nav="homes">Cancel</button><button class="button primary" type="submit">Preview changes</button></div>
    </form>
    ${previewPanel()}
  </section>`;
}

function previewPanel() {
  if (!preview) return "";
  return `<aside class="preview-panel"><div><span class="eyebrow">Dry-run passed</span><h3>${safe(preview.entry.alias)} is ready to ${safe(preview.action)}</h3></div><ol>${preview.plannedActions.map((item) => `<li>${safe(item)}</li>`).join("")}</ol>${preview.warnings.map((item) => `<p class="warning">${safe(item)}</p>`).join("")}<button id="apply-change" class="button primary">Confirm and apply</button></aside>`;
}

function render() {
  const available = report.homes.filter((home) => home.available).length;
  const skills = report.homes.reduce((sum, home) => sum + (home.summary?.skillCount ?? 0), 0);
  document.querySelector<HTMLDivElement>("#app")!.innerHTML = `<div class="app-shell">
    <aside class="sidebar"><div class="brand"><div class="brand-mark">CH</div><div><strong>CodexHome</strong><span>Manager</span></div></div><nav aria-label="Primary navigation">
      ${(["homes", "create", "import", "clone"] as ViewMode[]).map((item) => `<button class="nav-item ${mode === item ? "active" : ""}" data-nav="${item}">${icon(item)}<span>${item[0].toUpperCase() + item.slice(1)}</span></button>`).join("")}
    </nav><div class="privacy"><span class="privacy-dot"></span><div><strong>Local only</strong><span>Credentials never enter the registry</span></div></div></aside>
    <main><header><div><span class="eyebrow">Agent households</span><h1>${mode === "homes" ? "Your specialist families" : "Home lifecycle"}</h1></div><div class="header-actions"><span class="revision">Registry r${report.revision}</span><button class="icon-button" id="refresh" aria-label="Refresh">${icon("refresh")}</button></div></header>
      ${notice ? `<div class="notice">${safe(notice)}</div>` : ""}
      ${mode === "homes" ? `<section class="summary"><div><span>Registered</span><strong>${report.homes.length}</strong></div><div><span>Available</span><strong>${available}</strong></div><div><span>Total skills</span><strong>${skills}</strong></div><div class="registry-location"><span>Registry</span><strong title="${safe(report.registryPath)}">${safe(report.registryPath)}</strong></div></section>
      <div class="toolbar"><label class="search">${icon("search")}<input id="search" value="${safe(query)}" placeholder="Search alias, family, specialty…" /></label><button class="button primary" data-nav="create">${icon("create")} New Home</button></div>
      <section class="home-grid">${filteredHomes().map(homeCard).join("") || `<div class="empty"><strong>No matching families</strong><span>Try another alias or specialty.</span></div>`}</section>` : formView(mode)}
      ${busy ? `<div class="loading">Refreshing registry…</div>` : ""}
    </main></div>`;
  bindEvents();
}

function bindEvents() {
  document.querySelectorAll<HTMLElement>("[data-nav]").forEach((element) => element.addEventListener("click", () => { mode = element.dataset.nav as ViewMode; preview = null; notice = ""; render(); }));
  document.querySelector<HTMLInputElement>("#search")?.addEventListener("input", (event) => { query = (event.target as HTMLInputElement).value; render(); document.querySelector<HTMLInputElement>("#search")?.focus(); });
  document.querySelector("#refresh")?.addEventListener("click", refresh);
  document.querySelector<HTMLFormElement>("#home-form")?.addEventListener("submit", submitPreview);
  document.querySelector("#apply-change")?.addEventListener("click", applyChange);
}

let pendingRequest: { command: string; request: Record<string, unknown> } | null = null;

async function submitPreview(event: SubmitEvent) {
  event.preventDefault();
  const form = event.currentTarget as HTMLFormElement;
  const data = new FormData(form);
  const kind = form.dataset.kind as Exclude<ViewMode, "homes">;
  const request: Record<string, unknown> = { alias: data.get("alias"), label: data.get("label") || null, path: data.get("path"), specialties: String(data.get("specialties") || "").split(",").map((item) => item.trim()).filter(Boolean), dryRun: true };
  if (kind === "clone") { request.source = data.get("source"); request.copyCapabilities = data.get("copyCapabilities") === "on"; }
  pendingRequest = { command: `${kind}_home`, request };
  try { preview = await call<MutationResult>(pendingRequest.command, { request }); notice = ""; } catch (error) { notice = String(error); preview = null; }
  render();
}

async function applyChange() {
  if (!pendingRequest) return;
  busy = true; render();
  try { pendingRequest.request.dryRun = false; await call(pendingRequest.command, { request: pendingRequest.request }); mode = "homes"; preview = null; pendingRequest = null; await refresh(); } catch (error) { notice = String(error); busy = false; render(); }
}

render();
if (isTauri()) refresh();
