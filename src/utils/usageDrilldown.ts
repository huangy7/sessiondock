import type { UsageRecord } from "../types/session";
import { getModelPricing } from "../composables/usePricingCatalog";

export { getModelPricing } from "../composables/usePricingCatalog";

export interface ModelProjectItem {
  project: string;
  projectName: string;
  tokens: number;
  cost: number;
  percentage: number;
}

export interface ModelBreakdownItem {
  model: string;
  tokens: number;
  cost: number;
  projects: ModelProjectItem[];
}

export interface ProjectModelItem {
  model: string;
  tokens: number;
  cost: number;
  percentage: number;
}

export interface ProjectBreakdownItem {
  project: string;
  projectName: string;
  tokens: number;
  cost: number;
  models: ProjectModelItem[];
}

export interface PricingTier {
  input: number;
  output: number;
  cacheWrite: number;
  cacheRead: number;
}

export const MODEL_PRICING: Record<string, PricingTier> = {
  // Claude 4.6
  "claude-opus-4-6": { input: 5, output: 25, cacheWrite: 6.25, cacheRead: 0.5 },
  "claude-sonnet-4-6": { input: 3, output: 15, cacheWrite: 3.75, cacheRead: 0.3 },
  // Claude 4.5
  "claude-opus-4-5": { input: 5, output: 25, cacheWrite: 6.25, cacheRead: 0.5 },
  "claude-sonnet-4-5": { input: 3, output: 15, cacheWrite: 3.75, cacheRead: 0.3 },
  // Claude 4
  "claude-sonnet-4-20250514": { input: 3, output: 15, cacheWrite: 3.75, cacheRead: 0.3 },
  "claude-opus-4-20250514": { input: 15, output: 75, cacheWrite: 18.75, cacheRead: 1.5 },
  // Claude 3.5
  "claude-3-5-sonnet": { input: 3, output: 15, cacheWrite: 3.75, cacheRead: 0.3 },
  "claude-3-5-haiku": { input: 0.8, output: 4, cacheWrite: 1, cacheRead: 0.08 },
  // Claude 3
  "claude-3-opus": { input: 15, output: 75, cacheWrite: 18.75, cacheRead: 1.5 },
  "claude-3-haiku": { input: 0.25, output: 1.25, cacheWrite: 0.3, cacheRead: 0.03 },
  // Haiku 4.5
  "claude-haiku-4-5": { input: 1, output: 5, cacheWrite: 1.25, cacheRead: 0.1 },
  "claude-haiku-4-20250414": { input: 0.8, output: 4, cacheWrite: 1, cacheRead: 0.08 },
  // Gemini
  "gemini-2.5-pro": { input: 1.25, output: 5, cacheWrite: 1.25, cacheRead: 0.3125 },
  "gemini-2.5-flash": { input: 0.15, output: 0.6, cacheWrite: 0.15, cacheRead: 0.0375 },
  "gemini-2.0-flash": { input: 0.1, output: 0.4, cacheWrite: 0.1, cacheRead: 0.025 },
};

export function getRecordCost(record: UsageRecord): number {
  const pricing = getModelPricing(record.model);
  const inputCost = (record.input_tokens || 0) * (pricing.input_cost_per_token ?? 0.000003);
  const outputCost = (record.output_tokens || 0) * (pricing.output_cost_per_token ?? 0.000015);
  const cacheWriteCost = (record.cache_creation_tokens || 0) * (pricing.cache_creation_input_token_cost ?? 0.00000375);
  const cacheReadCost = (record.cache_read_tokens || 0) * (pricing.cache_read_input_token_cost ?? 0.0000003);
  return inputCost + outputCost + cacheWriteCost + cacheReadCost;
}

export function getProjectName(path: string): string {
  if (!path || path === "default") return "全局默认";
  const trimmed = path.replace(/[\\/]+$/, "");
  if (!trimmed) return "全局默认";
  const parts = trimmed.split(/[\\/]/);
  const name = parts[parts.length - 1];
  return name || "全局默认";
}

export function calculateModelBreakdown(records: UsageRecord[]): ModelBreakdownItem[] {
  const map = new Map<
    string,
    {
      tokens: number;
      cost: number;
      projectMap: Map<string, { tokens: number; cost: number }>;
    }
  >();

  for (const r of records) {
    const model = r.model || "unknown";
    const totalTokens =
      (r.input_tokens || 0) +
      (r.output_tokens || 0) +
      (r.cache_creation_tokens || 0) +
      (r.cache_read_tokens || 0);
    const cost = getRecordCost(r);
    const project = r.project || "default";

    let mEntry = map.get(model);
    if (!mEntry) {
      mEntry = { tokens: 0, cost: 0, projectMap: new Map() };
      map.set(model, mEntry);
    }
    mEntry.tokens += totalTokens;
    mEntry.cost += cost;

    const pEntry = mEntry.projectMap.get(project) || { tokens: 0, cost: 0 };
    pEntry.tokens += totalTokens;
    pEntry.cost += cost;
    mEntry.projectMap.set(project, pEntry);
  }

  const result: ModelBreakdownItem[] = [];
  for (const [model, mData] of map.entries()) {
    const projects: ModelProjectItem[] = [];
    for (const [proj, pData] of mData.projectMap.entries()) {
      projects.push({
        project: proj,
        projectName: getProjectName(proj),
        tokens: pData.tokens,
        cost: pData.cost,
        percentage: mData.tokens > 0 ? (pData.tokens / mData.tokens) * 100 : 0,
      });
    }
    projects.sort((a, b) => b.tokens - a.tokens || b.cost - a.cost);
    result.push({
      model,
      tokens: mData.tokens,
      cost: mData.cost,
      projects,
    });
  }

  return result.sort((a, b) => b.tokens - a.tokens || b.cost - a.cost);
}

export function calculateProjectBreakdown(records: UsageRecord[]): ProjectBreakdownItem[] {
  const map = new Map<
    string,
    {
      tokens: number;
      cost: number;
      modelMap: Map<string, { tokens: number; cost: number }>;
    }
  >();

  for (const r of records) {
    const project = r.project || "default";
    const totalTokens =
      (r.input_tokens || 0) +
      (r.output_tokens || 0) +
      (r.cache_creation_tokens || 0) +
      (r.cache_read_tokens || 0);
    const cost = getRecordCost(r);
    const model = r.model || "unknown";

    let pEntry = map.get(project);
    if (!pEntry) {
      pEntry = { tokens: 0, cost: 0, modelMap: new Map() };
      map.set(project, pEntry);
    }
    pEntry.tokens += totalTokens;
    pEntry.cost += cost;

    const mEntry = pEntry.modelMap.get(model) || { tokens: 0, cost: 0 };
    mEntry.tokens += totalTokens;
    mEntry.cost += cost;
    pEntry.modelMap.set(model, mEntry);
  }

  const result: ProjectBreakdownItem[] = [];
  for (const [proj, pData] of map.entries()) {
    const models: ProjectModelItem[] = [];
    for (const [model, mData] of pData.modelMap.entries()) {
      models.push({
        model,
        tokens: mData.tokens,
        cost: mData.cost,
        percentage: pData.tokens > 0 ? (mData.tokens / pData.tokens) * 100 : 0,
      });
    }
    models.sort((a, b) => b.cost - a.cost || b.tokens - a.tokens);
    result.push({
      project: proj,
      projectName: getProjectName(proj),
      tokens: pData.tokens,
      cost: pData.cost,
      models,
    });
  }

  return result.sort((a, b) => b.cost - a.cost || b.tokens - a.tokens);
}
