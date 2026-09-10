import { describe, expect, it } from "vitest";
import { DEFAULT_PET_THEME, parsePetTheme } from "./petTheme";

describe("parsePetTheme", () => {
  it("非对象输入回退到默认主题", () => {
    expect(parsePetTheme(undefined)).toEqual(DEFAULT_PET_THEME);
    expect(parsePetTheme(null)).toEqual(DEFAULT_PET_THEME);
    expect(parsePetTheme("junk")).toEqual(DEFAULT_PET_THEME);
  });

  it("解析合法的完整配置", () => {
    const theme = parsePetTheme({
      name: "Clawd",
      states: { idle: ["custom-idle.svg"] },
      reactions: {
        clickLeft: { file: "left.svg", duration: 1000 },
      },
      timings: { minDisplay: 500 },
    });
    expect(theme.name).toBe("Clawd");
    expect(theme.idleFiles).toEqual(["custom-idle.svg"]);
    expect(theme.reactions.clickLeft).toEqual({ files: ["left.svg"], duration: 1000 });
    expect(theme.minDisplay).toBe(500);
    // 未提供的字段合并默认值
    expect(theme.reactions.annoyed).toEqual(DEFAULT_PET_THEME.reactions.annoyed);
  });

  it("file 单数形式归一化为 files 数组", () => {
    const theme = parsePetTheme({
      reactions: { drag: { file: "drag.svg" } },
    });
    expect(theme.reactions.drag.files).toEqual(["drag.svg"]);
  });

  it("非法 duration 与空 files 回退默认", () => {
    const theme = parsePetTheme({
      reactions: {
        annoyed: { file: "a.svg", duration: -5 },
        double: { files: [] },
      },
    });
    expect(theme.reactions.annoyed.duration).toBe(DEFAULT_PET_THEME.reactions.annoyed.duration);
    expect(theme.reactions.double).toEqual(DEFAULT_PET_THEME.reactions.double);
  });

  it("idle 列表为空或非字符串时回退默认", () => {
    expect(parsePetTheme({ states: { idle: [] } }).idleFiles).toEqual(DEFAULT_PET_THEME.idleFiles);
    expect(parsePetTheme({ states: { idle: [1, 2] } }).idleFiles).toEqual(DEFAULT_PET_THEME.idleFiles);
  });

  it("解析合法的 eyeTracking 配置", () => {
    const theme = parsePetTheme({
      eyeTracking: {
        enabled: false,
        states: ["idle"],
        eyeRatioX: 0.5,
        eyeRatioY: 0.7,
        maxOffset: 4,
        bodyScale: 0.25,
        shadowStretch: 0.2,
        shadowShift: 0.4,
        ids: { eyes: "e", body: "b", shadow: "s" },
      },
    });
    expect(theme.eyeTracking).toEqual({
      enabled: false,
      states: ["idle"],
      eyeRatioX: 0.5,
      eyeRatioY: 0.7,
      maxOffset: 4,
      bodyScale: 0.25,
      shadowStretch: 0.2,
      shadowShift: 0.4,
      ids: { eyes: "e", body: "b", shadow: "s" },
    });
  });

  it("缺失 eyeTracking 回退默认", () => {
    expect(parsePetTheme({}).eyeTracking).toEqual(DEFAULT_PET_THEME.eyeTracking);
    expect(parsePetTheme({ eyeTracking: "junk" }).eyeTracking).toEqual(DEFAULT_PET_THEME.eyeTracking);
    expect(parsePetTheme({ eyeTracking: null }).eyeTracking).toEqual(DEFAULT_PET_THEME.eyeTracking);
  });

  it("eyeTracking 非法字段逐项回退默认", () => {
    const d = DEFAULT_PET_THEME.eyeTracking;
    const theme = parsePetTheme({
      eyeTracking: {
        enabled: "yes",
        states: "idle",
        eyeRatioX: "junk",
        eyeRatioY: Number.NaN,
        maxOffset: Number.POSITIVE_INFINITY,
        bodyScale: null,
        shadowStretch: undefined,
        shadowShift: {},
        ids: { eyes: 1, body: "b" },
      },
    });
    expect(theme.eyeTracking.enabled).toBe(d.enabled);
    expect(theme.eyeTracking.states).toEqual(d.states);
    expect(theme.eyeTracking.eyeRatioX).toBe(d.eyeRatioX);
    expect(theme.eyeTracking.eyeRatioY).toBe(d.eyeRatioY);
    expect(theme.eyeTracking.maxOffset).toBe(d.maxOffset);
    expect(theme.eyeTracking.bodyScale).toBe(d.bodyScale);
    expect(theme.eyeTracking.shadowStretch).toBe(d.shadowStretch);
    expect(theme.eyeTracking.shadowShift).toBe(d.shadowShift);
    expect(theme.eyeTracking.ids).toEqual(d.ids);
  });

  it("eyeTracking states 含非字符串元素时回退默认", () => {
    expect(
      parsePetTheme({ eyeTracking: { states: ["idle", 1] } }).eyeTracking.states,
    ).toEqual(DEFAULT_PET_THEME.eyeTracking.states);
  });

  it("解析合法的 dizzy 配置", () => {
    const theme = parsePetTheme({
      states: { dizzy: ["custom-dizzy.svg"] },
      timings: { autoReturn: { dizzy: 4000 } },
    });
    expect(theme.dizzyFiles).toEqual(["custom-dizzy.svg"]);
    expect(theme.dizzyDuration).toBe(4000);
  });

  it("dizzy 缺失或非法时回退默认", () => {
    expect(parsePetTheme({}).dizzyFiles).toEqual(DEFAULT_PET_THEME.dizzyFiles);
    expect(parsePetTheme({}).dizzyDuration).toBe(DEFAULT_PET_THEME.dizzyDuration);
    expect(parsePetTheme({ states: { dizzy: [] } }).dizzyFiles).toEqual(DEFAULT_PET_THEME.dizzyFiles);
    expect(parsePetTheme({ states: { dizzy: [1] } }).dizzyFiles).toEqual(DEFAULT_PET_THEME.dizzyFiles);
    expect(
      parsePetTheme({ timings: { autoReturn: { dizzy: -1 } } }).dizzyDuration,
    ).toBe(DEFAULT_PET_THEME.dizzyDuration);
    expect(
      parsePetTheme({ timings: { autoReturn: { dizzy: "junk" } } }).dizzyDuration,
    ).toBe(DEFAULT_PET_THEME.dizzyDuration);
    expect(parsePetTheme({ timings: { autoReturn: {} } }).dizzyDuration).toBe(
      DEFAULT_PET_THEME.dizzyDuration,
    );
  });
});

