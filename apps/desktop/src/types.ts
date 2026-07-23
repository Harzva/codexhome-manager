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

export type ObservabilityTotals = {
  tasks: number;
  runs: number;
  attempts: number;
  threads: number;
  toolCalls: number;
  artifacts: number;
  verifications: number;
  completedAttempts: number;
  failedAttempts: number;
  failureRateBasisPoints: number;
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  cachedInputTokens: number;
  cacheHits: number;
  cacheMisses: number;
  cacheHitRateBasisPoints: number;
  durationMs: number;
  estimatedCostMicrousd: number;
  retries: number;
};

export type ObservabilityGroup = {
  key: string;
  totals: ObservabilityTotals;
};

export type LatestHomeHealth = {
  homeId: string;
  timestampMs: number;
  status: "healthy" | "unhealthy" | "degraded" | string;
  snapshot: {
    serviceReachable: boolean;
    authValid: boolean | null;
    quotaRemainingBasisPoints: number | null;
    rateLimitResetAtMs: number | null;
    detailCode: string | null;
  };
};

export type ObservabilitySummary = {
  ok: boolean;
  schemaVersion: string;
  storePath: string;
  asOfTimestampMs: number | null;
  eventCount: number;
  totals: ObservabilityTotals;
  byHome: ObservabilityGroup[];
  byAccount: ObservabilityGroup[];
  byModel: ObservabilityGroup[];
  byThread: ObservabilityGroup[];
  latestHomeHealth: LatestHomeHealth[];
};
