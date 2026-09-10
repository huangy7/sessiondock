import { describe, it, expect } from "vitest";
import { expandPhraseVariables } from "./phrase-variables";

// 2026-08-11 是周二；本周一 = 08-10，本周五 = 08-14，上周一 = 08-03，上周五 = 08-07
const TUESDAY = new Date(2026, 7, 11, 15, 0, 0);

describe("expandPhraseVariables", () => {
  it("展开全部支持的变量", () => {
    const out = expandPhraseVariables(
      "总结 {{本周一}} 至 {{今天}} 的工作；对照 {{上周一}}~{{上周五}}；昨天={{昨天}}；周五={{本周五}}",
      TUESDAY,
    );
    expect(out).toBe(
      "总结 2026-08-10 至 2026-08-11 的工作；对照 2026-08-03~2026-08-07；昨天=2026-08-10；周五=2026-08-14",
    );
  });

  it("未知变量原样保留", () => {
    expect(expandPhraseVariables("看看 {{明年}} 的 {{今天}}", TUESDAY)).toBe(
      "看看 {{明年}} 的 2026-08-11",
    );
  });

  it("无变量文本原样返回", () => {
    expect(expandPhraseVariables("没有任何变量", TUESDAY)).toBe("没有任何变量");
  });

  it("同一变量出现多次全部展开", () => {
    expect(expandPhraseVariables("{{今天}}/{{今天}}", TUESDAY)).toBe("2026-08-11/2026-08-11");
  });
});