describe("parsePetTheme 会话联动状态", () => {
  it("解析 working/attention/happy 文件列表与 happyDuration", () => {
    const theme = parsePetTheme({
      states: {
        working: ["w.svg"],
        attention: ["a.svg"],
        happy: ["h.svg"],
      },
      timings: { autoReturn: { happy: 4000 } },
    });
    expect(theme.workingFiles).toEqual(["w.svg"]);
    expect(theme.attentionFiles).toEqual(["a.svg"]);
    expect(theme.happyFiles).toEqual(["h.svg"]);
    expect(theme.happyDuration).toBe(4000);
  });

  it("缺失或非法时回退默认值", () => {
    const theme = parsePetTheme({
      states: { working: [], attention: [1], happy: "junk" },
      timings: { autoReturn: { happy: -1 } },
    });
    expect(theme.workingFiles).toEqual(DEFAULT_PET_THEME.workingFiles);
    expect(theme.attentionFiles).toEqual(DEFAULT_PET_THEME.attentionFiles);
    expect(theme.happyFiles).toEqual(DEFAULT_PET_THEME.happyFiles);
    expect(theme.happyDuration).toBe(DEFAULT_PET_THEME.happyDuration);
  });

  it("缺省输入时三项均为默认", () => {
    const theme = parsePetTheme({});
    expect(theme.workingFiles).toEqual(DEFAULT_PET_THEME.workingFiles);
    expect(theme.attentionFiles).toEqual(DEFAULT_PET_THEME.attentionFiles);
    expect(theme.happyFiles).toEqual(DEFAULT_PET_THEME.happyFiles);
    expect(theme.happyDuration).toBe(DEFAULT_PET_THEME.happyDuration);
  });
});
