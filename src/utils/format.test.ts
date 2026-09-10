import { describe, expect, it } from "vitest";
import { formatCost, formatCostCompact, formatTokenCount } from "./format";

describe("formatCost", () => {
  it("formats with thousand separators and two decimals", () => {
    expect(formatCost(1234.56)).toBe("$1,234.56");
    expect(formatCost(673.66)).toBe("$673.66");
    expect(formatCost(0)).toBe("$0.00");
  });

  it("keeps the sign for negative values", () => {
    expect(formatCost(-12.5)).toBe("-$12.50");
  });

  it("treats non-finite input as zero", () => {
    expect(formatCost(NaN)).toBe("$0.00");
    expect(formatCost(Infinity)).toBe("$0.00");
  });
});

describe("formatCostCompact", () => {
  it("keeps two decimals below 100", () => {
    expect(formatCostCompact(0)).toBe("$0.00");
    expect(formatCostCompact(67.33)).toBe("$67.33");
    expect(formatCostCompact(99.99)).toBe("$99.99");
  });

  it("drops to one decimal from 100 up to 1000", () => {
    expect(formatCostCompact(100)).toBe("$100.0");
    expect(formatCostCompact(673.66)).toBe("$673.7");
    expect(formatCostCompact(999.94)).toBe("$999.9");
  });

  it("uses K suffix at thousands", () => {
    expect(formatCostCompact(999.95)).toBe("$1.00K");
    expect(formatCostCompact(1234.56)).toBe("$1.23K");
    expect(formatCostCompact(999994)).toBe("$999.99K");
  });

  it("uses M suffix at millions", () => {
    expect(formatCostCompact(999995)).toBe("$1.00M");
    expect(formatCostCompact(1_500_000)).toBe("$1.50M");
  });

  it("keeps the sign and handles non-finite input", () => {
    expect(formatCostCompact(-673.66)).toBe("-$673.7");
    expect(formatCostCompact(NaN)).toBe("$0.00");
  });
});

describe("formatTokenCount", () => {
  it("千以下原样显示", () => {
    expect(formatTokenCount(0)).toBe("0");
    expect(formatTokenCount(380)).toBe("380");
    expect(formatTokenCount(999)).toBe("999");
  });

  it("千级显示 k，一位小数并去尾零", () => {
    expect(formatTokenCount(1200)).toBe("1.2k");
    expect(formatTokenCount(5000)).toBe("5k");
    expect(formatTokenCount(12400)).toBe("12.4k");
  });

  it("百万级显示 M", () => {
    expect(formatTokenCount(2_500_000)).toBe("2.5M");
    expect(formatTokenCount(1_000_000)).toBe("1M");
  });

  it("异常输入兜底为 0", () => {
    expect(formatTokenCount(NaN)).toBe("0");
    expect(formatTokenCount(-5)).toBe("0");
  });
});
