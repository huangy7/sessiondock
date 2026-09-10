import { ref, computed, watch, type Ref, type ComputedRef } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { RemoteModelPricing, PricingCatalogItem, PricingCatalogStatus } from "../types/pricing";

export const STORAGE_KEY_CATALOG = "claudia-pricing-catalog-v2";
export const STORAGE_KEY_UPDATED_AT = "claudia-pricing-updated-at-v2";

export const DEFAULT_MODEL_PRICING: RemoteModelPricing = {
  input_cost_per_token: 3 / 1_000_000,
  output_cost_per_token: 15 / 1_000_000,
  cache_creation_input_token_cost: 3.75 / 1_000_000,
  cache_read_input_token_cost: 0.3 / 1_000_000,
};

function createPricing(
  input: number,
  output: number,
  cacheWrite: number = input * 1.25,
  cacheRead: number = input * 0.1
): RemoteModelPricing {
  return {
    input_cost_per_token: input / 1_000_000,
    output_cost_per_token: output / 1_000_000,
    cache_creation_input_token_cost: cacheWrite / 1_000_000,
    cache_read_input_token_cost: cacheRead / 1_000_000,
  };
}

export const DEFAULT_CATALOG: Record<string, RemoteModelPricing> = {
  // Claude 4.6
  "anthropic/claude-opus-4-6": createPricing(5, 25, 6.25, 0.5),
  "claude-opus-4-6": createPricing(5, 25, 6.25, 0.5),
  "anthropic/claude-sonnet-4-6": createPricing(3, 15, 3.75, 0.3),
  "claude-sonnet-4-6": createPricing(3, 15, 3.75, 0.3),

  // Claude 4.5
  "anthropic/claude-opus-4-5": createPricing(5, 25, 6.25, 0.5),
  "claude-opus-4-5": createPricing(5, 25, 6.25, 0.5),
  "anthropic/claude-sonnet-4-5": createPricing(3, 15, 3.75, 0.3),
  "claude-sonnet-4-5": createPricing(3, 15, 3.75, 0.3),
  "anthropic/claude-haiku-4-5": createPricing(1, 5, 1.25, 0.1),
  "claude-haiku-4-5": createPricing(1, 5, 1.25, 0.1),

  // Claude 4 (Legacy / Date suffixes)
  "anthropic/claude-sonnet-4": createPricing(3, 15, 3.75, 0.3),
  "claude-sonnet-4": createPricing(3, 15, 3.75, 0.3),
  "anthropic/claude-sonnet-4-20250514": createPricing(3, 15, 3.75, 0.3),
  "claude-sonnet-4-20250514": createPricing(3, 15, 3.75, 0.3),
  "anthropic/claude-opus-4-20250514": createPricing(15, 75, 18.75, 1.5),
  "claude-opus-4-20250514": createPricing(15, 75, 18.75, 1.5),
  "anthropic/claude-haiku-4-20250414": createPricing(0.8, 4, 1, 0.08),
  "claude-haiku-4-20250414": createPricing(0.8, 4, 1, 0.08),

  // Claude 3.5 & 3
  "anthropic/claude-3-5-sonnet": createPricing(3, 15, 3.75, 0.3),
  "claude-3-5-sonnet": createPricing(3, 15, 3.75, 0.3),
  "anthropic/claude-3-5-haiku": createPricing(0.8, 4, 1, 0.08),
  "claude-3-5-haiku": createPricing(0.8, 4, 1, 0.08),
  "anthropic/claude-3-opus": createPricing(15, 75, 18.75, 1.5),
  "claude-3-opus": createPricing(15, 75, 18.75, 1.5),
  "anthropic/claude-3-haiku": createPricing(0.25, 1.25, 0.3, 0.03),
  "claude-3-haiku": createPricing(0.25, 1.25, 0.3, 0.03),

  // Google Gemini
  "google/gemini-2.5-pro": createPricing(1.25, 5, 1.25, 0.3125),
  "gemini-2.5-pro": createPricing(1.25, 5, 1.25, 0.3125),
  "google/gemini-2.5-flash": createPricing(0.15, 0.6, 0.15, 0.0375),
  "gemini-2.5-flash": createPricing(0.15, 0.6, 0.15, 0.0375),
  "google/gemini-2.0-flash": createPricing(0.1, 0.4, 0.1, 0.025),
  "gemini-2.0-flash": createPricing(0.1, 0.4, 0.1, 0.025),

  // OpenAI
  "openai/gpt-4o": createPricing(2.5, 10, 2.5, 1.25),
  "gpt-4o": createPricing(2.5, 10, 2.5, 1.25),
  "openai/gpt-4o-mini": createPricing(0.15, 0.6, 0.15, 0.075),
  "gpt-4o-mini": createPricing(0.15, 0.6, 0.15, 0.075),

  // DeepSeek
  "deepseek/deepseek-chat": createPricing(0.27, 1.1, 0.27, 0.07),
  "deepseek-chat": createPricing(0.27, 1.1, 0.27, 0.07),
  "deepseek/deepseek-v3": createPricing(0.27, 1.1, 0.27, 0.07),
  "deepseek-v3": createPricing(0.27, 1.1, 0.27, 0.07),
  "deepseek/deepseek-reasoner": createPricing(0.55, 2.19, 0.55, 0.14),
  "deepseek-reasoner": createPricing(0.55, 2.19, 0.55, 0.14),

  // Moonshot / Kimi
  "moonshotai/kimi-k3": createPricing(3, 15, 3.75, 0.3),
  "kimi-k3": createPricing(3, 15, 3.75, 0.3),
  "moonshotai/kimi-k2.7-code": createPricing(0.95, 4, 1, 0.19),
  "kimi-k2.7-code": createPricing(0.95, 4, 1, 0.19),
  "moonshotai/kimi-k2.5": createPricing(0.6, 2.5, 0.6, 0.15),
  "kimi-k2.5": createPricing(0.6, 2.5, 0.6, 0.15),
  "moonshotai/kimi-k2-thinking": createPricing(0.6, 2.5, 0.6, 0.15),
  "kimi-k2-thinking": createPricing(0.6, 2.5, 0.6, 0.15),
  "moonshotai/moonshot-v1-8k": createPricing(1.68, 1.68, 1.68, 0.84),
  "moonshot-v1-8k": createPricing(1.68, 1.68, 1.68, 0.84),
  "moonshotai/moonshot-v1-32k": createPricing(3.36, 3.36, 3.36, 1.68),
  "moonshot-v1-32k": createPricing(3.36, 3.36, 3.36, 1.68),
  "moonshotai/moonshot-v1-128k": createPricing(8.4, 8.4, 8.4, 4.2),
  "moonshot-v1-128k": createPricing(8.4, 8.4, 8.4, 4.2),

  // Alibaba / 通义千问
  "alibaba/qwen3.7-max": createPricing(1.6, 6.4, 1.6, 0.4),
  "qwen3.7-max": createPricing(1.6, 6.4, 1.6, 0.4),
  "alibaba/qwen-max": createPricing(1.6, 6.4, 1.6, 0.4),
  "qwen-max": createPricing(1.6, 6.4, 1.6, 0.4),
  "alibaba/qwen-plus": createPricing(0.4, 1.2, 0.4, 0.1),
  "qwen-plus": createPricing(0.4, 1.2, 0.4, 0.1),
  "alibaba/qwen-turbo": createPricing(0.04, 0.08, 0.04, 0.01),
  "qwen-turbo": createPricing(0.04, 0.08, 0.04, 0.01),

  // Zhipu / 智谱 GLM
  "zhipuai/glm-5": createPricing(1, 3.2, 1, 0.2),
  "glm-5": createPricing(1, 3.2, 1, 0.2),
  "zhipuai/glm-4.7": createPricing(0.8, 2.8, 0.8, 0.16),
  "glm-4.7": createPricing(0.8, 2.8, 0.8, 0.16),
  "zhipuai/glm-4.6": createPricing(0.6, 2.2, 0.6, 0.11),
  "glm-4.6": createPricing(0.6, 2.2, 0.6, 0.11),
  "zhipuai/glm-4-flash": createPricing(0.01, 0.01, 0.01, 0.005),
  "glm-4-flash": createPricing(0.01, 0.01, 0.01, 0.005),

  // MiniMax
  "minimax/minimax-m2.7": createPricing(0.3, 1.2, 0.375, 0.06),
  "minimax-m2.7": createPricing(0.3, 1.2, 0.375, 0.06),
  "minimax/minimax-m2.5": createPricing(0.2, 0.8, 0.25, 0.04),
  "minimax-m2.5": createPricing(0.2, 0.8, 0.25, 0.04),

  // StepFun / 阶跃星辰
  "stepfun/step-3.5-flash": createPricing(0.1, 0.3, 0.1, 0.02),
  "step-3.5-flash": createPricing(0.1, 0.3, 0.1, 0.02),
};

