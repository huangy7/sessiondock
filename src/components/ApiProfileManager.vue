<script setup lang="ts">
import { ref, computed, nextTick, onMounted, onUnmounted, watch, defineAsyncComponent } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { ask, message, save, open } from "@tauri-apps/plugin-dialog";
import { readText as readClipboardText } from "@tauri-apps/plugin-clipboard-manager";
import SvgIcon from "./icons/SvgIcon.vue";
import ProfileImportConflictDialog, { type ImportCandidate, type ScopeImportItem } from "./api-profile/ProfileImportConflictDialog.vue";
import ApiProfileAttributionSection from "./api-profile/ApiProfileAttributionSection.vue";
import ProfileTabCreateDialog, { type ScopeCreateAction } from "./api-profile/ProfileTabCreateDialog.vue";
import ProfileTabEditDialog, { type TabInfo } from "./api-profile/ProfileTabEditDialog.vue";
import ScopeDeleteConfirmDialog from "./api-profile/ScopeDeleteConfirmDialog.vue";
import ApiProfileConnectionSection from "./api-profile/ApiProfileConnectionSection.vue";
import ApiProfileDeleteConfirm from "./api-profile/ApiProfileDeleteConfirm.vue";
import ApiProfileEditorHeader from "./api-profile/ApiProfileEditorHeader.vue";
import ApiProfileList, { type Profile, type FieldInfo } from "./api-profile/ApiProfileList.vue";
import ApiProfileModelOverridesSection from "./api-profile/ApiProfileModelOverridesSection.vue";
import ApiProfileProxyBanner from "./api-profile/ApiProfileProxyBanner.vue";
import ApiProfileRuntimeSection from "./api-profile/ApiProfileRuntimeSection.vue";
import ApiProfileScopeAllocation, { type ScopeBinding, type ProfileSummary, type ScopeDirStatus } from "./api-profile/ApiProfileScopeAllocation.vue";
import ElegantSelect, { type SelectOption } from "./common/ElegantSelect.vue";
const ApiProfileJsonEditor = defineAsyncComponent(() => import("./api-profile/ApiProfileJsonEditor.vue"));
const GlobalSettingsRawEditor = defineAsyncComponent(() => import("./api-profile/GlobalSettingsRawEditor.vue"));
import { addProxyStatusChangedListener } from "../composables/useProxy";
import { useSessions } from "../composables/useSessions";
import { resolveFeatureCliId } from "../composables/cliFilter";
import { isCliId, type CliId } from "../types/cli";
import {
  backfillEmptyDisplayNames,
  buildDisplayById,
  parseCachedModelEntries,
  resolveAutoDisplayName,
  type ModelEntry,
} from "../utils/modelDisplayName";
import { buildPriceById } from "../utils/modelPricing";
import { copyPromiseToClipboard } from "../utils/clipboard";

const props = defineProps<{ initialCliId?: CliId }>();
const { cliOptions } = useSessions();
const FEATURE_CLI_STORAGE_KEY = "claudia-feature-cli-api-profile";
const persistedCliId = localStorage.getItem(FEATURE_CLI_STORAGE_KEY);
const featureCliOptions = computed(() =>
  cliOptions.value.filter((cli) => cli.supportsApiProfiles),
);
const cliSelectOptions = computed<SelectOption[]>(() =>
  featureCliOptions.value.map((cli) => ({
    value: cli.id,
    label: cli.name,
    cliId: cli.id,
  })),
);
const currentCliId = ref<CliId>(resolveFeatureCliId(
  featureCliOptions.value,
  props.initialCliId,
  persistedCliId && isCliId(persistedCliId) ? persistedCliId : undefined,
) ?? "claude");
const currentCli = computed(() =>
  featureCliOptions.value.find((cli) => cli.id === currentCliId.value)
    ?? featureCliOptions.value[0]!,
);

// Proxy Banner State
const proxyBannerEnabled = ref(false);
const proxyBannerPort = ref(18080);

const proxyBannerText = computed(() => {
  const port = proxyBannerPort.value || 18080;
  return `API 代理已启用，Base URL 已临时指向 127.0.0.1:${port}，关闭代理后会自动还原。`;
});

const OFFICIAL_PROFILE_NAME = "Codex Official";

interface ProfileChangedPayload {
  cliId: string;
}

interface LoadAllProfilesOptions {
  refreshActiveEditor?: boolean;
  syncActiveFromCli?: boolean;
}

interface CascadeApplyResult {
  successCount: number;
  failedDirs: string[];
  affectedScopes: string[];
}

const profiles = ref<Profile[]>([]);
const scopeBindings = ref<ScopeBinding[]>([]);
const scopeDirStatuses = ref<ScopeDirStatus[]>([]);
const tabs = ref<TabInfo[]>([]);
const loadError = ref("");
const deleteConfirmName = ref<string | null>(null);
const deletingTab = ref<TabInfo | null>(null);
const showRawEditor = ref(false);
const showCreateTabDialog = ref(false);
const editingTab = ref<TabInfo | null>(null);

// Pending scope binding when user chooses "+ 新建并绑定配置..." from Scope Allocation
const pendingBindScope = ref<string | null>(null);

// Editor States
const editingProfile = ref<string | null>(null);
const editingName = ref("");
const editingContent = ref<Record<string, any>>({});
const editApiView = ref<"form" | "json">("form");
const editJsonText = ref("");
const editJsonError = ref("");
const editSaving = ref(false);
const editSaveSuccess = ref(false);
const showApiKey = ref(false);


const renamingProfile = ref<string | null>(null);
const renameInput = ref("");
const renameInProgress = ref(false);

const profileNameError = computed(() => {
  if (!editingProfile.value) return "";
  const trimmed = editingName.value.trim();
  if (!trimmed) {
    return "配置名称不能为空";
  }
  const norm = normalizeProfileName(trimmed);
  const oldNorm = normalizeProfileName(editingProfile.value);
  if (norm !== oldNorm) {
    const isConflict = profiles.value.some((p) => normalizeProfileName(p.name) === norm);
    if (isConflict) {
      return "已存在同名配置，请使用其他名称";
    }
  }
  return "";
});

const editErrorMessage = computed(() => profileNameError.value || editJsonError.value);

const isClaude = computed(() => currentCliId.value === "claude");
const isCodex = computed(() => currentCliId.value === "codex");

const claudeEffortLevels = ["low", "medium", "high", "xhigh", "max"] as const;
type AttributionMode = "default" | "custom" | "disabled";

const currentSettingsPathHint = computed(() => {
  if (isClaude.value) {
    return `${currentCli.value.dataDirPath}/settings.json`;
  }
  return `${currentCli.value.dataDirPath}/config.toml 与 auth.json`;
});

const profileFields = computed<FieldInfo[]>(() =>
  isClaude.value
    ? [
        { label: "Token", key: "env.ANTHROPIC_AUTH_TOKEN", masked: true },
        { label: "URL", key: "env.ANTHROPIC_BASE_URL" },
        { label: "主模型", key: "env.ANTHROPIC_MODEL" },
        { label: "Effort", key: "env.CLAUDE_CODE_EFFORT_LEVEL" },
        { label: "Haiku", key: "env.ANTHROPIC_DEFAULT_HAIKU_MODEL" },
        { label: "Sonnet", key: "env.ANTHROPIC_DEFAULT_SONNET_MODEL" },
        { label: "Opus", key: "env.ANTHROPIC_DEFAULT_OPUS_MODEL" },
        { label: "超时", key: "env.API_TIMEOUT_MS" },
        { label: "输出", key: "env.CLAUDE_CODE_MAX_OUTPUT_TOKENS" },
        { label: "Beta", key: "env.CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS" },
        { label: "流量", key: "env.CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC" },
        { label: "提交署名", key: "attribution.commit" },
        { label: "PR 署名", key: "attribution.pr" },
      ]
    : [
        { label: "Key", key: "auth.OPENAI_API_KEY", masked: true },
        { label: "URL", key: "codex.base_url" },
        { label: "模型", key: "codex.model" },
        { label: "推理", key: "codex.model_reasoning_effort" },
      ],
);

const profileCardHint = computed(() =>
  isClaude.value
    ? "配置保存后会自动级联同步至已绑定的作用域，hooks 等其他 settings.json 字段会保留原样"
    : "配置保存后会自动同步至 Codex 配置文件，其他 config.toml / auth.json 内容会保留原样",
);

const connectionSectionHint = computed(() =>
  isClaude.value
    ? "兼容 Claude API 的服务端点地址，不要以斜杠结尾"
    : "当前 Provider 的 base_url 地址，不要以斜杠结尾",
);

const modelPlaceholder = computed(() =>
  isClaude.value ? "claude-sonnet-4-20250514" : "gpt-5.4",
);

const apiKeyPlaceholder = computed(() =>
  isClaude.value ? "sk-ant-... 或 JWT token" : "sk-... 或 JWT token",
);

