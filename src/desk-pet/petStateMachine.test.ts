import { describe, expect, it } from "vitest";
import {
  createClickAccumulator,
  createPetStateMachine,
  type PetState,
} from "./petStateMachine";

/** 手动触发的假定时器：收集回调，fireAll 时执行未取消的 */
function makeFakeTimers() {
  const pending: Array<{ cb: () => void; ms: number; cancelled: boolean }> = [];
  return {
    pending,
    setTimeoutFn: (cb: () => void, ms: number) => {
      const handle = { cb, ms, cancelled: false };
      pending.push(handle);
      return handle;
    },
    clearTimeoutFn: (handle: unknown) => {
      (handle as { cancelled: boolean }).cancelled = true;
    },
    fireAll: () => {
      const batch = pending.splice(0, pending.length);
      batch.forEach((h) => {
        if (!h.cancelled) h.cb();
      });
    },
  };
}

function makeMachine(overrides: {
  random?: () => number;
} = {}) {
  const timers = makeFakeTimers();
  const seen: PetState[] = [];
  const machine = createPetStateMachine({
    durations: {
      "poke-left": 2500,
      "poke-right": 2500,
      annoyed: 3500,
      flail: 3500,
      happy: 4000,
    },
    random: overrides.random ?? (() => 0.9), // 默认走 poke 分支
    setTimeoutFn: timers.setTimeoutFn,
    clearTimeoutFn: timers.clearTimeoutFn,
  });
  machine.onChange((s) => seen.push(s));
  return { machine, timers, seen };
}

