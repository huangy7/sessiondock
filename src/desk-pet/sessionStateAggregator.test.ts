import { describe, expect, it } from "vitest";
import {
  createSessionStateAggregator,
  type PetBaseState,
} from "./sessionStateAggregator";

function makeAggregator() {
  const bases: PetBaseState[] = [];
  let celebrations = 0;
  const agg = createSessionStateAggregator({
    onBaseChange: (b) => bases.push(b),
    onCelebrate: () => {
      celebrations += 1;
    },
  });
  return { agg, bases, celebrate: () => celebrations };
}

describe("createSessionStateAggregator", () => {
  it("初始基础态为 idle", () => {
    const { agg } = makeAggregator();
    expect(agg.base).toBe("idle");
  });

  it("单会话跃迁：idle→working→attention→idle", () => {
    const { agg, bases } = makeAggregator();
    agg.applyStatus("s1", "active");
    expect(agg.base).toBe("working");
    agg.applyStatus("s1", "waiting_input");
    expect(agg.base).toBe("attention");
    agg.applyStatus("s1", "idle");
    expect(agg.base).toBe("idle");
    expect(bases).toEqual(["working", "attention", "idle"]);
  });

  it("多会话优先级：attention > working > idle", () => {
    const { agg } = makeAggregator();
    agg.applyStatus("s1", "active");
    agg.applyStatus("s2", "active");
    expect(agg.base).toBe("working");
    agg.applyStatus("s2", "waiting_input");
    expect(agg.base).toBe("attention");
    agg.applyStatus("s2", "idle");
    expect(agg.base).toBe("working"); // s1 仍 active
    agg.applyStatus("s1", "idle");
    expect(agg.base).toBe("idle");
  });

  it("Active→Exited 触发庆祝并移除会话", () => {
    const { agg, celebrate } = makeAggregator();
    agg.applyStatus("s1", "active");
    agg.applyStatus("s1", "exited");
    expect(celebrate()).toBe(1);
    expect(agg.base).toBe("idle");
  });

  it("WaitingInput→Exited 触发庆祝", () => {
    const { agg, celebrate } = makeAggregator();
    agg.applyStatus("s1", "waiting_input");
    agg.applyStatus("s1", "exited");
    expect(celebrate()).toBe(1);
  });

  it("Idle→Exited 不庆祝；未知会话 Exited 无效果", () => {
    const { agg, celebrate, bases } = makeAggregator();
    agg.applyStatus("s1", "idle");
    agg.applyStatus("s1", "exited");
    expect(celebrate()).toBe(0);
    agg.applyStatus("ghost", "exited");
    expect(celebrate()).toBe(0);
    expect(bases.filter((b) => b !== "idle")).toEqual([]);
  });

  it("重复状态事件幂等，不重复回调", () => {
    const { agg, bases } = makeAggregator();
    agg.applyStatus("s1", "active");
    agg.applyStatus("s1", "active");
    agg.applyStatus("s1", "active");
    expect(bases).toEqual(["working"]);
  });

  it("快照初始化：填充 Map 并算出基础态", () => {
    const { agg } = makeAggregator();
    agg.applySnapshot([
      { sessionId: "s1", status: "idle" },
      { sessionId: "s2", status: "active" },
      { sessionId: "s3", status: "exited" },
    ]);
    expect(agg.base).toBe("working");
  });

  it("快照后增量事件正常仲裁", () => {
    const { agg } = makeAggregator();
    agg.applySnapshot([{ sessionId: "s1", status: "active" }]);
    agg.applyStatus("s2", "waiting_input");
    expect(agg.base).toBe("attention");
    agg.applyStatus("s2", "exited");
    expect(agg.base).toBe("working");
  });
});