const editEnv = computed(
  () => (editingContent.value.env || {}) as Record<string, unknown>,
);
const editAttribution = computed(
  () => (editingContent.value.attribution || {}) as Record<string, unknown>,
);
const editCodex = computed(
  () => (editingContent.value.codex || {}) as Record<string, string>,
);
const editAuth = computed(
  () => (editingContent.value.auth || {}) as Record<string, string>,
);



function stringifyConfigValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return "";
}

function getNestedValue(content: Record<string, any>, key: string): string {
  const parts = key.split(".");
  let value: any = content;
  for (const part of parts) {
    if (value == null || typeof value !== "object") return "";
    value = value[part];
  }
  return stringifyConfigValue(value);
}

function hasNestedKey(content: Record<string, any>, path: string[]): boolean {
  let value: any = content;
  for (let i = 0; i < path.length - 1; i += 1) {
    const part = path[i];
    if (!value || typeof value !== "object" || Array.isArray(value)) return false;
    value = value[part];
  }
  return Boolean(
    value &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    Object.prototype.hasOwnProperty.call(value, path[path.length - 1]),
  );
}

function setNestedValue(path: string[], value: string) {
  let target: Record<string, any> = editingContent.value;
  for (let i = 0; i < path.length - 1; i += 1) {
    const key = path[i];
    const next = target[key];
    if (!next || typeof next !== "object" || Array.isArray(next)) {
      target[key] = {};
    }
    target = target[key] as Record<string, any>;
  }

  const lastKey = path[path.length - 1];
  if (value) {
    target[lastKey] = value;
  } else {
    delete target[lastKey];
  }

  editJsonError.value = "";
  pruneEmptyObjects(editingContent.value);
  syncEditJson();
}

function setNestedRawValue(path: string[], value: unknown) {
  let target: Record<string, any> = editingContent.value;
  for (let i = 0; i < path.length - 1; i += 1) {
    const key = path[i];
    const next = target[key];
    if (!next || typeof next !== "object" || Array.isArray(next)) {
      target[key] = {};
    }
    target = target[key] as Record<string, any>;
  }

  target[path[path.length - 1]] = value;
  editJsonError.value = "";
  pruneEmptyObjects(editingContent.value);
  syncEditJson();
}

function pruneEmptyObjects(value: Record<string, any>) {
  for (const key of Object.keys(value)) {
    const current = value[key];
    if (!current || typeof current !== "object" || Array.isArray(current)) continue;
    pruneEmptyObjects(current as Record<string, any>);
    if (Object.keys(current).length === 0) {
      delete value[key];
    }
  }
}

function ensureCodexDefaults() {
  if (!editingContent.value.codex || typeof editingContent.value.codex !== "object") {
    editingContent.value.codex = {};
  }
  const codex = editingContent.value.codex as Record<string, string>;
  if (!codex.model_provider) {
    codex.model_provider = "openai-chat-completions";
  }
  if (!codex.provider_name) {
    codex.provider_name = "OpenAI using Chat Completions";
  }
  if (!codex.wire_api) {
    codex.wire_api = "responses";
  }
}

const editApiKey = computed<string, string>({
  get: () =>
    isClaude.value
      ? stringifyConfigValue(editEnv.value.ANTHROPIC_AUTH_TOKEN)
      : stringifyConfigValue(editAuth.value.OPENAI_API_KEY),
  set: (value: string) => {

    if (isClaude.value) {
      setNestedValue(["env", "ANTHROPIC_AUTH_TOKEN"], value);
      return;
    }
    ensureCodexDefaults();
    setNestedValue(["auth", "OPENAI_API_KEY"], value);
  },
});

const editBaseUrl = computed<string, string>({
  get: () =>
    isClaude.value
      ? stringifyConfigValue(editEnv.value.ANTHROPIC_BASE_URL)
      : stringifyConfigValue(editCodex.value.base_url),
  set: (value: string) => {
    if (isClaude.value) {
      setNestedValue(["env", "ANTHROPIC_BASE_URL"], value);
      return;
    }
    ensureCodexDefaults();
    setNestedValue(["codex", "base_url"], value);
  },
});

// Model list fetch mode
const availableModels = ref<ModelEntry[]>([]);
const modelsLoading = ref(false);
const modelsError = ref("");

/** id → display 名称映射（id 不含 [1m] 后缀），用于自动填充显示名称 */
const displayById = computed(() => buildDisplayById(availableModels.value));

/** id → 精简价格（$in/$out per M）映射，用于下拉项 hover 提示 */
const priceById = computed(() => buildPriceById(availableModels.value));

const CACHE_KEY_PREFIX = "claudia-model-list-cache:";



/** Model list filtered by current CLI type */
const filteredModels = computed(() => {
  const ids = availableModels.value.map((m) => m.id);
  if (isClaude.value) {
    // Claude 配置可能指向多模型网关（DeepSeek/Gemini/GLM 等），
    // 不按 anthropic. 前缀过滤，展示拉取到的全部模型
    return ids;
  }
  return ids.filter(id => /codex|gpt/i.test(id));
});

interface ModelGroup {
  label: string;
  items: string[];
}

const GROUP_RULES: { label: string; keywords: string[] }[] = [
  { label: "Anthropic", keywords: ["claude", "anthropic"] },
  { label: "DeepSeek", keywords: ["deepseek"] },
  { label: "Gemini",   keywords: ["gemini"] },
  { label: "Kimi",     keywords: ["kimi"] },
  { label: "GLM",      keywords: ["glm"] },
  { label: "OpenAI",   keywords: ["gpt", "codex"] },
];

/** filteredModels grouped by provider keyword */
const groupedModels = computed((): ModelGroup[] => {
  const buckets: ModelGroup[] = GROUP_RULES.map(r => ({ label: r.label, items: [] }));
  const others: string[] = [];
  for (const id of filteredModels.value) {
    const norm = id.replace(/^anthropic\./i, "").toLowerCase();
    const idx = GROUP_RULES.findIndex(r => r.keywords.some(kw => norm.includes(kw)));
    if (idx >= 0) buckets[idx].items.push(id);
    else others.push(id);
  }
  const result = buckets.filter(b => b.items.length > 0);
  if (others.length > 0) result.push({ label: "其他", items: others });
  return result;
});

/** Which model field's dropdown is currently open */
const activeCombobox = ref<string | null>(null);
let comboboxBlurTimer: ReturnType<typeof setTimeout> | null = null;

function openCombobox(field: string) {
  if (comboboxBlurTimer !== null) {
    clearTimeout(comboboxBlurTimer);
    comboboxBlurTimer = null;
  }
  if (field === "effort") {
    activeCombobox.value = field;
    return;
  }
  if (groupedModels.value.length > 0) {
    activeCombobox.value = field;
  }
}

function comboboxBlur() {
  comboboxBlurTimer = setTimeout(() => {
    activeCombobox.value = null;
    comboboxBlurTimer = null;
  }, 150);
}

function selectMainModel(model: string) {
  editModel.value = model;
  activeCombobox.value = null;
}

function selectClaudeEffortLevel(level: string) {
  editClaudeEffortLevel.value = level;
  activeCombobox.value = null;
}

function updateDefaultModel(field: "haiku" | "sonnet" | "opus", value: string) {
  if (field === "haiku") {
    editHaikuModel.value = value;
    return;
  }
  if (field === "sonnet") {
    editSonnetModel.value = value;
    return;
  }
  editOpusModel.value = value;
}

function updateDefaultModelName(field: "haiku" | "sonnet" | "opus", value: string) {
  if (field === "haiku") {
    editHaikuModelName.value = value;
    return;
  }
  if (field === "sonnet") {
    editSonnetModelName.value = value;
    return;
  }
  editOpusModelName.value = value;
}

function selectDefaultModel(field: "haiku" | "sonnet" | "opus" | "fallback", value: string) {
  if (field === "fallback") {
    editModel.value = value;
  } else {
    const prevModel = { haiku: editHaikuModel, sonnet: editSonnetModel, opus: editOpusModel }[field].value;
    const currentName = { haiku: editHaikuModelName, sonnet: editSonnetModelName, opus: editOpusModelName }[field].value;
    updateDefaultModel(field, value);
    // 智能填充显示名称：名称为空或仍是上一个模型的自动填充值时，用新模型的 display 覆盖
    const autoName = resolveAutoDisplayName({
      currentName,
      prevModel,
      nextModel: value,
      displayById: displayById.value,
    });
    if (autoName !== null) updateDefaultModelName(field, autoName);
  }
  activeCombobox.value = null;
}

/** localStorage cache key for current base URL */
const modelListCacheKey = computed(() => {
  const baseUrl = (editBaseUrl.value as string).replace(/\/+$/, '');
  return `${CACHE_KEY_PREFIX}${baseUrl}`;
});

interface CachedModels {
  base_url: string;
  models: ModelEntry[];
  fetched_at: string;
}

function loadCachedModels(): boolean {
  const entries = parseCachedModelEntries(localStorage.getItem(modelListCacheKey.value));
  if (entries.length > 0) {
    availableModels.value = entries;
    return true;
  }
  return false;
}

const fetchModelsSuccessMsg = ref("");

