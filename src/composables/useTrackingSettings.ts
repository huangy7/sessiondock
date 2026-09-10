import { computed, ref } from "vue";

export type TrackingSettings = {
  enabled: boolean;
  teamId: number | null;
  sprintId: number | null;
  pageSize: number;
};

const TRACKING_SETTINGS_KEY = "claudia:nps-tracking-settings:v1";
const LEGACY_CONFIG_KEY = "claudia:nps-tracking-config:v1";

function readNumber(value: unknown): number | null {
  return Number.isFinite(value) ? Number(value) : null;
}

function loadSettings(): TrackingSettings {
  try {
    const raw = localStorage.getItem(TRACKING_SETTINGS_KEY) ?? localStorage.getItem(LEGACY_CONFIG_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      return {
        enabled: parsed.enabled === true,
        teamId: readNumber(parsed.teamId),
        sprintId: readNumber(parsed.sprintId),
        pageSize: Number.isFinite(parsed.pageSize) ? Number(parsed.pageSize) : 80,
      };
    }
  } catch {
    // Ignore corrupted local settings.
  }

  return {
    enabled: false,
    teamId: null,
    sprintId: null,
    pageSize: 80,
  };
}

const settings = ref<TrackingSettings>(loadSettings());

function saveTrackingSettings() {
  localStorage.setItem(TRACKING_SETTINGS_KEY, JSON.stringify(settings.value));
}

function updateTrackingSettings(patch: Partial<TrackingSettings>) {
  settings.value = {
    ...settings.value,
    ...patch,
  };
  saveTrackingSettings();
}

const trackingInvokeConfig = computed(() => ({
  teamId: settings.value.teamId,
  sprintId: settings.value.sprintId,
  pageSize: settings.value.pageSize || 80,
}));

export function useTrackingSettings() {
  return {
    trackingSettings: settings,
    trackingInvokeConfig,
    saveTrackingSettings,
    updateTrackingSettings,
  };
}