describe("createPetStateMachine", () => {
  it("初始状态为 idle", () => {
    const { machine } = makeMachine();
    expect(machine.state).toBe("idle");
  });

  it("单击（count=1）不改变状态", () => {
    const { machine } = makeMachine();
    machine.dispatch({ type: "click", count: 1, side: "left" });
    expect(machine.state).toBe("idle");
  });

  it("双击按半区触发 poke，duration 到点自动回 idle", () => {
    const { machine, timers, seen } = makeMachine();
    machine.dispatch({ type: "click", count: 2, side: "right" });
    expect(machine.state).toBe("poke-right");
    expect(seen).toEqual(["poke-right"]);
    expect(timers.pending[0]?.ms).toBe(2500);
    timers.fireAll();
    expect(machine.state).toBe("idle");
  });

  it("双击随机进入 annoyed（random < 0.3）", () => {
    const { machine } = makeMachine({ random: () => 0.1 });
    machine.dispatch({ type: "click", count: 2, side: "left" });
    expect(machine.state).toBe("annoyed");
  });

  it("4 连击触发 flail", () => {
    const { machine } = makeMachine();
    machine.dispatch({ type: "click", count: 4, side: "left" });
    expect(machine.state).toBe("flail");
  });

  it("oneshot 期间新点击可打断，旧定时器失效不回退", () => {
    const { machine, timers } = makeMachine();
    machine.dispatch({ type: "click", count: 2, side: "left" });
    expect(machine.state).toBe("poke-left");
    machine.dispatch({ type: "click", count: 2, side: "right" });
    expect(machine.state).toBe("poke-right");
    timers.fireAll(); // 两个定时器都在，但旧的已被 token 判死
    expect(machine.state).toBe("idle");
  });

  it("拖拽立即中断 oneshot，且取消自动回退定时器", () => {
    const { machine, timers } = makeMachine();
    machine.dispatch({ type: "click", count: 2, side: "left" });
    machine.dispatch({ type: "dragStart", direction: "left" });
    expect(machine.state).toBe("drag-left");
    timers.fireAll();
    expect(machine.state).toBe("drag-left"); // 不被旧定时器打回 idle
  });

  it("dragMove 按方向切换 drag-left / drag-right", () => {
    const { machine } = makeMachine();
    machine.dispatch({ type: "dragStart", direction: "right" });
    expect(machine.state).toBe("drag-right");
    machine.dispatch({ type: "dragMove", direction: "left" });
    expect(machine.state).toBe("drag-left");
  });

  it("dragEnd 回 idle；非拖拽态收到 dragEnd 无影响", () => {
    const { machine } = makeMachine();
    machine.dispatch({ type: "dragStart", direction: "left" });
    machine.dispatch({ type: "dragEnd" });
    expect(machine.state).toBe("idle");
    machine.dispatch({ type: "click", count: 2, side: "left" });
    machine.dispatch({ type: "dragEnd" }); // oneshot 中的 dragEnd 不应打断
    expect(machine.state).toBe("poke-left");
  });

  it("拖拽中忽略点击事件", () => {
    const { machine } = makeMachine();
    machine.dispatch({ type: "dragStart", direction: "left" });
    machine.dispatch({ type: "click", count: 4, side: "left" });
    expect(machine.state).toBe("drag-left");
  });

  it("idle 状态收到 oneshotEnd 无副作用", () => {
    const { machine } = makeMachine();
    machine.dispatch({ type: "oneshotEnd" });
    expect(machine.state).toBe("idle");
  });

  it("同状态 oneshot 重复触发：重武装定时器并再次通知", () => {
    const { machine, timers, seen } = makeMachine();
    machine.dispatch({ type: "click", count: 2, side: "left" });
    expect(machine.state).toBe("poke-left");
    expect(timers.pending.length).toBe(1);
    const firstTimer = timers.pending[0];

    machine.dispatch({ type: "click", count: 2, side: "left" });
    expect(machine.state).toBe("poke-left");
    // 旧定时器被取消，新定时器重新武装
    expect(firstTimer?.cancelled).toBe(true);
    expect(timers.pending.length).toBe(2);
    // listener 收到第二次通知（视图可重启动画）
    expect(seen).toEqual(["poke-left", "poke-left"]);
    // 从新计时点回 idle（旧定时器已死，不会提前打断）
    timers.fireAll();
    expect(machine.state).toBe("idle");
  });

  it("同状态非 oneshot 重复触发仍是 no-op", () => {
    const { machine, seen } = makeMachine();
    machine.dispatch({ type: "dragStart", direction: "left" });
    machine.dispatch({ type: "dragMove", direction: "left" });
    expect(machine.state).toBe("drag-left");
    expect(seen).toEqual(["drag-left"]);
  });

  it("onChange 返回的退订函数可单独移除 listener", () => {
    const { machine, seen } = makeMachine();
    const extra: PetState[] = [];
    const unsubscribe = machine.onChange((s) => extra.push(s));
    machine.dispatch({ type: "click", count: 2, side: "left" });
    expect(extra).toEqual(["poke-left"]);
    unsubscribe();
    machine.dispatch({ type: "click", count: 2, side: "right" });
    expect(extra).toEqual(["poke-left"]); // 不再收到
    expect(seen).toEqual(["poke-left", "poke-right"]); // 其他 listener 不受影响
  });

  it("dispose 后定时器取消，不再回退", () => {
    const { machine, timers } = makeMachine();
    machine.dispatch({ type: "click", count: 2, side: "left" });
    machine.dispose();
    timers.fireAll();
    expect(machine.state).toBe("poke-left");
  });

  it("idle 收到 dizzy 事件进入 dizzy，duration 到点自动回 idle", () => {
    const timers = makeFakeTimers();
    const seen: PetState[] = [];
    const machine = createPetStateMachine({
      durations: { dizzy: 6000 },
      random: () => 0.9,
      setTimeoutFn: timers.setTimeoutFn,
      clearTimeoutFn: timers.clearTimeoutFn,
    });
    machine.onChange((s) => seen.push(s));
    machine.dispatch({ type: "dizzy" });
    expect(machine.state).toBe("dizzy");
    expect(seen).toEqual(["dizzy"]);
    expect(timers.pending[0]?.ms).toBe(6000);
    timers.fireAll();
    expect(machine.state).toBe("idle");
  });

  it("非 idle 状态收到 dizzy 事件被忽略", () => {
    const { machine } = makeMachine();
    machine.dispatch({ type: "click", count: 2, side: "left" });
    machine.dispatch({ type: "dizzy" });
    expect(machine.state).toBe("poke-left");
    machine.dispatch({ type: "dragStart", direction: "left" });
    machine.dispatch({ type: "dizzy" });
    expect(machine.state).toBe("drag-left");
  });

  it("dizzy 中拖拽立即中断并取消自动回退定时器", () => {
    const timers = makeFakeTimers();
    const machine = createPetStateMachine({
      durations: { dizzy: 6000 },
      random: () => 0.9,
      setTimeoutFn: timers.setTimeoutFn,
      clearTimeoutFn: timers.clearTimeoutFn,
    });
    machine.dispatch({ type: "dizzy" });
    expect(machine.state).toBe("dizzy");
    machine.dispatch({ type: "dragStart", direction: "right" });
    expect(machine.state).toBe("drag-right");
    timers.fireAll();
    expect(machine.state).toBe("drag-right"); // 不被旧定时器打回 idle
  });
});