function showToast(msg: string) {
  fetchModelsSuccessMsg.value = msg;
  setTimeout(() => {
    if (fetchModelsSuccessMsg.value === msg) {
      fetchModelsSuccessMsg.value = "";
    }
  }, 3000);
}

async function fetchModels(): Promise<boolean> {
  if (modelsLoading.value) return false;
  const baseUrl = (editBaseUrl.value as string).replace(/\/+$/, '');
  if (!baseUrl || !editApiKey.value) {
    modelsError.value = "请先填写 API Key";
    return false;
  }

  modelsLoading.value = true;
  modelsError.value = "";
  try {
    const { models, latency_ms } = await invoke("fetch_available_models", {
      baseUrl: baseUrl,
      apiKey: editApiKey.value as string,
    }) as { models: ModelEntry[], latency_ms: number };

    availableModels.value = models;
    showToast(`获取到 ${models.length} 个模型（耗时 ${latency_ms}ms）`);

    const cache: CachedModels = {
      base_url: baseUrl,
      models,
      fetched_at: new Date().toISOString(),
    };
    localStorage.setItem(modelListCacheKey.value, JSON.stringify(cache));

    // 补齐「模型映射」中名称为空且能匹配到 display 的角色行
    if (isClaude.value) {
      const filled = backfillEmptyDisplayNames(
        { haiku: editHaikuModel.value, sonnet: editSonnetModel.value, opus: editOpusModel.value },
        { haiku: editHaikuModelName.value, sonnet: editSonnetModelName.value, opus: editOpusModelName.value },
        displayById.value,
      );
      for (const [role, name] of Object.entries(filled)) {
        updateDefaultModelName(role as "haiku" | "sonnet" | "opus", name);
      }
    }
    return true;
  } catch (err: any) {
    modelsError.value = typeof err === "string" ? err : (err?.message || "拉取模型列表失败");
    return false;
  } finally {
    modelsLoading.value = false;
  }
}

async function handleRefreshModels() {
  await fetchModels();
}

watch(() => editBaseUrl.value, () => {
  if (editBaseUrl.value) {
    const hit = loadCachedModels();
    if (!hit) availableModels.value = [];
  } else {
    availableModels.value = [];
  }
}, { immediate: true });

const editModel = computed<string, string>({
  get: () =>
    isClaude.value
      ? stringifyConfigValue(editEnv.value.ANTHROPIC_MODEL)
      : stringifyConfigValue(editCodex.value.model),
  set: (value: string) => {
    if (isClaude.value) {
      setNestedValue(["env", "ANTHROPIC_MODEL"], value);
      return;
    }
    ensureCodexDefaults();
    setNestedValue(["codex", "model"], value);
  },
});

const editHaikuModel = computed<string, string>({
  get: () => stringifyConfigValue(editEnv.value.ANTHROPIC_DEFAULT_HAIKU_MODEL),
  set: (value: string) => setNestedValue(["env", "ANTHROPIC_DEFAULT_HAIKU_MODEL"], value),
});
const editHaikuModelName = computed<string, string>({
  get: () => stringifyConfigValue(editEnv.value.ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME),
  set: (value: string) => setNestedValue(["env", "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME"], value),
});
const editSonnetModel = computed<string, string>({
  get: () => stringifyConfigValue(editEnv.value.ANTHROPIC_DEFAULT_SONNET_MODEL),
  set: (value: string) => setNestedValue(["env", "ANTHROPIC_DEFAULT_SONNET_MODEL"], value),
});
const editSonnetModelName = computed<string, string>({
  get: () => stringifyConfigValue(editEnv.value.ANTHROPIC_DEFAULT_SONNET_MODEL_NAME),
  set: (value: string) => setNestedValue(["env", "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME"], value),
});
const editOpusModel = computed<string, string>({
  get: () => stringifyConfigValue(editEnv.value.ANTHROPIC_DEFAULT_OPUS_MODEL),
  set: (value: string) => setNestedValue(["env", "ANTHROPIC_DEFAULT_OPUS_MODEL"], value),
});
const editOpusModelName = computed<string, string>({
  get: () => stringifyConfigValue(editEnv.value.ANTHROPIC_DEFAULT_OPUS_MODEL_NAME),
  set: (value: string) => setNestedValue(["env", "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME"], value),
});
const editTimeoutMs = computed<string, string>({
  get: () => stringifyConfigValue(editEnv.value.API_TIMEOUT_MS),
  set: (value: string) => setNestedValue(["env", "API_TIMEOUT_MS"], value),
});
const editClaudeMaxOutputTokens = computed<string, string>({
  get: () => stringifyConfigValue(editEnv.value.CLAUDE_CODE_MAX_OUTPUT_TOKENS),
  set: (value: string) => setNestedValue(["env", "CLAUDE_CODE_MAX_OUTPUT_TOKENS"], value),
});
const editClaudeDisableExperimentalBetas = computed<string, string>({
  get: () => stringifyConfigValue(editEnv.value.CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS),
  set: (value: string) => setNestedValue(["env", "CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS"], value),
});
const editClaudeDisableNonessentialTraffic = computed<string, string>({
  get: () => stringifyConfigValue(editEnv.value.CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC),
  set: (value: string) => setNestedValue(["env", "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"], value),
});
const editClaudeEffortLevel = computed<string, string>({
  get: () => getNestedValue(editingContent.value, "env.CLAUDE_CODE_EFFORT_LEVEL"),
  set: (value: string) => setNestedValue(["env", "CLAUDE_CODE_EFFORT_LEVEL"], value),
});
const editAttributionMode = computed<AttributionMode>({
  get: () => {
    const hasCommit = hasNestedKey(editingContent.value, ["attribution", "commit"]);
    const hasPr = hasNestedKey(editingContent.value, ["attribution", "pr"]);
    if (!hasCommit && !hasPr) return "default";
    if (editAttribution.value.commit === "" && editAttribution.value.pr === "") {
      return "disabled";
    }
    return "custom";
  },
  set: (mode: AttributionMode) => {
    if (mode === "default") {
      setNestedValue(["attribution", "commit"], "");
      setNestedValue(["attribution", "pr"], "");
      return;
    }
    if (mode === "disabled") {
      setNestedRawValue(["attribution", "commit"], "");
      setNestedRawValue(["attribution", "pr"], "");
      return;
    }

    const currentCommit = stringifyConfigValue(editAttribution.value.commit);
    const currentPr = stringifyConfigValue(editAttribution.value.pr);
    setNestedRawValue(
      ["attribution", "commit"],
      currentCommit || "Co-Authored-By: Claude <noreply@anthropic.com>",
    );
    if (currentPr) {
      setNestedRawValue(["attribution", "pr"], currentPr);
    } else {
      setNestedValue(["attribution", "pr"], "");
    }
  },
});
const editCommitAttribution = computed<string, string>({
  get: () => stringifyConfigValue(editAttribution.value.commit),
  set: (value: string) => setNestedValue(["attribution", "commit"], value),
});
const editPrAttribution = computed<string, string>({
  get: () => stringifyConfigValue(editAttribution.value.pr),
  set: (value: string) => setNestedValue(["attribution", "pr"], value),
});
const editReasoningEffort = computed<string, string>({
  get: () => stringifyConfigValue(editCodex.value.model_reasoning_effort),
  set: (value: string) => {
    ensureCodexDefaults();
    setNestedValue(["codex", "model_reasoning_effort"], value);
  },
});

function syncEditJson() {
  editJsonText.value = JSON.stringify(editingContent.value, null, 2);
  editJsonError.value = "";
}

function onEditJsonInput(text: string) {
  editJsonText.value = text;
  try {
    editingContent.value = JSON.parse(text);
    if (isCodex.value) {
      ensureCodexDefaults();
    }
    editJsonError.value = "";
  } catch (e: any) {
    editJsonError.value = e.message;
  }
}

function switchEditView(view: "form" | "json") {
  if (view === "json") {
    syncEditJson();
  }
  editApiView.value = view;
}

function normalizeProfileName(name: string): string {
  return name.trim().toLocaleLowerCase();
}

function findProfileNameConflict(name: string, excludeName?: string): string | null {
  const normalized = normalizeProfileName(name);
  if (!normalized) return null;
  const excludeNorm = excludeName ? normalizeProfileName(excludeName) : null;

  return (
    profiles.value.find(
      (profile) => (excludeNorm ? normalizeProfileName(profile.name) !== excludeNorm : true) && normalizeProfileName(profile.name) === normalized,
    )?.name ?? null
  );
}

function hasNonEmptyCodexOAuthValue(value: unknown): boolean {
  if (value == null) return false;
  if (typeof value === "string") return value.trim().length > 0;
  if (Array.isArray(value)) return value.length > 0;
  if (typeof value === "object") return Object.keys(value as Record<string, unknown>).length > 0;
  return true;
}

function codexSettingsHasManagedProviderFields(settings: Record<string, any>): boolean {
  const codex = settings.codex;
  if (!codex || typeof codex !== "object" || Array.isArray(codex)) return false;
  const fields = ["base_url", "model", "model_reasoning_effort", "provider_name", "wire_api"];
  if (fields.some((key) => hasNonEmptyCodexOAuthValue(codex[key]))) return true;
  const provider = typeof codex.model_provider === "string" ? codex.model_provider.trim() : "";
  return provider.length > 0 && provider !== "openai-chat-completions";
}