function loadPersistedCatalog(): Record<string, RemoteModelPricing> | null {
  try {
    const saved = localStorage.getItem(STORAGE_KEY_CATALOG);
    if (saved) {
      const parsed = JSON.parse(saved);
      if (parsed && typeof parsed === "object" && Object.keys(parsed).length > 0) {
        return { ...DEFAULT_CATALOG, ...parsed };
      }
    }
  } catch {
    // Ignore localStorage errors
  }
  return null;
}

function loadPersistedUpdatedAt(): string | null {
  try {
    return localStorage.getItem(STORAGE_KEY_UPDATED_AT);
  } catch {
    return null;
  }
}

// Singleton state
const catalog = ref<Record<string, RemoteModelPricing>>(loadPersistedCatalog() || { ...DEFAULT_CATALOG });
const updatedAt = ref<string | null>(loadPersistedUpdatedAt());
const isRefreshing = ref<boolean>(false);
const refreshError = ref<string>("");

export function normalizeModelKey(model: string): string {
  if (!model) return "";
  return model
    .trim()
    .toLowerCase()
    .replace(/^models\//, "");
}

export function stripVersionAndVariant(model: string): string[] {
  const normalized = normalizeModelKey(model);
  if (!normalized) return [];

  const candidates = new Set<string>();

  // Extract provider & pure model if / exists
  let pureModel = normalized;
  let providerPrefix = "";
  if (normalized.includes("/")) {
    const slashIdx = normalized.indexOf("/");
    providerPrefix = normalized.slice(0, slashIdx + 1);
    pureModel = normalized.slice(slashIdx + 1);
    candidates.add(pureModel);
  }

  // Variations to transform
  const bases = [pureModel];
  if (providerPrefix) {
    bases.push(normalized);
  }

  for (const base of bases) {
    // 1. Strip date: -20250514, -2024-08-06, -0514, etc.
    const withoutDate = base
      .replace(/[-_.]\d{4}-\d{2}-\d{2}$/, "")
      .replace(/[-_.]\d{8}$/, "")
      .replace(/[-_.]\d{4}$/, "");
    if (withoutDate && withoutDate !== base) {
      candidates.add(withoutDate);
      if (withoutDate.includes("/")) {
        candidates.add(withoutDate.slice(withoutDate.indexOf("/") + 1));
      }
    }

    // 2. Strip variant suffixes: -latest, -preview, -fast, -mini, -turbo, -pro, -lite, -build, etc.
    const withoutVariant = base
      .replace(/-(?:latest|preview|fast|turbo|mini|pro|lite|build|chat|reasoner)(?:-\d+)?$/g, "");
    if (withoutVariant && withoutVariant !== base) {
      candidates.add(withoutVariant);
      if (withoutVariant.includes("/")) {
        candidates.add(withoutVariant.slice(withoutVariant.indexOf("/") + 1));
      }
    }

    // 3. Combined date + variant strip
    const withoutBoth = withoutDate
      .replace(/-(?:latest|preview|fast|turbo|mini|pro|lite|build|chat|reasoner)(?:-\d+)?$/g, "");
    if (withoutBoth && withoutBoth !== base) {
      candidates.add(withoutBoth);
      if (withoutBoth.includes("/")) {
        candidates.add(withoutBoth.slice(withoutBoth.indexOf("/") + 1));
      }
    }

    // 4. Dot vs Dash version replacement: claude-3.5-sonnet <-> claude-3-5-sonnet
    if (/\d+\.\d+/.test(base)) {
      const dashed = base.replace(/(\d+)\.(\d+)/g, "$1-$2");
      candidates.add(dashed);
      if (dashed.includes("/")) {
        candidates.add(dashed.slice(dashed.indexOf("/") + 1));
      }
    }
    if (/\d+-\d+/.test(base)) {
      const dotted = base.replace(/(\d+)-(\d+)/g, "$1.$2");
      candidates.add(dotted);
      if (dotted.includes("/")) {
        candidates.add(dotted.slice(dotted.indexOf("/") + 1));
      }
    }
  }

  candidates.delete(normalized);
  return Array.from(candidates);
}

const pricingCache = new Map<string, RemoteModelPricing>();
watch(catalog, () => pricingCache.clear(), { deep: true });

export function getModelPricing(model: string): RemoteModelPricing {
  if (!model) return DEFAULT_MODEL_PRICING;
  const key = normalizeModelKey(model);
  if (!key) return DEFAULT_MODEL_PRICING;

  const cached = pricingCache.get(key);
  if (cached) return cached;

  let result: RemoteModelPricing | undefined;

  // 1. Exact match in catalog
  if (catalog.value[key]) {
    result = catalog.value[key];
  }

  // 2. Candidate match
  if (!result) {
    const candidates = stripVersionAndVariant(key);
    for (const candidate of candidates) {
      if (catalog.value[candidate]) {
        result = catalog.value[candidate];
        break;
      }
    }
  }

  // 3. Keyword heuristic matching (O(1))
  if (!result) {
    if (key.includes("opus")) result = catalog.value["claude-opus-4-6"] || DEFAULT_CATALOG["claude-opus-4-6"];
    else if (key.includes("haiku")) result = catalog.value["claude-3-5-haiku"] || DEFAULT_CATALOG["claude-3-5-haiku"];
    else if (key.includes("flash")) result = catalog.value["gemini-2.5-flash"] || DEFAULT_CATALOG["gemini-2.5-flash"];
    else if (key.includes("sonnet")) result = catalog.value["claude-sonnet-4-6"] || DEFAULT_CATALOG["claude-sonnet-4-6"];
    else if (key.includes("gpt-4o-mini")) result = catalog.value["gpt-4o-mini"] || DEFAULT_CATALOG["gpt-4o-mini"];
    else if (key.includes("gpt-4") || key.includes("gpt-4o")) result = catalog.value["gpt-4o"] || DEFAULT_CATALOG["gpt-4o"];
    else if (key.includes("deepseek")) result = catalog.value["deepseek-chat"] || DEFAULT_CATALOG["deepseek-chat"];
    else if (key.includes("kimi") || key.includes("moonshot")) result = catalog.value["kimi-k3"] || DEFAULT_CATALOG["kimi-k3"];
    else if (key.includes("qwen")) result = catalog.value["qwen-max"] || DEFAULT_CATALOG["qwen-max"];
    else if (key.includes("glm")) result = catalog.value["glm-5"] || DEFAULT_CATALOG["glm-5"];
    else if (key.includes("minimax") || key.includes("abab")) result = catalog.value["minimax-m2.7"] || DEFAULT_CATALOG["minimax-m2.7"];
    else if (key.includes("step")) result = catalog.value["step-3.5-flash"] || DEFAULT_CATALOG["step-3.5-flash"];
    else if (key.includes("grok")) result = catalog.value["grok-4.5"] || DEFAULT_CATALOG["grok-4.5"];
  }

  const finalPricing = result || DEFAULT_MODEL_PRICING;
  pricingCache.set(key, finalPricing);
  return finalPricing;
}

export interface PricingCatalogResponse {
  catalog: Record<string, RemoteModelPricing>;
  updated_at: string | null;
  model_count: number;
}

export async function fetchPricingCatalog(force = false): Promise<void> {
  if (isRefreshing.value) return;
  if (!force && updatedAt.value && Object.keys(catalog.value).length > 0) return;

  isRefreshing.value = true;
  refreshError.value = "";

  try {
    // 1. Try native Rust backend first (fast reqwest, system proxy, gzip decompression, SQLite caching)
    try {
      const res = await invoke<PricingCatalogResponse>("refresh_pricing_catalog");
      if (res && res.catalog && Object.keys(res.catalog).length > 0) {
        const nextCatalog = { ...DEFAULT_CATALOG, ...res.catalog };
        const now = res.updated_at || new Date().toISOString();
        pricingCache.clear();
        catalog.value = nextCatalog;
        updatedAt.value = now;
        try {
          localStorage.setItem(STORAGE_KEY_CATALOG, JSON.stringify(nextCatalog));
          localStorage.setItem(STORAGE_KEY_UPDATED_AT, now);
        } catch {
          // Ignore localStorage errors
        }
        return;
      }
    } catch {
      // Fallback to web fetch below
    }

    // 2. Web / Vitest fallback
    const res = await fetch("https://models.dev/api.json");
    if (!res.ok) {
      throw new Error(`HTTP ${res.status}: ${res.statusText}`);
    }
    const data = await res.json();
    const nextCatalog: Record<string, RemoteModelPricing> = { ...DEFAULT_CATALOG };

    if (data && typeof data === "object") {
      if (Array.isArray(data)) {
        for (const item of data) {
          if (item && typeof item === "object") {
            const provider = String(item.provider || item.providerId || "custom").toLowerCase();
            const model = String(item.id || item.model || item.name || "").toLowerCase();
            if (model) {
              const cost = item.cost || item.pricing || {};
              const pricing: RemoteModelPricing = {
                input_cost_per_token: cost.input != null ? Number(cost.input) / 1_000_000 : undefined,
                output_cost_per_token: cost.output != null ? Number(cost.output) / 1_000_000 : undefined,
                cache_read_input_token_cost: cost.cache_read != null ? Number(cost.cache_read) / 1_000_000 : undefined,
                cache_creation_input_token_cost:
                  cost.cache_write != null
                    ? Number(cost.cache_write) / 1_000_000
                    : cost.cache_creation != null
                    ? Number(cost.cache_creation) / 1_000_000
                    : undefined,
              };
              nextCatalog[`${provider}/${model}`] = pricing;
              nextCatalog[model] = pricing;
            }
          }
        }
      } else {
        const PRIMARY_KEYS = [
          "anthropic",
          "openai",
          "google",
          "deepseek",
          "moonshotai",
          "moonshotai-cn",
          "kimi-for-coding",
          "zhipuai",
          "zai",
          "alibaba",
          "alibaba-cn",
          "minimax",
          "minimax-cn",
          "stepfun",
          "stepfun-ai",
          "xai",
          "meta",
          "mistral",
          "baichuan",
          "volcengine",
        ];
        for (const [providerKey, providerObj] of Object.entries(data as Record<string, any>)) {
          if (!providerObj || typeof providerObj !== "object") continue;
          const pKey = providerKey.toLowerCase();
          if (!PRIMARY_KEYS.some((k) => pKey === k || pKey.startsWith(k))) continue;
          const models = providerObj.models || (providerObj.cost ? { [providerKey]: providerObj } : {});
          for (const [modelKey, modelObj] of Object.entries(models as Record<string, any>)) {
            if (!modelObj || typeof modelObj !== "object") continue;
            const mKey = (modelObj.id || modelKey).toLowerCase();
            const cost = modelObj.cost || modelObj.pricing || {};
            const pricing: RemoteModelPricing = {
              input_cost_per_token: cost.input != null ? Number(cost.input) / 1_000_000 : undefined,
              output_cost_per_token: cost.output != null ? Number(cost.output) / 1_000_000 : undefined,
              cache_read_input_token_cost: cost.cache_read != null ? Number(cost.cache_read) / 1_000_000 : undefined,
              cache_creation_input_token_cost:
                cost.cache_write != null
                  ? Number(cost.cache_write) / 1_000_000
                  : cost.cache_creation != null
                  ? Number(cost.cache_creation) / 1_000_000
                  : undefined,
            };
            nextCatalog[`${pKey}/${mKey}`] = pricing;
            nextCatalog[mKey] = pricing;
          }
        }
      }
    }

    const now = new Date().toISOString();
    pricingCache.clear();
    catalog.value = nextCatalog;
    updatedAt.value = now;

    try {
      localStorage.setItem(STORAGE_KEY_CATALOG, JSON.stringify(nextCatalog));
      localStorage.setItem(STORAGE_KEY_UPDATED_AT, now);
    } catch {
      // Ignore localStorage errors
    }
  } catch (err: unknown) {
    refreshError.value = err instanceof Error ? err.message : String(err);
  } finally {
    isRefreshing.value = false;
  }
}

function formatProviderName(provider: string): string {
  const p = provider.toLowerCase();
  if (p.includes("anthropic") || p.includes("claude")) return "Anthropic";
  if (p.includes("openai") || p.includes("chatgpt") || p.includes("codex")) return "OpenAI";
  if (p.includes("google") || p.includes("gemini") || p.includes("vertex")) return "Google";
  if (p.includes("deepseek")) return "DeepSeek";
  if (p.includes("moonshot") || p.includes("kimi")) return "Moonshot / Kimi";
  if (p.includes("zhipu") || p.includes("zai") || p.includes("glm")) return "Zhipu / 智谱 GLM";
  if (p.includes("alibaba") || p.includes("qwen") || p.includes("dashscope") || p.includes("aliyun")) return "Alibaba / 通义千问";
  if (p.includes("minimax")) return "MiniMax";
  if (p.includes("stepfun") || p.includes("step")) return "StepFun / 阶跃星辰";
  if (p.includes("baichuan")) return "Baichuan / 百川";
  if (p.includes("volcengine") || p.includes("bytedance") || p.includes("doubao")) return "ByteDance / 豆包";
  if (p.includes("tencent") || p.includes("hunyuan")) return "Tencent / 腾讯混元";
  if (p.includes("baidu") || p.includes("qianfan") || p.includes("ernie")) return "Baidu / 百度千帆";
  if (p.includes("siliconflow") || p.includes("silicon")) return "SiliconFlow";
  if (p.includes("meta") || p.includes("llama")) return "Meta";
  if (p.includes("mistral")) return "Mistral";
  if (p.includes("groq")) return "Groq";
  if (p.includes("xai") || p.includes("grok")) return "xAI";
  return provider.charAt(0).toUpperCase() + provider.slice(1);
}

const catalogItems = computed<PricingCatalogItem[]>(() => {
  const items: PricingCatalogItem[] = [];
  const seen = new Set<string>();

  // Pass 1: Keys with provider/model
  for (const [key, pricing] of Object.entries(catalog.value)) {
    if (key.includes("/")) {
      const slashIdx = key.indexOf("/");
      const providerRaw = key.slice(0, slashIdx);
      const model = key.slice(slashIdx + 1);
      const provider = formatProviderName(providerRaw);
      const uniqueKey = `${provider}::${model.toLowerCase()}`;
      if (seen.has(uniqueKey)) continue;
      seen.add(uniqueKey);

      const inputPer1M = Number(((pricing.input_cost_per_token || 0) * 1_000_000).toFixed(4));
      const outputPer1M = Number(((pricing.output_cost_per_token || 0) * 1_000_000).toFixed(4));
      const cacheWritePer1M = Number(((pricing.cache_creation_input_token_cost || 0) * 1_000_000).toFixed(4));
      const cacheReadPer1M = Number(((pricing.cache_read_input_token_cost || 0) * 1_000_000).toFixed(4));

      items.push({
        key,
        model,
        provider,
        inputPer1M,
        outputPer1M,
        cacheWritePer1M,
        cacheReadPer1M,
      });
    }
  }

  // Pass 2: Keys without slash that are not in seen
  for (const [key, pricing] of Object.entries(catalog.value)) {
    if (!key.includes("/")) {
      let inferredProvider = "Other";
      const lk = key.toLowerCase();
      if (lk.startsWith("claude")) inferredProvider = "Anthropic";
      else if (lk.startsWith("gpt") || lk.startsWith("o1") || lk.startsWith("o3")) inferredProvider = "OpenAI";
      else if (lk.startsWith("gemini")) inferredProvider = "Google";
      else if (lk.startsWith("deepseek")) inferredProvider = "DeepSeek";
      else if (lk.startsWith("kimi") || lk.startsWith("moonshot")) inferredProvider = "Moonshot / Kimi";
      else if (lk.startsWith("glm")) inferredProvider = "Zhipu / 智谱 GLM";
      else if (lk.startsWith("qwen")) inferredProvider = "Alibaba / 通义千问";
      else if (lk.startsWith("minimax") || lk.startsWith("abab")) inferredProvider = "MiniMax";
      else if (lk.startsWith("step")) inferredProvider = "StepFun / 阶跃星辰";

      const uniqueKey = `${inferredProvider}::${key.toLowerCase()}`;
      if (seen.has(uniqueKey)) continue;
      seen.add(uniqueKey);

      if (inferredProvider === "Other" && !DEFAULT_CATALOG[key]) continue;

      const inputPer1M = Number(((pricing.input_cost_per_token || 0) * 1_000_000).toFixed(4));
      const outputPer1M = Number(((pricing.output_cost_per_token || 0) * 1_000_000).toFixed(4));
      const cacheWritePer1M = Number(((pricing.cache_creation_input_token_cost || 0) * 1_000_000).toFixed(4));
      const cacheReadPer1M = Number(((pricing.cache_read_input_token_cost || 0) * 1_000_000).toFixed(4));

      items.push({
        key,
        model: key,
        provider: inferredProvider,
        inputPer1M,
        outputPer1M,
        cacheWritePer1M,
        cacheReadPer1M,
      });
    }
  }

  return items.sort((a, b) => a.provider.localeCompare(b.provider) || a.model.localeCompare(b.model));
});

const status = computed<PricingCatalogStatus>(() => ({
  model_count: catalogItems.value.length,
  updated_at: updatedAt.value,
}));

export function resetPricingCatalogStore(): void {
  pricingCache.clear();
  catalog.value = loadPersistedCatalog() || { ...DEFAULT_CATALOG };
  updatedAt.value = loadPersistedUpdatedAt();
  isRefreshing.value = false;
  refreshError.value = "";
}

export function usePricingCatalog() {
  return {
    catalog: catalog as Ref<Record<string, RemoteModelPricing>>,
    catalogItems: catalogItems as ComputedRef<PricingCatalogItem[]>,
    status: status as ComputedRef<PricingCatalogStatus>,
    updatedAt: updatedAt as Ref<string | null>,
    isRefreshing: isRefreshing as Ref<boolean>,
    refreshError: refreshError as Ref<string>,
    fetchPricingCatalog,
    getModelPricing,
  };
}
