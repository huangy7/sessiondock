import type { PtyStatus } from "../types/pty";
import type { PetBaseState } from "./petStateMachine";

export type { PetBaseState };

export interface SessionStatusLike {
  sessionId: string;
  status: PtyStatus;
}

export interface AggregatorCallbacks {
  onBaseChange(base: PetBaseState): void;
  onCelebrate(): void;
}

export interface SessionStateAggregator {
  readonly base: PetBaseState;
  /** 应用单条 pty-status-changed 事件 */
  applyStatus(sessionId: string, status: PtyStatus): void;
  /** 应用启动快照（list_pty_sessions），清空后重建 */
  applySnapshot(sessions: SessionStatusLike[]): void;
}

/**
 * 会话状态聚合：把多个 PTY 会话的状态折叠为桌宠唯一基础态。
 * 优先级 attention > working > idle；Active/WaitingInput → Exited 跃迁触发庆祝。
 */
export function createSessionStateAggregator(
  callbacks: AggregatorCallbacks,
): SessionStateAggregator {
  const sessions = new Map<string, PtyStatus>();
  let currentBase: PetBaseState = "idle";

  function computeBase(): PetBaseState {
    let hasActive = false;
    for (const status of sessions.values()) {
      if (status === "waiting_input") return "attention";
      if (status === "active") hasActive = true;
    }
    return hasActive ? "working" : "idle";
  }

  function recompute(): void {
    const next = computeBase();
    if (next !== currentBase) {
      currentBase = next;
      callbacks.onBaseChange(next);
    }
  }

  return {
    get base() {
      return currentBase;
    },
    applyStatus(sessionId, status) {
      const prev = sessions.get(sessionId);
      if (status === "exited") {
        if (prev === undefined) return;
        const celebrate = prev === "active" || prev === "waiting_input";
        sessions.delete(sessionId);
        recompute();
        // 庆祝放最后：回调抛异常时 Map 已一致，且回调读到的是最新 base
        if (celebrate) {
          callbacks.onCelebrate();
        }
        return;
      }
      if (prev === status) return;
      sessions.set(sessionId, status);
      recompute();
    },
    applySnapshot(list) {
      sessions.clear();
      for (const s of list) {
        if (s.status !== "exited") {
          sessions.set(s.sessionId, s.status);
        }
      }
      recompute();
    },
  };
}