function codexOfficialContentFromSettings(settings: Record<string, any>): Record<string, any> {
  const auth = settings.auth;
  if (!auth || typeof auth !== "object" || Array.isArray(auth)) return {};
  return { auth: JSON.parse(JSON.stringify(auth)) };
}

async function loadProxyBanner() {
  try {
    const status = await invoke<{ enabled: boolean; running: boolean; port: number }>(
      "proxy_status",
      { cliId: currentCliId.value },
    );
    proxyBannerEnabled.value = status.enabled && status.running;
    proxyBannerPort.value = status.port || 18080;
  } catch {
    proxyBannerEnabled.value = false;
    proxyBannerPort.value = 18080;
  }
}

// Available Profiles formatted for Scope Allocation dropdown
const availableProfileSummaries = computed<ProfileSummary[]>(() => {
  return profiles.value.map((p) => {
    let model = "";
    let baseUrl = "";
    if (isClaude.value) {
      model = getNestedValue(p.content, "env.ANTHROPIC_MODEL");
      baseUrl = getNestedValue(p.content, "env.ANTHROPIC_BASE_URL");
    } else {
      model = getNestedValue(p.content, "codex.model");
      baseUrl = getNestedValue(p.content, "codex.base_url");
    }
    return {
      name: p.name,
      model: model || undefined,
      baseUrl: baseUrl || undefined,
    };
  });
});

// Backup Menu
const showImportConflictDialog = ref(false);
const importCandidates = ref<ImportCandidate[]>([]);
const importedScopeItems = ref<ScopeImportItem[]>([]);
const importedBackupScopeBindings = ref<any[]>([]);
const showImportMenu = ref(false);
const showExportMenu = ref(false);

function closeBackupMenus() {
  showImportMenu.value = false;
  showExportMenu.value = false;
}

function toggleImportMenu() {
  showImportMenu.value = !showImportMenu.value;
  showExportMenu.value = false;
}

function toggleExportMenu() {
  showExportMenu.value = !showExportMenu.value;
  showImportMenu.value = false;
}

async function onExportToClipboard() {
  closeBackupMenus();
  const confirm = await ask("备份内容将包含所有 CLI 的完整 API 配置及明文 Key/Token，剪贴板内容可被其他应用读取，请谨慎操作并及时清理。", {
    title: "导出备份警告",
    kind: "warning",
    okLabel: "继续复制",
    cancelLabel: "取消",
  });
  if (!confirm) return;

  try {
    // 必须在用户手势激活期内同步发起写入（旧版 WKWebView 限制），
    // 因此用 copyPromiseToClipboard 包裹异步的导出 Promise
    const copied = await copyPromiseToClipboard(
      invoke<any>("export_profiles_backup", {}).then((data) =>
        JSON.stringify(data, null, 2)
      )
    );
    if (copied) {
      showToast("配置备份已复制到剪贴板");
    } else {
      loadError.value = "写入剪贴板失败";
    }
  } catch (err: any) {
    loadError.value = String(err);
  }
}

async function onImportFromClipboard() {
  closeBackupMenus();
  // 走原生插件读取剪贴板：Web 层 navigator.clipboard.readText() 在 WKWebView
  // 中会不定时弹出系统「粘贴」确认按钮（transient activation 判定），原生读取无此问题
  let text = "";
  try {
    text = (await readClipboardText()) ?? "";
  } catch (err) {
    await message(`无法读取剪贴板内容：${err}`, { title: "导入失败", kind: "error" });
    return;
  }
  if (!text.trim()) {
    await message("剪贴板为空，请先复制配置备份 JSON", { title: "无法导入", kind: "warning" });
    return;
  }

  try {
    const backupData = await invoke<any>("parse_profiles_backup", { content: text });
    await processBackupData(backupData);
  } catch (err: any) {
    await message(`剪贴板内容不是有效的配置备份：${err}`, { title: "导入失败", kind: "error" });
  }
}

async function onExportBackup() {
  const confirm = await ask("备份文件将包含所有 CLI 的完整 API 配置及明文 Key/Token，请妥善保管所导出的 JSON 文件。", {
    title: "导出备份警告",
    kind: "warning",
    okLabel: "继续导出",
    cancelLabel: "取消",
  });
  if (!confirm) return;

  try {
    const d = new Date();
    const YYYYMMDD = `${d.getFullYear()}${String(d.getMonth()+1).padStart(2,'0')}${String(d.getDate()).padStart(2,'0')}`;
    const HHMM = `${String(d.getHours()).padStart(2,'0')}${String(d.getMinutes()).padStart(2,'0')}`;
    const defaultName = `claudia_api_profiles_${YYYYMMDD}_${HHMM}.json`;
    const filePath = await save({ defaultPath: defaultName });
    if (!filePath) return;

    await invoke("export_profiles_backup", { savePath: filePath });
    showToast("导出配置备份成功");
  } catch (err: any) {
    loadError.value = String(err);
  }
}

