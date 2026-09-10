import { describe, expect, it } from "vitest";
import {
  EXIT_NUDGE,
  MINI_OFFSET_RATIO,
  NORMAL_SIZE,
  PEEK_OFFSET,
  SNAP_TOLERANCE,
  clampMiniY,
  exitRestingX,
  miniFrameX,
  miniPeekX,
  miniVisibleWidth,
  snapEdgeForDrop,
} from "./miniMode";

// 对齐原项目数值：offsetRatio 0.486 / PEEK 25 / margin = width * 0.25 / 容差 30
describe("constants", () => {
  it("与原项目 mini.js 一致", () => {
    expect(NORMAL_SIZE).toBe(200);
    expect(MINI_OFFSET_RATIO).toBe(0.486);
    expect(PEEK_OFFSET).toBe(25);
    expect(SNAP_TOLERANCE).toBe(30);
  });
});

describe("snapEdgeForDrop", () => {
  // 屏幕 1512 宽，窗口 200 宽 → margin = round(200 * 0.25) = 50
  // 右吸附阈值：x >= 1512 - 200 + 50 - 30 = 1332
  it("右缘在阈值内吸附右边缘（含边界）", () => {
    expect(snapEdgeForDrop(1332, 200, 0, 1512)).toBe("right"); // 正好阈值
    expect(snapEdgeForDrop(1400, 200, 0, 1512)).toBe("right");
  });
  it("左缘在阈值内吸附左边缘（含边界）", () => {
    // 左吸附阈值：x <= 0 - 50 + 30 = -20
    expect(snapEdgeForDrop(-20, 200, 0, 1512)).toBe("left"); // 正好阈值
    expect(snapEdgeForDrop(-50, 200, 0, 1512)).toBe("left");
  });
  it("中间区域不吸附", () => {
    expect(snapEdgeForDrop(1331, 200, 0, 1512)).toBeNull(); // 右阈值外 1px
    expect(snapEdgeForDrop(1512 - 200, 200, 0, 1512)).toBeNull(); // 正好贴边仍差 20px
    expect(snapEdgeForDrop(-19, 200, 0, 1512)).toBeNull(); // 左阈值外 1px
    expect(snapEdgeForDrop(600, 200, 0, 1512)).toBeNull();
  });
  it("左右都满足时优先右边缘", () => {
    // 窗口比屏幕还宽的极端情况
    expect(snapEdgeForDrop(0, 2000, 0, 1512)).toBe("right");
  });
});

describe("miniFrameX", () => {
  // 窗口保持 200 原尺寸：right → 1512 - round(200 * 0.514) = 1512 - 103
  it("右边缘：51.4% 可见，其余藏屏外", () => {
    expect(miniFrameX("right", 200, 0, 1512)).toBe(1409);
  });
  it("左边缘：48.6% 藏屏外", () => {
    expect(miniFrameX("left", 200, 0, 1512)).toBe(-97);
  });
});

describe("miniVisibleWidth", () => {
  it("右边缘可见宽 = round(width * 0.514)", () => {
    expect(miniVisibleWidth("right", 200)).toBe(103);
  });
  it("左边缘可见宽 = round(width * 0.486)", () => {
    expect(miniVisibleWidth("left", 200)).toBe(97);
  });
});

describe("miniPeekX", () => {
  it("左边缘 peek：向屏内 +25", () => {
    expect(miniPeekX(-97, "left")).toBe(-72);
  });
  it("右边缘 peek：向屏内 -25", () => {
    expect(miniPeekX(1409, "right")).toBe(1384);
  });
});

describe("exitRestingX", () => {
  // 屏幕 1512 宽，窗口 200 宽 → margin = 50
  // 右触发区阈值：1512 - 200 + 50 - 30 = 1332；内推落点：1512 - 200 + 50 - 100 = 1262
  it("右触发区内推约 100px（含边界）", () => {
    expect(EXIT_NUDGE).toBe(100);
    expect(exitRestingX(1400, 200, 0, 1512)).toBe(1262);
    expect(exitRestingX(1332, 200, 0, 1512)).toBe(1262); // 正好阈值（>=）
  });
  it("左触发区内推约 100px（含边界）", () => {
    // 左触发区阈值：0 - 50 + 30 = -20；内推落点：-20 + 100 = 80
    expect(exitRestingX(-30, 200, 0, 1512)).toBe(80);
    expect(exitRestingX(-20, 200, 0, 1512)).toBe(80); // 正好阈值（<=）
  });
  it("非触发区保持吸附前位置", () => {
    expect(exitRestingX(600, 200, 0, 1512)).toBe(600);
    expect(exitRestingX(1331, 200, 0, 1512)).toBe(1331); // 右阈值外 1px
  });
});

describe("clampMiniY", () => {
  it("钳制在屏幕垂直范围内（窗口高 NORMAL_SIZE）", () => {
    expect(clampMiniY(500, 0, 982)).toBe(500);
    expect(clampMiniY(-50, 0, 982)).toBe(0);
    expect(clampMiniY(9999, 0, 982)).toBe(982 - 200);
  });
});
