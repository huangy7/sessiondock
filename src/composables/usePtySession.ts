import { ref, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { PtySessionInfo, PtyStatusChangedPayload, AgentStatusMode } from "../types/pty";
import { basename } from "../utils/projectPath";

// Module-scope singleton state
const sessions = ref<PtySessionInfo[]>([]);
const customLabels = ref<Record<string, string>>({});
let statusListener: UnlistenFn | null = null;

function getSessionLabel(sessionId: string, projectPath: string): string {
  return customLabels.value[sessionId] ?? basename(projectPath) ?? projectPath;
}

function setSessionLabel(sessionId: string, label: string) {
  customLabels.value[sessionId] = label;
}

function ensureStatusListener() {
  if (statusListener) return;
  listen<PtyStatusChangedPayload>("pty-status-changed", (event) => {
    const { sessionId, status } = event.payload;
    const session = sessions.value.find((s) => s.sessionId === sessionId);
    if (session) {
      session.status = status;
    }
  }).then((unlisten) => {
    statusListener = unlisten;
  });
}

function closeStatusListener() {
  if (statusListener) {
    statusListener();
    statusListener = null;
  }
}

const waitingInputCount = computed(() =>
  sessions.value.filter((s) => s.status === "waiting_input").length
);

async function refreshSessions() {
  const list = await invoke<PtySessionInfo[]>("list_pty_sessions");
  sessions.value = list;
}

async function createSession(
  projectPath: string,
  cliKind: string,
  sessionId?: string,
  resumeSessionId?: string,
  skipPermissions?: boolean,
  profileName?: string | null,
  agentStatusMode?: AgentStatusMode,
): Promise<PtySessionInfo> {
  const info = await invoke<PtySessionInfo>("create_pty_session", {
    projectPath,
    cliKind,
    sessionId,
    resumeSessionId,
    skipPermissions,
    profileName: profileName || undefined,
    agentStatusMode,
  });
  sessions.value.push(info);
  return info;
}

async function closeSession(sessionId: string): Promise<void> {
  try {
    await invoke("close_pty_session", { sessionId });
  } catch {
    // Guardian thread may have already removed the session after process exit
  }
  sessions.value = sessions.value.filter((s) => s.sessionId !== sessionId);
}

function removeExitedSession(sessionId: string) {
  sessions.value = sessions.value.filter((s) => s.sessionId !== sessionId);
}

export function usePtySession() {
  ensureStatusListener();
  return {
    sessions,
    customLabels,
    getSessionLabel,
    setSessionLabel,
    waitingInputCount,
    refreshSessions,
    createSession,
    closeSession,
    removeExitedSession,
    closeStatusListener,
  };
}