async function onImportBackup() {
  closeBackupMenus();
  try {
    const selected = await open({
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!selected) return;

    const backupData = await invoke<any>("read_profiles_backup", { filePath: selected as string });
    await processBackupData(backupData);
  } catch (err: any) {
    loadError.value = String(err);
  }
}

// 文件导入与剪贴板导入共用的下游流程：格式校验 → 冲突检测 → 导入
async function processBackupData(backupData: any) {
  try {
    if (!backupData || !Array.isArray(backupData.profiles)) {
      await message("备份文件格式无效", { title: "错误", kind: "error" });
      return;
    }
    if (backupData.profiles.length === 0) {
      await message("备份文件中未包含任何 API 配置！", { title: "文件为空", kind: "warning" });
      return;
    }

    importedBackupScopeBindings.value = backupData.scope_bindings || [];
    // 作用域冲突检测：同 id → 已存在（默认覆盖）；同名不同 id → 重名（默认跳过）；否则新增
    const backupTabs: any[] = backupData.tabs || [];
    const tabCliIds: string[] = [...new Set<string>(backupTabs.map((t: any) => String(t.cli_id)))];
    const localTabsByCli = new Map<string, any[]>();
    await Promise.all(
      tabCliIds.map(async (cliId) => {
        try {
          localTabsByCli.set(cliId, await invoke<any[]>("list_profile_tabs", { cliId }));
        } catch {
          localTabsByCli.set(cliId, []);
        }
      })
    );
    importedScopeItems.value = backupTabs.map((t: any) => {
      const local = localTabsByCli.get(String(t.cli_id)) ?? [];
      const norm = String(t.name).trim().toLowerCase();
      const sameId = local.some((l: any) => l.id === t.id);
      const sameName = local.some(
        (l: any) => l.id !== t.id && String(l.name).trim().toLowerCase() === norm
      );
      const status: ScopeImportItem["status"] = sameId
        ? "existing"
        : sameName
          ? "nameConflict"
          : "new";
      return {
        id: String(t.id),
        cli_id: String(t.cli_id),
        name: String(t.name),
        dirs: Array.isArray(t.dirs) ? t.dirs : [],
        status,
        action: status === "existing" ? "overwrite" : status === "nameConflict" ? "skip" : "add",
      } as ScopeImportItem;
    });
    const candidates: ImportCandidate[] = [];
    // 备份可包含多个 CLI 的配置，冲突检测须按各 CLI 的本地配置分别比对
    const cliIds: string[] = [
      ...new Set<string>(backupData.profiles.map((p: any) => String(p.cli_id))),
    ];
    const namesByCli = new Map<string, Set<string>>();
    await Promise.all(
      cliIds.map(async (cliId) => {
        const names = await invoke<string[]>("list_profiles", { cliId });
        namesByCli.set(cliId, new Set(names.map((n) => normalizeProfileName(n))));
      })
    );

    for (const p of backupData.profiles) {
      const isConflict =
        namesByCli.get(String(p.cli_id))?.has(normalizeProfileName(p.name)) ?? false;
      candidates.push({
        cli_id: p.cli_id,
        scope: p.scope || "global",
        name: p.name,
        content: typeof p.content === "string" ? JSON.parse(p.content) : p.content,
        is_active: p.is_active,
        isConflict,
        action: "overwrite",
        newName: p.name,
      });
    }

    if (
      candidates.some((c) => c.isConflict) ||
      importedScopeItems.value.some((s) => s.status === "nameConflict")
    ) {
      importCandidates.value = candidates;
      showImportConflictDialog.value = true;
    } else {
      await executeImport(candidates, importedScopeItems.value);
    }
  } catch (err: any) {
    loadError.value = String(err);
  }
}

async function executeImport(candidates: ImportCandidate[], scopeItems?: ScopeImportItem[]) {
  try {
    const items = candidates
      .filter((c) => c.action !== "skip")
      .map((c) => ({
        cli_id: c.cli_id,
        scope: c.scope,
        name: c.name,
        content: typeof c.content === "string" ? c.content : JSON.stringify(c.content, null, 2),
        is_active: c.is_active,
        action: c.action,
        new_name: c.action === "rename" ? c.newName : undefined,
      }));

    // 仅提交用户未跳过的作用域（overwrite=更新、add=新增、skip=跳过）
    const tabsToImport = (scopeItems ?? importedScopeItems.value)
      .filter((s) => s.action !== "skip")
      .map((s) => ({
        id: s.id,
        cli_id: s.cli_id,
        name: s.name,
        dirs: s.dirs,
        sort_order: 0,
      }));

    if (items.length > 0 || tabsToImport.length > 0 || importedBackupScopeBindings.value.length > 0) {
      await invoke("import_profiles_backup", {
        items,
        tabs: tabsToImport,
        scopeBindings: importedBackupScopeBindings.value,
      });
      await loadAllProfiles();
      showToast("导入恢复配置成功");
    }
    showImportConflictDialog.value = false;
  } catch (err: any) {
    loadError.value = String(err);
  }
}

async function loadTabs() {
  if (currentCliId.value !== "claude") {
    tabs.value = [];
    return;
  }
  try {
    tabs.value = await invoke<TabInfo[]>("list_profile_tabs", { cliId: "claude" });
  } catch (err) {
    console.error("Load tabs failed:", err);
  }
}

async function loadAllProfiles(options: LoadAllProfilesOptions = {}) {
  const {
    refreshActiveEditor = false,
  } = options;

  try {
    const cliId = currentCliId.value;

    await loadTabs();

    const names = await invoke<string[]>("list_profiles", { cliId });
    const bindings = await invoke<ScopeBinding[]>("get_scope_bindings", { cliId });
    scopeBindings.value = bindings;

    if (cliId === "claude") {
      try {
        scopeDirStatuses.value = await invoke<ScopeDirStatus[]>("get_scope_dir_status", { cliId });
      } catch (err) {
        console.error("Load scope dir status failed:", err);
        scopeDirStatuses.value = [];
      }
    } else {
      scopeDirStatuses.value = [];
    }

    const profileResults = await Promise.all(
      names.map(async (name) => {
        try {
          const raw = await invoke<string>("read_profile", { cliId, name });
          const usedInScopes = bindings
            .filter((b) => b.activeProfile === name)
            .map((b) => ({
              scope: b.scope,
              name: b.name || (b.isGlobal || b.scope === "global" ? "全局" : b.scope),
            }));

          return {
            name,
            content: JSON.parse(raw),
            usedInScopes,
          } as Profile;
        } catch {
          return null;
        }
      })
    );
    const loaded = profileResults.filter((p): p is Profile => p !== null);
    profiles.value = loaded;

    if (refreshActiveEditor && editingProfile.value) {
      const editing = loaded.find((profile) => profile.name === editingProfile.value);
      if (editing) {
        editingContent.value = JSON.parse(JSON.stringify(editing.content));
        if (isCodex.value) {
          ensureCodexDefaults();
        }
        syncEditJson();
      }
    }

    loadError.value = "";
    invoke("refresh_tray_menu", { cliId }).catch(() => {});
  } catch (e: any) {
    loadError.value = String(e);
  }
}

async function createProfile() {
  let base = "新配置";
  let name = base;
  let i = 1;
  const existing = new Set(profiles.value.map((profile) => normalizeProfileName(profile.name)));
  while (existing.has(normalizeProfileName(name))) {
    i += 1;
    name = `${base} ${i}`;
  }
  try {
    const cliId = currentCliId.value;
    const content = isCodex.value
      ? JSON.stringify(
          {
            auth: {},
            codex: {
              model_provider: "openai-chat-completions",
              provider_name: "OpenAI using Chat Completions",
              wire_api: "responses",
            },
          },
          null,
          2,
        )
      : "{}";
    await invoke("save_profile", { cliId, name, content });
    await loadAllProfiles();
    openEditor(name);
  } catch (e: any) {
    loadError.value = String(e);
  }
}

async function duplicateProfile(srcName: string) {
  if (srcName === OFFICIAL_PROFILE_NAME) return;
  const src = profiles.value.find((profile) => profile.name === srcName);
  if (!src) return;
  let name = `${srcName} copy`;
  let i = 1;
  const existing = new Set(profiles.value.map((profile) => normalizeProfileName(profile.name)));
  while (existing.has(normalizeProfileName(name))) {
    i += 1;
    name = `${srcName} copy ${i}`;
  }
  try {
    const contentCopy = JSON.parse(JSON.stringify(src.content));
    await invoke("save_profile", {
      cliId: currentCliId.value,
      name,
      content: JSON.stringify(contentCopy, null, 2),
    });
    await loadAllProfiles({ syncActiveFromCli: false });
    openEditor(name);
  } catch (e: any) {
    loadError.value = String(e);
  }
}

function requestDeleteProfile(name: string) {
  if (name === OFFICIAL_PROFILE_NAME) return;
  deleteConfirmName.value = name;
}

function cancelDelete() {
  deleteConfirmName.value = null;
}

async function confirmDeleteProfile() {
  const name = deleteConfirmName.value;
  if (!name) return;
  deleteConfirmName.value = null;
  try {
    await invoke("delete_profile", { cliId: currentCliId.value, name });
    if (editingProfile.value === name) editingProfile.value = null;
    await loadAllProfiles();
    showToast(`配置「${name}」已删除`);
  } catch (e: any) {
    loadError.value = String(e);
  }
}

function startRename(name: string) {
  if (name === OFFICIAL_PROFILE_NAME) return;
  renamingProfile.value = name;
  renameInput.value = name;
  nextTick(() => {
    const el = document.querySelector(".profile-rename-input") as HTMLInputElement;
    el?.focus();
    el?.select();
  });
}

function cancelRename() {
  renamingProfile.value = null;
  renameInput.value = "";
}

function handleCardDoubleClick(name: string) {
  if (renamingProfile.value === name) return;
  openEditor(name);
}

async function confirmRename(oldName: string) {
  if (renameInProgress.value) return;
  const newName = renameInput.value.trim();
  if (!newName || newName === oldName) {
    cancelRename();
    return;
  }

  const conflictName = findProfileNameConflict(newName, oldName);
  if (conflictName) {
    loadError.value = `配置 '${conflictName}' 已存在`;
    return;
  }

  renameInProgress.value = true;
  try {
    await invoke("rename_profile", {
      cliId: currentCliId.value,
      oldName,
      newName,
    });
    if (editingProfile.value === oldName) editingProfile.value = newName;
    cancelRename();
    await loadAllProfiles();
    showToast(`配置已重命名为「${newName}」`);
  } catch (e: any) {
    loadError.value = String(e);
    cancelRename();
  } finally {
    renameInProgress.value = false;
  }
}

function openEditor(name: string) {
  if (name === OFFICIAL_PROFILE_NAME) return;
  const profile = profiles.value.find((item) => item.name === name);
  if (!profile) return;
  editingProfile.value = name;
  editingName.value = name;
  editingContent.value = JSON.parse(JSON.stringify(profile.content));
  if (isCodex.value) {
    ensureCodexDefaults();
  }
  editApiView.value = "form";
  showApiKey.value = false;

  syncEditJson();
  editSaveSuccess.value = false;
}

function closeEditor() {
  editingProfile.value = null;
  editingName.value = "";
  pendingBindScope.value = null;
}

async function saveEditingProfile() {
  if (profileNameError.value || editErrorMessage.value || !editingProfile.value) return;
  editSaving.value = true;
  editSaveSuccess.value = false;
  try {
    if (isCodex.value) {
      ensureCodexDefaults();
    }
    const content = JSON.stringify(editingContent.value, null, 2);
    const oldName = editingProfile.value;
    const newName = editingName.value.trim();

    if (newName !== oldName) {
      await invoke("rename_profile", {
        cliId: currentCliId.value,
        oldName,
        newName,
      });
      editingProfile.value = newName;
    }

    const result = await invoke<CascadeApplyResult>("save_profile", {
      cliId: currentCliId.value,
      name: newName,
      content,
    });

    if (pendingBindScope.value) {
      const scopeToBind = pendingBindScope.value;
      pendingBindScope.value = null;
      await invoke("set_scope_binding", {
        cliId: currentCliId.value,
        scope: scopeToBind,
        profileName: newName,
      });
      showToast(`已保存配置并绑定至作用域`);
    } else if (result && result.affectedScopes && result.affectedScopes.length > 0) {
      showToast(`已保存配置，并已自动级联更新 ${result.affectedScopes.length} 个作用域`);
    } else {
      showToast(`已保存配置`);
    }

    editSaveSuccess.value = true;
    setTimeout(() => {
      editSaveSuccess.value = false;
    }, 2000);
    await loadAllProfiles();
  } catch (e: any) {
    editJsonError.value = String(e);
  } finally {
    editSaving.value = false;
  }
}

async function saveAndApplyEditingProfile() {
  if (profileNameError.value || editErrorMessage.value || !editingProfile.value) return;
  editSaving.value = true;
  editSaveSuccess.value = false;
  try {
    if (isCodex.value) {
      ensureCodexDefaults();
    }
    const content = JSON.stringify(editingContent.value, null, 2);
    const oldName = editingProfile.value;
    const newName = editingName.value.trim();

    if (newName !== oldName) {
      await invoke("rename_profile", {
        cliId: currentCliId.value,
        oldName,
        newName,
      });
      editingProfile.value = newName;
    }

    await invoke<CascadeApplyResult>("save_profile", {
      cliId: currentCliId.value,
      name: newName,
      content,
    });

    const targetScope = pendingBindScope.value || "global";
    pendingBindScope.value = null;
    await invoke("set_scope_binding", {
      cliId: currentCliId.value,
      scope: targetScope,
      profileName: newName,
    });

    showToast(`已保存并应用配置「${newName}」`);
    editSaveSuccess.value = true;
    setTimeout(() => {
      editSaveSuccess.value = false;
    }, 2000);
    await loadAllProfiles();
  } catch (e: any) {
    editJsonError.value = String(e);
  } finally {
    editSaving.value = false;
  }
}

const isEditingActiveProfile = computed(() => {
  if (!editingProfile.value) return false;
  const globalBinding = scopeBindings.value.find((b) => b.isGlobal || b.scope === "global");
  return globalBinding?.activeProfile === editingProfile.value;
});

function profileMatchesSettings(
  profile: Record<string, any>,
  settings: Record<string, any>,
): boolean {
  let hasConfiguredField = false;

  for (const field of profileFields.value) {
    const profileVal = getNestedValue(profile, field.key);
    const settingsVal = getNestedValue(settings, field.key);
    if (profileVal) {
      hasConfiguredField = true;
      if (profileVal !== settingsVal) return false;
    }
  }

  return hasConfiguredField;
}

// Scope Actions
async function handleChangeBinding(scope: string, profileName: string) {
  try {
    await invoke("set_scope_binding", {
      cliId: currentCliId.value,
      scope,
      profileName,
    });

    const targetScope = scopeBindings.value.find((b) => b.scope === scope);
    const scopeName = targetScope?.name || (scope === "global" ? "全局" : scope);

    if (scope === "global") {
      showToast(`全局生效配置已切换为「${profileName}」`);
    } else if (profileName) {
      showToast(`作用域「${scopeName}」已应用配置「${profileName}」`);
    } else {
      showToast(`作用域「${scopeName}」已恢复继承全局配置`);
    }

    await loadAllProfiles();
  } catch (e: any) {
    loadError.value = String(e);
  }
}

async function createTab(name: string, dirs: string[], action: ScopeCreateAction | null) {
  try {
    const tab = await invoke<TabInfo>("create_profile_tab", { cliId: "claude", name, dirs });

    let actionMsg = "";
    if (action?.type === "bind" && action.profileName) {
      await invoke("set_scope_binding", {
        cliId: "claude",
        scope: tab.id,
        profileName: action.profileName,
      });
      actionMsg = `，已绑定配置「${action.profileName}」`;
    } else if (action?.type === "import") {
      const imported = await invoke<string>("import_scope_dir_config", {
        cliId: "claude",
        dir: action.sourceDir,
        scope: tab.id,
      });
      actionMsg = `，已导入项目已有配置为「${imported}」并绑定`;
    }

    await loadTabs();
    showCreateTabDialog.value = false;
    await loadAllProfiles();
    showToast(`作用域「${name}」创建成功${actionMsg}`);
  } catch (err: any) {
    loadError.value = err.message || "创建作用域失败";
  }
}

async function updateTab(name: string, dirs: string[]) {
  if (!editingTab.value) return;
  try {
    const tabId = editingTab.value.id;
    await invoke("update_profile_tab", { tabId, name, dirs });
    await loadTabs();
    editingTab.value = null;
    await loadAllProfiles();
    showToast(`作用域「${name}」已更新`);
  } catch (err: any) {
    loadError.value = err.message || "修改作用域失败";
  }
}

async function deleteTab(tab: TabInfo, cleanDisk: boolean) {
  try {
    await invoke("delete_profile_tab", { tabId: tab.id, cleanDisk });
    await loadTabs();
    await loadAllProfiles();
    showToast(`作用域「${tab.name}」已删除`);
  } catch (err: any) {
    await message(err.message || "删除作用域失败", { title: "错误", kind: "error" });
  }
}

function handleEditScope(scopeId: string) {
  const tab = tabs.value.find((t) => t.id === scopeId);
  if (tab) {
    editingTab.value = tab;
  }
}

async function handleDeleteScope(scopeId: string) {
  const tab = tabs.value.find((t) => t.id === scopeId);
  if (tab) {
    deletingTab.value = tab;
  }
}

async function confirmDeleteTab(cleanDisk: boolean) {
  const tab = deletingTab.value;
  deletingTab.value = null;
  if (tab) {
    await deleteTab(tab, cleanDisk);
  }
}

// Cross navigation linkings
function handleNavigateToScope(_scopeId: string) {
  const el = document.querySelector(".scope-allocation-section");
  if (el) {
    el.scrollIntoView({ behavior: "smooth" });
  }
}

function handleEditProfileFromScope(profileName: string) {
  openEditor(profileName);
}

async function handleCreateProfileForScope(scopeId: string) {
  pendingBindScope.value = scopeId;
  await createProfile();
}

async function initProfiles() {
  const cliId = currentCliId.value;
  await loadAllProfiles();

  let currentSettings: Record<string, any> = {};
  try {
    const raw = await invoke<string>("read_scope_settings", { cliId, scope: "global" });
    currentSettings = JSON.parse(raw);
  } catch {
    currentSettings = {};
  }

  const hasContent = Object.keys(currentSettings).length > 0;
  const authObj = (currentSettings as any)?.auth;
  const hasCodexOAuth = Boolean(
    isCodex.value &&
    authObj &&
    typeof authObj === "object" &&
    Object.entries(authObj).some(
      ([k, v]) => k !== "auth_mode" && k !== "OPENAI_API_KEY" && hasNonEmptyCodexOAuthValue(v),
    )
  );

  // If OAuth logged in on Codex but Codex Official profile doesn't exist, create it
  if (hasContent && hasCodexOAuth && !profiles.value.some((p) => p.name === OFFICIAL_PROFILE_NAME)) {
    await invoke("save_profile", {
      cliId: currentCliId.value,
      name: OFFICIAL_PROFILE_NAME,
      content: JSON.stringify(codexOfficialContentFromSettings(currentSettings), null, 2),
    });
    await loadAllProfiles();
  }

  if (
    hasCodexOAuth &&
    !codexSettingsHasManagedProviderFields(currentSettings) &&
    profiles.value.some((profile) => profile.name === OFFICIAL_PROFILE_NAME) &&
    !scopeBindings.value.some((b) => b.activeProfile)
  ) {
    await invoke("set_scope_binding", {
      cliId: currentCliId.value,
      scope: "global",
      profileName: OFFICIAL_PROFILE_NAME,
    });
    await loadAllProfiles();
    return;
  }

  if (profiles.value.length === 0) {
    if (hasContent) {
      await invoke("save_profile", {
        cliId: currentCliId.value,
        name: "default",
        content: JSON.stringify(currentSettings, null, 2),
      });
      await invoke("set_scope_binding", {
        cliId: currentCliId.value,
        scope: "global",
        profileName: "default",
      });
      await loadAllProfiles();
    }
    return;
  }

  const hasAnyActiveBinding = scopeBindings.value.some((b) => b.activeProfile);
  if (hasAnyActiveBinding) {
    return;
  }

  if (hasContent) {
    // 精确优先：按 base_url + token + 模型字段身份匹配（后端），无命中再退回宽松子集匹配
    let matched: Profile | undefined;
    try {
      const exactName = await invoke<string | null>("match_profile_by_settings", {
        cliId: currentCliId.value,
        settingsContent: JSON.stringify(currentSettings),
      });
      if (exactName) {
        matched = profiles.value.find((profile) => profile.name === exactName);
      }
    } catch (err) {
      console.error("Exact profile match failed, falling back:", err);
    }
    if (!matched) {
      matched = profiles.value.find((profile) =>
        profileMatchesSettings(profile.content, currentSettings),
      );
    }
    if (matched) {
      await invoke("set_scope_binding", {
        cliId: currentCliId.value,
        scope: "global",
        profileName: matched.name,
      });
      await loadAllProfiles();
    } else {
      let name = "default";
      let i = 1;
      const existing = new Set(profiles.value.map((profile) => profile.name));
      while (existing.has(name)) {
        i += 1;
        name = `default ${i}`;
      }
      await invoke("save_profile", {
        cliId: currentCliId.value,
        name,
        content: JSON.stringify(currentSettings, null, 2),
      });
      await invoke("set_scope_binding", {
        cliId: currentCliId.value,
        scope: "global",
        profileName: name,
      });
      await loadAllProfiles();
    }
  }
}

let unlistenProfileChanged: UnlistenFn | null = null;
let removeProxyStatusListener: (() => void) | null = null;

async function reloadForCli() {
  closeEditor();
  cancelRename();
  deleteConfirmName.value = null;
  pendingBindScope.value = null;
  await Promise.all([initProfiles(), loadProxyBanner()]);
}

watch(currentCliId, (cliId) => {
  if (cliId) localStorage.setItem(FEATURE_CLI_STORAGE_KEY, cliId);
  reloadForCli();
});

watch(
  () => props.initialCliId,
  (entryCliId) => {
    currentCliId.value = resolveFeatureCliId(
      featureCliOptions.value,
      entryCliId,
      currentCliId.value,
    ) ?? currentCliId.value;
  },
);

watch(renameInput, () => {
  if (renamingProfile.value) {
    loadError.value = "";
  }
});

function handleEditorKeydown(e: KeyboardEvent) {
  if (!editingProfile.value) return;
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") {
    e.preventDefault();
    void saveEditingProfile();
  } else if (e.key === "Escape" && !activeCombobox.value) {
    closeEditor();
  }
}

