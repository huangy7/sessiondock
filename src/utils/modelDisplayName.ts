import { strip1m } from "./modelSuffix";

/** 网关 /v1/models 返回的模型条目（display / billing_rules 可能缺失） */
export interface ModelEntry {
  id: string;
  display?: string | null;
  billing_rules?: string | null;
}

/** id → display 映射；无 display（或纯空白）的条目不收录 */
export function buildDisplayById(entries: ModelEntry[]): Map<string, string> {
  const map = new Map<string, string>();
  for (const entry of entries) {
    const display = entry.display?.trim();
    if (display) map.set(entry.id, display);
  }
  return map;
}

/**
 * 选中模型时决定「显示名称」是否自动填充。
 * 返回要写入的名称；返回 null 表示不改动（保留用户手填值）。
 * 规则：当前名称为空，或等于上一个模型的 display（说明是自动填的）→ 用新模型的 display 覆盖。
 */
export function resolveAutoDisplayName(opts: {
  currentName: string;
  prevModel: string;
  nextModel: string;
  displayById: Map<string, string>;
}): string | null {
  const nextDisplay = opts.displayById.get(strip1m(opts.nextModel));
  if (!nextDisplay) return null;
  const current = opts.currentName.trim();
  if (!current) return nextDisplay;
  const prevDisplay = opts.displayById.get(strip1m(opts.prevModel));
  if (prevDisplay && current === prevDisplay) return nextDisplay;
  return null;
}

type ModelRole = "haiku" | "sonnet" | "opus";

/** 拉取模型列表后，补齐名称为空且模型 id 有 display 的角色行 */
export function backfillEmptyDisplayNames(
  models: Record<ModelRole, string>,
  names: Record<ModelRole, string>,
  displayById: Map<string, string>,
): Partial<Record<ModelRole, string>> {
  const result: Partial<Record<ModelRole, string>> = {};
  for (const role of ["haiku", "sonnet", "opus"] as const) {
    if (names[role].trim()) continue;
    const display = displayById.get(strip1m(models[role]));
    if (display) result[role] = display;
  }
  return result;
}

/**
 * 解析 localStorage 里的模型列表缓存。
 * 兼容旧格式 { model_ids: string[] }（升级为无 display 的条目）与新格式 { models: ModelEntry[] }。
 */
export function parseCachedModelEntries(raw: string | null): ModelEntry[] {
  if (!raw) return [];
  try {
    const cached = JSON.parse(raw);
    if (Array.isArray(cached?.models)) {
      return cached.models
        .filter((m: unknown) => m && typeof (m as ModelEntry).id === "string")
        .map((m: ModelEntry) => ({
          id: m.id,
          display: m.display ?? undefined,
          billing_rules: m.billing_rules ?? undefined,
        }));
    }
    if (Array.isArray(cached?.model_ids)) {
      return cached.model_ids
        .filter((id: unknown) => typeof id === "string")
        .map((id: string) => ({ id }));
    }
  } catch {
    // fall through
  }
  return [];
}
