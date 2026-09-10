export type PetBaseState = "idle" | "working" | "attention";

export type PetState =
  | PetBaseState
  | "poke-left"
  | "poke-right"
  | "annoyed"
  | "flail"
  | "dizzy"
  | "happy"
  | "drag-left"
  | "drag-right";

export type PetEvent =
  | { type: "click"; count: number; side: "left" | "right" }
  | { type: "dragStart"; direction: "left" | "right" }
  | { type: "dragMove"; direction: "left" | "right" }
  | { type: "dragEnd" }
  | { type: "oneshotEnd" }
  | { type: "dizzy" }
  | { type: "happy" };

const BASE_STATES: ReadonlySet<PetState> = new Set(["idle", "working", "attention"]);

const ONESHOT_STATES: ReadonlySet<PetState> = new Set([
  "poke-left",
  "poke-right",
  "annoyed",
  "flail",
  "dizzy",
  "happy",
]);

const DRAG_STATES: ReadonlySet<PetState> = new Set(["drag-left", "drag-right"]);

/** 双击反应中 annoyed 出现的概率，其余按点击半区 poke */
const ANNOYED_PROBABILITY = 0.3;

export interface PetMachineOptions {
  /** oneshot 状态展示时长（ms），按状态名索引 */
  durations: Partial<Record<PetState, number>>;
  random?: () => number;
  setTimeoutFn?: (cb: () => void, ms: number) => unknown;
  clearTimeoutFn?: (handle: unknown) => void;
}

export interface PetStateMachine {
  readonly state: PetState;
  readonly baseState: PetBaseState;
  dispatch(event: PetEvent): void;
  /** 外部驱动的基础态切换：无覆盖层时立即生效，覆盖中仅记录 */
  setBaseState(state: PetBaseState): void;
  /** 订阅状态变化；返回退订函数，可单独移除该 listener */
  onChange(listener: (state: PetState) => void): () => void;
  dispose(): void;
}

export function createPetStateMachine(options: PetMachineOptions): PetStateMachine {
  const random = options.random ?? Math.random;
  const setTimeoutFn =
    options.setTimeoutFn ?? ((cb: () => void, ms: number) => setTimeout(cb, ms));
  const clearTimeoutFn =
    options.clearTimeoutFn ?? ((handle: unknown) => clearTimeout(handle as number));

  let current: PetState = "idle";
  let base: PetBaseState = "idle";
  let timer: unknown = null;
  let timerToken = 0;
  let disposed = false;
  const listeners = new Set<(state: PetState) => void>();

  function cancelTimer() {
    timerToken += 1;
    if (timer !== null) {
      clearTimeoutFn(timer);
      timer = null;
    }
  }

  function enter(next: PetState) {
    if (disposed) return;
    // 同状态：仅 oneshot 需要重武装定时器并再次通知（让视图重启动画），其余 no-op
    if (next === current && !ONESHOT_STATES.has(next)) return;
    cancelTimer();
    current = next;
    if (ONESHOT_STATES.has(next)) {
      const token = timerToken;
      const ms = options.durations[next] ?? 2500;
      timer = setTimeoutFn(() => {
        if (token !== timerToken || disposed) return;
        dispatch({ type: "oneshotEnd" });
      }, ms);
    }
    listeners.forEach((l) => l(next));
  }

  function dispatch(event: PetEvent): void {
    if (disposed) return;
    switch (event.type) {
      case "click": {
        if (DRAG_STATES.has(current)) return;
        if (event.count >= 4) {
          enter("flail");
        } else if (event.count >= 2) {
          if (random() < ANNOYED_PROBABILITY) {
            enter("annoyed");
          } else {
            enter(event.side === "left" ? "poke-left" : "poke-right");
          }
        }
        return;
      }
      case "dragStart":
      case "dragMove": {
        enter(event.direction === "left" ? "drag-left" : "drag-right");
        return;
      }
      case "dragEnd": {
        if (DRAG_STATES.has(current)) enter(base);
        return;
      }
      case "oneshotEnd": {
        if (ONESHOT_STATES.has(current)) enter(base);
        return;
      }
      case "dizzy": {
        // 仅普通 idle 可进入头晕（对齐原项目：mini 与反应态不触发）
        if (current === "idle") enter("dizzy");
        return;
      }
      case "happy": {
        if (DRAG_STATES.has(current)) return;
        enter("happy");
        return;
      }
    }
  }

  return {
    get state() {
      return current;
    },
    get baseState() {
      return base;
    },
    dispatch,
    setBaseState(next) {
      if (disposed) return;
      base = next;
      if (BASE_STATES.has(current)) {
        enter(next);
      }
    },
    onChange(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    dispose() {
      disposed = true;
      cancelTimer();
      listeners.clear();
    },
  };
}

export interface ClickAccumulatorOptions {
  /** 相邻点击的最大间隔（ms），以及 2~3 击后的静默判定期 */
  windowMs?: number;
  /** 反应回调：count>=4 立即调用；count 2~3 在静默期后调用 */
  onReaction: (count: number) => void;
  nowFn?: () => number;
  setTimeoutFn?: (cb: () => void, ms: number) => unknown;
  clearTimeoutFn?: (handle: unknown) => void;
}

export interface ClickAccumulator {
  registerClick(): void;
  reset(): void;
  dispose(): void;
}

/**
 * 间隔制点击累计（对齐原项目语义）：
 * - 相邻点击间隔 ≤windowMs 即累计（不限总跨度）
 * - count>=4 立即触发（flail）
 * - count 2~3 等 windowMs 静默期后触发（poke/annoyed），期间新点击重新武装定时器
 */
export function createClickAccumulator(options: ClickAccumulatorOptions): ClickAccumulator {
  const windowMs = options.windowMs ?? 400;
  const nowFn = options.nowFn ?? (() => Date.now());
  const setTimeoutFn =
    options.setTimeoutFn ?? ((cb: () => void, ms: number) => setTimeout(cb, ms));
  const clearTimeoutFn =
    options.clearTimeoutFn ?? ((handle: unknown) => clearTimeout(handle as number));

  let count = 0;
  let lastClickAt: number | null = null;
  let timer: unknown = null;
  let disposed = false;

  function clearTimer() {
    if (timer !== null) {
      clearTimeoutFn(timer);
      timer = null;
    }
  }

  return {
    registerClick() {
      if (disposed) return;
      const now = nowFn();
      if (lastClickAt !== null && now - lastClickAt > windowMs) {
        count = 0; // 间隔超窗，重新计数
      }
      lastClickAt = now;
      count += 1;
      clearTimer();
      if (count >= 4) {
        count = 0;
        lastClickAt = null;
        options.onReaction(4);
        return;
      }
      timer = setTimeoutFn(() => {
        timer = null;
        if (disposed) return;
        const finalCount = count;
        count = 0;
        lastClickAt = null;
        if (finalCount >= 2) {
          options.onReaction(finalCount);
        }
      }, windowMs);
    },
    reset() {
      count = 0;
      lastClickAt = null;
      clearTimer();
    },
    dispose() {
      disposed = true;
      count = 0;
      clearTimer();
    },
  };
}