onMounted(async () => {
  window.addEventListener("keydown", handleEditorKeydown);
  document.addEventListener("click", closeBackupMenus);
  await Promise.all([initProfiles(), loadProxyBanner()]);
  unlistenProfileChanged = await listen<ProfileChangedPayload>("profile-changed", (event) => {
    if (event.payload?.cliId !== currentCliId.value) return;
    void loadAllProfiles();
  });
  removeProxyStatusListener = addProxyStatusChangedListener(({ cliId, status }) => {
    if (cliId !== currentCliId.value) return;
    proxyBannerEnabled.value = !!status?.enabled && !!status?.running;
    proxyBannerPort.value = status?.port || 18080;
    void loadAllProfiles({ refreshActiveEditor: true });
  });
});

onUnmounted(() => {
  window.removeEventListener("keydown", handleEditorKeydown);
  document.removeEventListener("click", closeBackupMenus);
  if (comboboxBlurTimer !== null) {
    clearTimeout(comboboxBlurTimer);
    comboboxBlurTimer = null;
  }
  unlistenProfileChanged?.();
  removeProxyStatusListener?.();
});

const exposedEditingProfile = computed(() => editingProfile.value);

defineExpose({
  editingProfile: exposedEditingProfile,
  editJsonError: editErrorMessage,
  editSaving,
  editSaveSuccess,
  isEditingActiveProfile,
  closeEditor,
  saveEditingProfile,
  saveAndApplyEditingProfile,
});
</script>