describe("双层模型：基础态 + 覆盖层", () => {
  it("setBaseState 无覆盖时立即切换", () => {
    const { machine } = makeMachine();
    machine.setBaseState("working");
    expect(machine.state).toBe("working");
    expect(machine.baseState).toBe("working");
    machine.setBaseState("attention");
    expect(machine.state).toBe("attention");
  });

  it("oneshot 覆盖中 setBaseState 仅记录，播完回最新基础态", () => {
    const { machine, timers } = makeMachine();
    machine.dispatch({ type: "click", count: 2, side: "left" });
    expect(machine.state).toBe("poke-left");
    machine.setBaseState("working");
    expect(machine.state).toBe("poke-left"); // 不打扰覆盖层
    machine.setBaseState("attention"); // 中途又变一次
    timers.fireAll();
    expect(machine.state).toBe("attention"); // 回最新基础态，不是 idle
  });

  it("拖拽结束回最新基础态", () => {
    const { machine } = makeMachine();
    machine.setBaseState("working");
    machine.dispatch({ type: "dragStart", direction: "left" });
    machine.setBaseState("idle");
    machine.dispatch({ type: "dragEnd" });
    expect(machine.state).toBe("idle");
  });

  it("happy oneshot：播完回基础态", () => {
    const { machine, timers } = makeMachine();
    machine.setBaseState("working");
    machine.dispatch({ type: "happy" });
    expect(machine.state).toBe("happy");
    expect(timers.pending[0]?.ms).toBe(4000);
    timers.fireAll();
    expect(machine.state).toBe("working");
  });

  it("拖拽中忽略 happy 事件", () => {
    const { machine } = makeMachine();
    machine.dispatch({ type: "dragStart", direction: "left" });
    machine.dispatch({ type: "happy" });
    expect(machine.state).toBe("drag-left");
  });

  it("happy 可被新 oneshot 覆盖、被拖拽中断", () => {
    const { machine } = makeMachine();
    machine.dispatch({ type: "happy" });
    machine.dispatch({ type: "click", count: 2, side: "right" });
    expect(machine.state).toBe("poke-right");
    machine.dispatch({ type: "happy" });
    machine.dispatch({ type: "dragStart", direction: "left" });
    expect(machine.state).toBe("drag-left");
  });

  it("基础态默认 idle，既有 oneshot 回 idle 行为兼容", () => {
    const { machine, timers } = makeMachine();
    machine.dispatch({ type: "click", count: 2, side: "left" });
    timers.fireAll();
    expect(machine.state).toBe("idle");
  });
});

describe("createClickAccumulator", () => {
  function makeAccumulator() {
    const timers = makeFakeTimers();
    const reactions: number[] = [];
    const acc = createClickAccumulator({
      windowMs: 400,
      onReaction: (count) => reactions.push(count),
      setTimeoutFn: timers.setTimeoutFn,
      clearTimeoutFn: timers.clearTimeoutFn,
    });
    return { acc, timers, reactions };
  }

  it("双击在 400ms 静默期后触发 poke（count=2）", () => {
    const { acc, timers, reactions } = makeAccumulator();
    acc.registerClick();
    acc.registerClick();
    expect(reactions).toEqual([]); // 未立即触发
    timers.fireAll();
    expect(reactions).toEqual([2]);
  });

  it("间隔 ≤400ms 的连续点击持续累计，不限总跨度", () => {
    let now = 0;
    const timers = makeFakeTimers();
    const reactions: number[] = [];
    const acc = createClickAccumulator({
      windowMs: 400,
      onReaction: (c) => reactions.push(c),
      setTimeoutFn: timers.setTimeoutFn,
      clearTimeoutFn: timers.clearTimeoutFn,
      nowFn: () => now,
    });
    acc.registerClick(); // t=0
    now = 350;
    acc.registerClick(); // 间隔 350
    now = 700;
    acc.registerClick(); // 间隔 350，总跨度 700 > 400 仍累计
    timers.fireAll();
    expect(reactions).toEqual([3]);
  });

  it("间隔超 400ms 重新计数", () => {
    let now = 0;
    const timers = makeFakeTimers();
    const reactions: number[] = [];
    const acc = createClickAccumulator({
      windowMs: 400,
      onReaction: (c) => reactions.push(c),
      setTimeoutFn: timers.setTimeoutFn,
      clearTimeoutFn: timers.clearTimeoutFn,
      nowFn: () => now,
    });
    acc.registerClick(); // t=0
    now = 500;
    acc.registerClick(); // 间隔 500 > 400 → 重新从 1 计
    now = 600;
    acc.registerClick(); // 间隔 100 → count=2
    timers.fireAll();
    expect(reactions).toEqual([2]);
  });

  it("第 4 击立即触发 flail，不等静默期", () => {
    const { acc, timers, reactions } = makeAccumulator();
    acc.registerClick();
    acc.registerClick();
    acc.registerClick();
    expect(reactions).toEqual([]);
    acc.registerClick();
    expect(reactions).toEqual([4]); // 立即
    timers.fireAll();
    expect(reactions).toEqual([4]); // 无重复触发
  });

  it("reset 取消待触发的判定", () => {
    const { acc, timers, reactions } = makeAccumulator();
    acc.registerClick();
    acc.registerClick();
    acc.reset();
    timers.fireAll();
    expect(reactions).toEqual([]);
  });

  it("dispose 后不再触发", () => {
    const { acc, timers, reactions } = makeAccumulator();
    acc.registerClick();
    acc.registerClick();
    acc.dispose();
    timers.fireAll();
    expect(reactions).toEqual([]);
  });
});
