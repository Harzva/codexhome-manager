export type HomeSummary = {
  provider: string | null;
  model: string | null;
  skillCount: number;
  mcpServerCount: number;
  ruleCount: number;
  hookCount: number;
};

export type HomeView = {
  id: string;
  alias: string;
  label: string;
  path: string;
  specialties: string[];
  origin: "created" | "imported" | "cloned";
  available: boolean;
  summary: HomeSummary | null;
  issues: string[];
};

export type RegistryReport = {
  ok: boolean;
  schemaVersion: string;
  registryPath: string;
  revision: number;
  homes: HomeView[];
};

export type MutationResult = {
  action: "create" | "import" | "clone";
  dryRun: boolean;
  registryRevision: number;
  entry: HomeView;
  plannedActions: string[];
  warnings: string[];
  copySummary: { filesCopied: number; filesSkipped: number };
};