<template>
  <div v-if="loadError" class="error-banner">{{ loadError }}</div>

  <ApiProfileProxyBanner v-if="proxyBannerEnabled" :text="proxyBannerText" />

  <Transition name="model-toast">
    <div v-if="fetchModelsSuccessMsg" class="fetch-model-success-toast">
      <SvgIcon name="check" :size="16" />
      <span>{{ fetchModelsSuccessMsg }}</span>
    </div>
  </Transition>

  <!-- Page Top Header (Hidden in Editor mode) -->
  <div v-if="!editingProfile" class="api-page-header">
    <div class="header-title-wrap">
      <h2 class="page-title">API 配置</h2>
      <div class="header-cli-select-wrap">
        <ElegantSelect
          v-model="currentCliId"
          :options="cliSelectOptions"
          size="small"
          data-testid="api-profile-cli-select"
          aria-label="API 配置 CLI"
        />
      </div>
    </div>

    <!-- Top Right Utilities -->
    <div class="header-actions">
      <div class="backup-menu-wrapper" @click.stop>
        <button
          type="button"
          class="btn-header-tool"
          title="导入 API 配置备份"
          @click="toggleImportMenu"
        >
          <SvgIcon name="upload" :size="12" />
          <span>导入备份</span>
          <SvgIcon name="chevron-down" :size="10" class="backup-chevron" :class="{ rotated: showImportMenu }" />
        </button>
        <div v-if="showImportMenu" class="backup-menu">
          <button type="button" @click="onImportBackup">从文件导入…</button>
          <button type="button" @click="onImportFromClipboard">从剪贴板导入</button>
        </div>
      </div>

      <div class="backup-menu-wrapper" @click.stop>
        <button
          type="button"
          class="btn-header-tool"
          title="导出 API 配置备份"
          @click="toggleExportMenu"
        >
          <SvgIcon name="download" :size="12" />
          <span>导出备份</span>
          <SvgIcon name="chevron-down" :size="10" class="backup-chevron" :class="{ rotated: showExportMenu }" />
        </button>
        <div v-if="showExportMenu" class="backup-menu">
          <button type="button" @click="showExportMenu = false; onExportBackup()">导出到文件…</button>
          <button type="button" @click="onExportToClipboard">复制到剪贴板</button>
        </div>
      </div>
    </div>
  </div>

  <!-- Main Views Transition -->
  <Transition name="profile-stage-swap" mode="out-in">
    <div :key="editingProfile ? 'editor' : 'stream'" class="profile-stage-shell">
      <!-- 1. Single Stream List (Profiles + Scopes) -->
      <template v-if="!editingProfile">
        <div class="api-stream-container">
          <!-- Section 1: Profiles Library -->
          <ApiProfileList
            :profiles="profiles"
            :profile-fields="profileFields"
            :renaming-profile="renamingProfile"
            v-model:rename-input="renameInput"
            :current-cli-name="currentCli.name"
            :current-settings-path-hint="currentSettingsPathHint"
            :profile-card-hint="profileCardHint"
            :current-cli-id="currentCliId"
            :scope-bindings="scopeBindings"
            :load-error="loadError"
            @card-double-click="handleCardDoubleClick"
            @confirm-rename="confirmRename"
            @cancel-rename="cancelRename"
            @start-rename="startRename"
            @open-editor="openEditor"
            @duplicate="duplicateProfile"
            @delete="requestDeleteProfile"
            @navigate-to-scope="handleNavigateToScope"
            @create="createProfile"
            @set-global="(name) => handleChangeBinding('global', name)"
            @open-raw-settings="showRawEditor = true"
          />

          <!-- Section 2: Scope Allocation (if Claude CLI) -->
          <ApiProfileScopeAllocation
            v-if="isClaude"
            :scope-bindings="scopeBindings"
            :scope-dir-status="scopeDirStatuses"
            :available-profiles="availableProfileSummaries"
            :current-cli-name="currentCli.name"
            :current-cli-id="currentCliId"
            :is-claude="isClaude"
            @change-binding="handleChangeBinding"
            @create-scope="showCreateTabDialog = true"
            @edit-scope="handleEditScope"
            @delete-scope="handleDeleteScope"
            @edit-profile="handleEditProfileFromScope"
            @create-profile-for-scope="handleCreateProfileForScope"
          />
        </div>
      </template>

      <!-- 2. Profile Editor Mode -->
      <template v-else>
        <ApiProfileEditorHeader
          :title="editingName || editingProfile"
          :view="editApiView"
          @back="closeEditor"
          @switch-view="switchEditView"
        />

        <div v-if="editErrorMessage" class="error-banner">{{ editErrorMessage }}</div>

        <Transition name="profile-editor-swap" mode="out-in">
          <div :key="editApiView" class="profile-editor-stage">
            <template v-if="editApiView === 'form'">
              <ApiProfileConnectionSection
                :profile-name="editingName"
                :profile-name-error="profileNameError"
                :is-official="editingProfile === OFFICIAL_PROFILE_NAME"
                :is-claude="isClaude"
                :show-api-key="showApiKey"
                :api-key="editApiKey"
                :api-key-placeholder="apiKeyPlaceholder"
                :base-url="editBaseUrl"
                :base-url-placeholder="isClaude ? 'https://api.anthropic.com' : 'https://api.openai.com/v1'"
                :connection-section-hint="connectionSectionHint"
                :model="editModel"
                :model-placeholder="modelPlaceholder"
                :active-combobox="activeCombobox"
                :grouped-models="groupedModels"
                :filtered-models="filteredModels"
                :price-by-id="priceById"
                :claude-effort-levels="claudeEffortLevels"
                :claude-effort-level="editClaudeEffortLevel"
                :timeout-ms="editTimeoutMs"
                :reasoning-effort="editReasoningEffort"
                :models-loading="modelsLoading"
                :models-error="modelsError"
                @update-profile-name="editingName = $event"
                @update-api-key="editApiKey = $event"
                @toggle-api-key="showApiKey = !showApiKey"
                @update-base-url="editBaseUrl = $event"
                @update-model="editModel = $event"
                @open-combobox="openCombobox"
                @combobox-blur="comboboxBlur"
                @select-model="selectMainModel"
                @update-claude-effort-level="editClaudeEffortLevel = $event"
                @select-claude-effort-level="selectClaudeEffortLevel"
                @update-timeout-ms="editTimeoutMs = $event"
                @update-reasoning-effort="editReasoningEffort = $event"
                @refresh-models="handleRefreshModels"
              />

              <ApiProfileModelOverridesSection
                v-if="isClaude"
                :cli-name="currentCli.name"
                :models-loading="modelsLoading"
                :models-error="modelsError"
                :active-combobox="activeCombobox"
                :grouped-models="groupedModels"
                :filtered-models="filteredModels"
                :price-by-id="priceById"
                :haiku-model="editHaikuModel"
                :haiku-model-name="editHaikuModelName"
                :sonnet-model="editSonnetModel"
                :sonnet-model-name="editSonnetModelName"
                :opus-model="editOpusModel"
                :opus-model-name="editOpusModelName"
                :fallback-model="editModel"
                @refresh-models="handleRefreshModels"
                @update-model="updateDefaultModel"
                @update-model-name="updateDefaultModelName"
                @update-fallback-model="editModel = $event"
                @open-combobox="openCombobox"
                @combobox-blur="comboboxBlur"
                @select-model="selectDefaultModel"
              />

              <ApiProfileRuntimeSection
                v-if="isClaude"
                :max-output-tokens="editClaudeMaxOutputTokens"
                :disable-experimental-betas="editClaudeDisableExperimentalBetas"
                :disable-nonessential-traffic="editClaudeDisableNonessentialTraffic"
                @update-max-output-tokens="editClaudeMaxOutputTokens = $event"
                @update-disable-experimental-betas="editClaudeDisableExperimentalBetas = $event"
                @update-disable-nonessential-traffic="editClaudeDisableNonessentialTraffic = $event"
              />

              <ApiProfileAttributionSection
                v-if="isClaude"
                :mode="editAttributionMode"
                :commit-attribution="editCommitAttribution"
                :pr-attribution="editPrAttribution"
                @update-mode="editAttributionMode = $event"
                @update-commit-attribution="editCommitAttribution = $event"
                @update-pr-attribution="editPrAttribution = $event"
              />

              <section v-else class="settings-section">
                <h3 class="section-title">Codex Provider</h3>
                <p class="section-hint">
                  当前会把模型写入 `model`，并把 Base URL 写到 `model_providers.{{ editCodex.model_provider || 'openai-chat-completions' }}.base_url`。
                </p>
              </section>
            </template>

            <ApiProfileJsonEditor
              v-else
              :error-message="editErrorMessage"
              :json-text="editJsonText"
              @update-json="onEditJsonInput"
            />
          </div>
        </Transition>
      </template>
    </div>
  </Transition>



  <!-- Delete Confirm Dialog -->
  <Transition name="fade">
    <ApiProfileDeleteConfirm
      v-if="deleteConfirmName"
      :name="deleteConfirmName"
      @cancel="cancelDelete"
      @confirm="confirmDeleteProfile"
    />
  </Transition>

  <!-- Scope Create Dialog -->
  <ProfileTabCreateDialog
    v-if="showCreateTabDialog"
    :existing-names="tabs.map((t) => t.name)"
    :cli-id="currentCliId"
    @close="showCreateTabDialog = false"
    @submit="createTab"
  />

  <!-- Scope Delete Confirm Dialog -->
  <ScopeDeleteConfirmDialog
    v-if="deletingTab"
    :name="deletingTab.name"
    @cancel="deletingTab = null"
    @confirm="confirmDeleteTab"
  />

  <!-- Scope Edit Dialog -->
  <ProfileTabEditDialog
    v-if="editingTab"
    :tab="editingTab"
    :existing-names="tabs.map((t) => t.name)"
    @close="editingTab = null"
    @submit="updateTab"
  />

  <!-- Raw Settings Editor Modal -->
  <Transition name="fade">
    <GlobalSettingsRawEditor
      v-if="showRawEditor"
      :cli-id="currentCliId"
      scope="global"
      @close="showRawEditor = false"
      @saved="loadAllProfiles({ refreshActiveEditor: true })"
    />
  </Transition>

  <!-- Backup Import Conflict Dialog -->
  <ProfileImportConflictDialog
    v-if="showImportConflictDialog"
    :candidates="importCandidates"
    :scopes="importedScopeItems"
    :scope-bindings="importedBackupScopeBindings"
    @close="showImportConflictDialog = false"
    @confirm="executeImport"
  />
