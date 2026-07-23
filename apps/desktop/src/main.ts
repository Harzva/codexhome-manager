import { invoke } from "@tauri-apps/api/core";
import "./styles.css";
import type {
  HomeView,
  MutationResult,
  ObservabilityGroup,
  ObservabilitySummary,
  RegistryReport,
} from "./types";

type ViewMode = "homes" | "observability" | "create" | "import" | "clone";

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

const emptyTotals = {
  tasks: 0, runs: 0, attempts: 0, threads: 0, toolCalls: 0, artifacts: 0, verifications: 0,
  completedAttempts: 0, failedAttempts: 0, failureRateBasisPoints: 0, inputTokens: 0,
  outputTokens: 0, totalTokens: 0, cachedInputTokens: 0, cacheHits: 0, cacheMisses: 0,
  cacheHitRateBasisPoints: 0, durationMs: 0, estimatedCostMicrousd: 0, retries: 0,
};

const mockObservability: ObservabilitySummary = {
  ok: true,
  schemaVersion: "codexhome.observability-summary.v1",
  storePath: "~/.codexhome/observability/events.jsonl",
  asOfTimestampMs: Date.now(),
  eventCount: 148,
  totals: {
    ...emptyTotals, tasks: 12, runs: 15, attempts: 18, threads: 14, toolCalls: 46,
    artifacts: 9, verifications: 17, completedAttempts: 16, failedAttempts: 2,
    failureRateBasisPoints: 1111, inputTokens: 184200, outputTokens: 27100,
    totalTokens: 211300, cachedInputTokens: 121000, cacheHits: 31, cacheMisses: 8,
    cacheHitRateBasisPoints: 7948, durationMs: 1842000, estimatedCostMicrousd: 186000,
    retries: 3,
  },
  byHome: [
    { key: "8e771a9d23ac", totals: { ...emptyTotals, runs: 7, attempts: 8, totalTokens: 112000, inputTokens: 98000, outputTokens: 14000, durationMs: 820000, completedAttempts: 8 } },
    { key: "ac1801e7ff41", totals: { ...emptyTotals, runs: 6, attempts: 7, totalTokens: 78300, inputTokens: 67200, outputTokens: 11100, durationMs: 760000, completedAttempts: 6, failedAttempts: 1, failureRateBasisPoints: 1428 } },
    { key: "911b42aa03e1", totals: { ...emptyTotals, runs: 2, attempts: 3, totalTokens: 21000, inputTokens: 19000, outputTokens: 2000, durationMs: 262000, completedAttempts: 2, failedAttempts: 1, failureRateBasisPoints: 3333 } },
  ],
  byAccount: [],
  byModel: [
    { key: "gpt-5.4", totals: { ...emptyTotals, runs: 12, attempts: 14, totalTokens: 174000, inputTokens: 151000, outputTokens: 23000, durationMs: 1430000 } },
    { key: "gpt-5.2", totals: { ...emptyTotals, runs: 3, attempts: 4, totalTokens: 37300, inputTokens: 33200, outputTokens: 4100, durationMs: 412000 } },
  ],
  byThread: [
    { key: "thread-research-07", totals: { ...emptyTotals, runs: 1, attempts: 2, totalTokens: 38200, inputTokens: 34000, outputTokens: 4200, durationMs: 284000 } },
  ],
  latestHomeHealth: [
    { homeId: "8e771a9d23ac", timestampMs: Date.now(), status: "healthy", snapshot: { serviceReachable: true, authValid: true, quotaRemainingBasisPoints: 6800, rateLimitResetAtMs: null, detailCode: null } },
    { homeId: "ac1801e7ff41", timestampMs: Date.now(), status: "degraded", snapshot: { serviceReachable: true, authValid: true, quotaRemainingBasisPoints: 2100, rateLimitResetAtMs: null, detailCode: "quota_low" } },
  ],
};

let report = mockReport;
let observability = mockObservability;
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
    observability = await call<ObservabilitySummary>("observability_summary");
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
    homes: "⌂", observability: "▥", create: "+", import: "↙", clone: "⧉", refresh: "↻", search: "⌕",
  };
  return `<span class="icon" aria-hidden="true">${icons[name]}</span>`;
}

function compactNumber(value: number) {
  return new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(value);
}

