import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

export type AutoIndexInterval = "off" | "5m" | "10m" | "30m";

export const AUTO_INDEX_INTERVAL_MAP: Record<AutoIndexInterval, number> = {
  off: 0,
  "5m": 5 * 60 * 1000,
  "10m": 10 * 60 * 1000,
  "30m": 30 * 60 * 1000,
};

export const AUTO_INDEX_OPTIONS: { value: AutoIndexInterval; label: string }[] = [
  { value: "off", label: "关闭" },
  { value: "5m", label: "每 5 分钟" },
  { value: "10m", label: "每 10 分钟" },
  { value: "30m", label: "每 30 分钟" },
];

const STORAGE_KEY = "claudia-auto-index-interval";

function getStoredInterval(): AutoIndexInterval {
  const val = localStorage.getItem(STORAGE_KEY);
  if (val === "off" || val === "5m" || val === "10m" || val === "30m") {
    return val;
  }
  return "off"; // 默认关闭，避免升级或首次启动造成 CPU/IO 峰值卡顿
}

const autoIndexInterval = ref<AutoIndexInterval>(getStoredInterval());
const isSyncing = ref(false);
let schedulerTimer: ReturnType<typeof setTimeout> | null = null;
let activeGetVisibleCliIds: (() => string[]) | null = null;
let activeIsBusy: (() => boolean) | null = null;

function setAutoIndexInterval(interval: AutoIndexInterval) {
  autoIndexInterval.value = interval;
  localStorage.setItem(STORAGE_KEY, interval);
  resetScheduler();
}

interface SearchIndexStatusCheck {
  ready: boolean;
  requiresConfirmation: boolean;
  missingSessions: number;
}

// 允许后台静默补齐的最大未索引会话数；超过此数量说明有较多积压，坚决不在后台静默全量跑，避免争抢用户 CPU/IO
export const SILENT_INCREMENTAL_MAX_SESSIONS = 3;

async function runSilentIncrementalSync(cliIds: string[]) {
  if (isSyncing.value || cliIds.length === 0 || autoIndexInterval.value === "off") return;
  isSyncing.value = true;
  try {
    for (const cliId of cliIds) {
      try {
        const status = await invoke<SearchIndexStatusCheck>("get_search_index_status", { cliId });
        if (!status.ready) {
          // 若需要用户确认或缺失会话超过轻量阈值，直接跳过，留待用户主动搜索时确认
          if (status.requiresConfirmation || status.missingSessions > SILENT_INCREMENTAL_MAX_SESSIONS) {
            continue;
          }
        }
        await invoke("ensure_search_index_ready", { cliId });
      } catch (err) {
        // 静默运行，不打扰用户
        console.warn(`[AutoIndex] 静默增量同步 ${cliId} 失败:`, err);
      }
    }
  } finally {
    isSyncing.value = false;
  }
}

function scheduleNext() {
  if (schedulerTimer) {
    clearTimeout(schedulerTimer);
    schedulerTimer = null;
  }

  const intervalMs = AUTO_INDEX_INTERVAL_MAP[autoIndexInterval.value];
  if (intervalMs <= 0) return;

  schedulerTimer = setTimeout(async () => {
    if (activeIsBusy && activeIsBusy()) {
      // 繁忙中跳过本轮，顺延调度
      scheduleNext();
      return;
    }

    if (activeGetVisibleCliIds) {
      const ids = activeGetVisibleCliIds();
      await runSilentIncrementalSync(ids);
    }

    scheduleNext();
  }, intervalMs);
}

function resetScheduler() {
  if (schedulerTimer) {
    clearTimeout(schedulerTimer);
    schedulerTimer = null;
  }
  scheduleNext();
}

export function useAutoIndex() {
  function startAutoIndexScheduler(
    getVisibleCliIds: () => string[],
    isBusy: () => boolean,
  ) {
    activeGetVisibleCliIds = getVisibleCliIds;
    activeIsBusy = isBusy;
    resetScheduler();
  }

  function stopAutoIndexScheduler() {
    if (schedulerTimer) {
      clearTimeout(schedulerTimer);
      schedulerTimer = null;
    }
    activeGetVisibleCliIds = null;
    activeIsBusy = null;
  }

  return {
    autoIndexInterval,
    setAutoIndexInterval,
    isSyncing,
    startAutoIndexScheduler,
    stopAutoIndexScheduler,
    runSilentIncrementalSync,
  };
}