</template>

<style scoped>
.error-banner {
  padding: var(--space-2) var(--space-3);
  background: rgba(220, 38, 38, 0.1);
  color: var(--color-danger);
  border-radius: var(--radius-md);
  font-size: var(--text-sm);
  margin-bottom: var(--space-3);
}

.api-page-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  margin-bottom: 12px;
  padding-bottom: 10px;
  border-bottom: 1px solid var(--color-border);
  flex-wrap: nowrap;
}

.header-title-wrap {
  display: flex;
  align-items: center;
  gap: 8px;
}

.page-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--color-text);
  margin: 0;
  line-height: 1.2;
}

.header-cli-select-wrap {
  min-width: 120px;
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

.btn-header-tool {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 26px;
  padding: 0 8px;
  border-radius: 6px;
  border: 1px solid var(--color-border);
  background: var(--color-bg);
  color: var(--color-text-secondary);
  font-size: 11px;
  font-weight: 500;
  cursor: pointer;
  white-space: nowrap;
  flex-shrink: 0;
  transition: all var(--transition-fast);
}

.btn-header-tool:hover {
  color: var(--color-primary);
  border-color: var(--color-primary);
  background: var(--color-bg-hover);
}

.backup-menu-wrapper {
  position: relative;
  display: inline-flex;
}

.backup-chevron {
  transition: transform var(--transition-fast);
}

.backup-chevron.rotated {
  transform: rotate(180deg);
}

.backup-menu {
  position: absolute;
  top: calc(100% + 4px);
  right: 0;
  z-index: 100;
  background-color: var(--color-surface-elevated, var(--color-bg));
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md, 8px);
  padding: 4px;
  display: flex;
  flex-direction: column;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.16);
  min-width: 128px;
  white-space: nowrap;
}

.backup-menu button {
  background: none;
  border: none;
  padding: 6px 12px;
  text-align: left;
  border-radius: var(--radius-sm);
  color: var(--color-text-secondary);
  cursor: pointer;
  font-size: 12px;
  white-space: nowrap;
  transition: background-color var(--transition-fast), color var(--transition-fast);
}

.backup-menu button:hover {
  background-color: var(--color-surface-hover, var(--color-bg-hover));
  color: var(--color-text);
}

.api-stream-container {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.profile-bottom-hint {
  font-size: 11px;
  color: var(--color-text-muted);
  margin-top: 10px;
  line-height: 1.5;
  padding-top: 8px;
  border-top: 1px solid var(--color-border);
}

.settings-link-wrapper {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  color: var(--color-text);
  font-family: var(--font-mono, monospace);
  background: var(--color-bg-secondary);
  padding: 1px 6px;
  border-radius: 4px;
  border: 1px solid var(--color-border);
}

.open-file-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  border: none;
  background: transparent;
  color: var(--color-text-muted);
  cursor: pointer;
  border-radius: 2px;
}

.open-file-btn:hover {
  color: var(--color-primary);
  background: var(--color-bg-hover);
}

.btn-icon-action {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  padding: 0;
  border-radius: 6px;
  border: 1px solid var(--color-border);
  background: var(--color-bg);
  color: var(--color-text-muted);
  cursor: pointer;
  white-space: nowrap;
  flex-shrink: 0;
  transition: all var(--transition-fast);
}

.btn-icon-action:hover {
  color: var(--color-primary);
  border-color: var(--color-primary);
  background: var(--color-bg-hover);
}

.profile-stage-shell {
  min-height: 240px;
}
.profile-editor-stage {
  min-height: 220px;
}
.profile-stage-swap-enter-active,
.profile-stage-swap-leave-active,
.profile-editor-swap-enter-active,
.profile-editor-swap-leave-active {
  transition: opacity 180ms ease, transform 220ms ease;
}
.profile-stage-swap-enter-from,
.profile-stage-swap-leave-to,
.profile-editor-swap-enter-from,
.profile-editor-swap-leave-to {
  opacity: 0;
  transform: translateY(8px);
}
.settings-section {
  margin-bottom: var(--space-5);
}
.section-title {
  font-size: var(--text-sm);
  font-weight: 600;
  margin: 0 0 var(--space-2);
  color: var(--color-text);
}
.section-hint {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  margin: 0 0 var(--space-3);
}

.fade-enter-active, .fade-leave-active {
  transition: opacity 0.15s;
}
.fade-enter-from, .fade-leave-to {
  opacity: 0;
}

.fetch-model-success-toast {
  position: absolute;
  top: 16px;
  left: 50%;
  transform: translateX(-50%);
  z-index: 1000;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 16px;
  background-color: color-mix(in srgb, var(--color-success) 15%, var(--color-bg));
  color: var(--color-success);
  border: 1px solid color-mix(in srgb, var(--color-success) 30%, var(--color-border));
  border-radius: var(--radius-md);
  font-size: var(--text-sm);
  font-weight: 500;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.08);
}
:root[data-theme="dark"] .fetch-model-success-toast {
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.2);
}
.fetch-model-success-toast svg {
  color: var(--color-success);
}
.model-toast-enter-active,
.model-toast-leave-active {
  transition: all var(--transition-slow);
}
.model-toast-enter-from,
.model-toast-leave-to {
  opacity: 0;
  transform: translate(-50%, -10px);
}
</style>