function duration(value: number) {
  if (value < 1000) return `${value} ms`;
  const seconds = Math.round(value / 1000);
  if (seconds < 60) return `${seconds} s`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes}m ${seconds % 60}s`;
}

function percent(basisPoints: number) {
  return `${(basisPoints / 100).toFixed(1)}%`;
}

function homeName(id: string) {
  const home = report.homes.find((candidate) => candidate.id === id);
  return home ? `${home.alias} · ${home.label}` : id;
}

function groupRows(groups: ObservabilityGroup[], resolveKey: (key: string) => string = (key) => key) {
  const maxTokens = Math.max(...groups.map((group) => group.totals.totalTokens), 1);
  if (!groups.length) return `<div class="empty-row">No recorded groups</div>`;
  return groups.map((group) => `<div class="breakdown-row">
    <div class="breakdown-key" title="${safe(resolveKey(group.key))}">${safe(resolveKey(group.key))}</div>
    <div class="usage-bar"><span style="width:${Math.max((group.totals.totalTokens / maxTokens) * 100, 2)}%"></span></div>
    <strong>${compactNumber(group.totals.totalTokens)}</strong>
    <span>${group.totals.attempts} attempts</span>
    <span>${duration(group.totals.durationMs)}</span>
  </div>`).join("");
}

function observabilityView() {
  const totals = observability.totals;
  return `<section class="observability-view">
    <section class="metric-strip">
      <div><span>Total tokens</span><strong>${compactNumber(totals.totalTokens)}</strong><small>${compactNumber(totals.cachedInputTokens)} cached input</small></div>
      <div><span>Duration</span><strong>${duration(totals.durationMs)}</strong><small>${totals.runs} runs · ${totals.attempts} attempts</small></div>
      <div><span>Cache hit rate</span><strong>${percent(totals.cacheHitRateBasisPoints)}</strong><small>${totals.cacheHits} hits · ${totals.cacheMisses} misses</small></div>
      <div><span>Failure rate</span><strong>${percent(totals.failureRateBasisPoints)}</strong><small>${totals.failedAttempts} failed · ${totals.retries} retries</small></div>
      <div><span>Estimated cost</span><strong>$${(totals.estimatedCostMicrousd / 1_000_000).toFixed(3)}</strong><small>${totals.toolCalls} tool calls</small></div>
    </section>
    <div class="analysis-grid">
      <section class="analysis-panel span-two"><div class="panel-heading"><div><span class="eyebrow">Home comparison</span><h2>Usage by specialist family</h2></div><span>${observability.eventCount} events</span></div>
        <div class="breakdown-head"><span>Home</span><span>Token share</span><span>Tokens</span><span>Attempts</span><span>Duration</span></div>
        ${groupRows(observability.byHome, homeName)}
      </section>
      <section class="analysis-panel"><div class="panel-heading"><div><span class="eyebrow">Models</span><h2>Model usage</h2></div></div>${groupRows(observability.byModel)}</section>
      <section class="analysis-panel health-panel"><div class="panel-heading"><div><span class="eyebrow">Runtime</span><h2>Home health</h2></div></div>
        ${observability.latestHomeHealth.length ? observability.latestHomeHealth.map((health) => `<div class="health-row">
          <span class="health-dot ${safe(health.status)}"></span>
          <div><strong>${safe(homeName(health.homeId))}</strong><small>${safe(health.snapshot.detailCode ?? health.status)}</small></div>
          <span>${health.snapshot.quotaRemainingBasisPoints === null ? "Quota —" : `${percent(health.snapshot.quotaRemainingBasisPoints)} quota`}</span>
        </div>`).join("") : `<div class="empty-row">No health snapshots</div>`}
      </section>
      <section class="analysis-panel span-two"><div class="panel-heading"><div><span class="eyebrow">Threads</span><h2>Highest-cost task threads</h2></div><span>${totals.threads} linked</span></div>${groupRows(observability.byThread.slice(0, 8))}</section>
    </div>
    <p class="store-path" title="${safe(observability.storePath)}">Store · ${safe(observability.storePath)}</p>
  </section>`;
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
      ${(["homes", "observability", "create", "import", "clone"] as ViewMode[]).map((item) => `<button class="nav-item ${mode === item ? "active" : ""}" data-nav="${item}">${icon(item)}<span>${item[0].toUpperCase() + item.slice(1)}</span></button>`).join("")}
    </nav><div class="privacy"><span class="privacy-dot"></span><div><strong>Local only</strong><span>Credentials never enter the registry</span></div></div></aside>
    <main><header><div><span class="eyebrow">${mode === "observability" ? "Operations" : "Agent households"}</span><h1>${mode === "homes" ? "Your specialist families" : mode === "observability" ? "Run observability" : "Home lifecycle"}</h1></div><div class="header-actions"><span class="revision">${mode === "observability" ? `Events ${observability.eventCount}` : `Registry r${report.revision}`}</span><button class="icon-button" id="refresh" aria-label="Refresh">${icon("refresh")}</button></div></header>
      ${notice ? `<div class="notice">${safe(notice)}</div>` : ""}
      ${mode === "homes" ? `<section class="summary"><div><span>Registered</span><strong>${report.homes.length}</strong></div><div><span>Available</span><strong>${available}</strong></div><div><span>Total skills</span><strong>${skills}</strong></div><div class="registry-location"><span>Registry</span><strong title="${safe(report.registryPath)}">${safe(report.registryPath)}</strong></div></section>
      <div class="toolbar"><label class="search">${icon("search")}<input id="search" value="${safe(query)}" placeholder="Search alias, family, specialty…" /></label><button class="button primary" data-nav="create">${icon("create")} New Home</button></div>
      <section class="home-grid">${filteredHomes().map(homeCard).join("") || `<div class="empty"><strong>No matching families</strong><span>Try another alias or specialty.</span></div>`}</section>` : mode === "observability" ? observabilityView() : formView(mode)}
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
