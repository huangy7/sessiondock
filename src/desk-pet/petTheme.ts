import rawTheme from "../assets/desk-pet/clawd/theme.json";

export interface PetReaction {
  files: string[];
  duration: number;
}

export interface EyeTrackingConfig {
  enabled: boolean;
  /** 启用眼睛跟随的状态名（"idle" / "mini-idle"） */
  states: string[];
  eyeRatioX: number;
  eyeRatioY: number;
  maxOffset: number;
  bodyScale: number;
  shadowStretch: number;
  shadowShift: number;
  ids: { eyes: string; body: string; shadow: string };
}

export interface PetTheme {
  name: string;
  idleFiles: string[];
  reactions: {
    drag: PetReaction;
    clickLeft: PetReaction;
    clickRight: PetReaction;
    annoyed: PetReaction;
    double: PetReaction;
  };
  /** oneshot 反应动画的最短展示时长下限（ms） */
  minDisplay: number;
  /** dizzy 状态的 SVG 文件列表 */
  dizzyFiles: string[];
  /** dizzy 展示时长（ms），来自 timings.autoReturn.dizzy */
  dizzyDuration: number;
  /** working 状态（会话活跃）的 SVG 文件列表 */
  workingFiles: string[];
  /** attention 状态（会话等待输入）的 SVG 文件列表 */
  attentionFiles: string[];
  /** happy oneshot（会话完成庆祝）的 SVG 文件列表 */
  happyFiles: string[];
  /** happy oneshot 展示时长（ms），来自 timings.autoReturn.happy */
  happyDuration: number;
  eyeTracking: EyeTrackingConfig;
}

export const DEFAULT_PET_THEME: PetTheme = {
  name: "fallback",
  idleFiles: ["clawd-idle-follow.svg"],
  reactions: {
    drag: { files: ["clawd-react-drag.svg"], duration: 0 },
    clickLeft: { files: ["clawd-react-left.svg"], duration: 2500 },
    clickRight: { files: ["clawd-react-right.svg"], duration: 2500 },
    annoyed: { files: ["clawd-react-annoyed.svg"], duration: 3500 },
    double: { files: ["clawd-react-double.svg", "clawd-react-double-jump.svg"], duration: 3500 },
  },
  minDisplay: 800,
  dizzyFiles: ["clawd-dizzy.svg"],
  dizzyDuration: 6000,
  workingFiles: ["clawd-working-typing.svg"],
  attentionFiles: ["clawd-happy.svg"],
  happyFiles: ["clawd-react-double-jump.svg"],
  happyDuration: 4000,
  eyeTracking: {
    enabled: true,
    states: ["idle", "mini-idle"],
    eyeRatioX: 0.489,
    eyeRatioY: 0.756,
    maxOffset: 3,
    bodyScale: 0.33,
    shadowStretch: 0.15,
    shadowShift: 0.3,
    ids: { eyes: "eyes-js", body: "body-js", shadow: "shadow-js" },
  },
};

interface RawReaction {
  file?: unknown;
  files?: unknown;
  duration?: unknown;
}

function normalizeReaction(raw: unknown, fallback: PetReaction): PetReaction {
  if (typeof raw !== "object" || raw === null) return fallback;
  const r = raw as RawReaction;
  let files: string[] | null = null;
  if (Array.isArray(r.files) && r.files.length > 0 && r.files.every((f) => typeof f === "string")) {
    files = r.files as string[];
  } else if (typeof r.file === "string") {
    files = [r.file];
  }
  if (!files) return fallback;
  const duration =
    typeof r.duration === "number" && r.duration > 0 ? r.duration : fallback.duration;
  return { files, duration };
}

function normalizeFileList(raw: unknown, fallback: string[]): string[] {
  return Array.isArray(raw) && raw.length > 0 && raw.every((f) => typeof f === "string")
    ? (raw as string[])
    : fallback;
}

function normalizeFiniteNumber(raw: unknown, fallback: number): number {
  return typeof raw === "number" && Number.isFinite(raw) ? raw : fallback;
}

function normalizeEyeTracking(raw: unknown, fallback: EyeTrackingConfig): EyeTrackingConfig {
  if (typeof raw !== "object" || raw === null) return fallback;
  const obj = raw as Record<string, unknown>;
  const ids =
    typeof obj.ids === "object" && obj.ids !== null
      ? (obj.ids as Record<string, unknown>)
      : {};
  const idsValid =
    typeof ids.eyes === "string" && typeof ids.body === "string" && typeof ids.shadow === "string";
  return {
    enabled: typeof obj.enabled === "boolean" ? obj.enabled : fallback.enabled,
    states:
      Array.isArray(obj.states) &&
      obj.states.length > 0 &&
      obj.states.every((s) => typeof s === "string")
        ? (obj.states as string[])
        : fallback.states,
    eyeRatioX: normalizeFiniteNumber(obj.eyeRatioX, fallback.eyeRatioX),
    eyeRatioY: normalizeFiniteNumber(obj.eyeRatioY, fallback.eyeRatioY),
    maxOffset: normalizeFiniteNumber(obj.maxOffset, fallback.maxOffset),
    bodyScale: normalizeFiniteNumber(obj.bodyScale, fallback.bodyScale),
    shadowStretch: normalizeFiniteNumber(obj.shadowStretch, fallback.shadowStretch),
    shadowShift: normalizeFiniteNumber(obj.shadowShift, fallback.shadowShift),
    ids: idsValid
      ? { eyes: ids.eyes as string, body: ids.body as string, shadow: ids.shadow as string }
      : fallback.ids,
  };
}

export function parsePetTheme(raw: unknown): PetTheme {
  const d = DEFAULT_PET_THEME;
  if (typeof raw !== "object" || raw === null) return d;
  const obj = raw as Record<string, unknown>;

  const states = (obj.states ?? {}) as Record<string, unknown>;
  const idleFiles = normalizeFileList(states.idle, d.idleFiles);

  const reactions = (obj.reactions ?? {}) as Record<string, unknown>;

  const timings = (obj.timings ?? {}) as Record<string, unknown>;
  const minDisplay =
    typeof timings.minDisplay === "number" && timings.minDisplay > 0
      ? timings.minDisplay
      : d.minDisplay;

  const dizzyFiles = normalizeFileList(states.dizzy, d.dizzyFiles);

  const autoReturn = (timings.autoReturn ?? {}) as Record<string, unknown>;
  const dizzyDuration =
    typeof autoReturn.dizzy === "number" && autoReturn.dizzy > 0
      ? autoReturn.dizzy
      : d.dizzyDuration;
  const happyDuration =
    typeof autoReturn.happy === "number" && autoReturn.happy > 0
      ? autoReturn.happy
      : d.happyDuration;

  return {
    name: typeof obj.name === "string" ? obj.name : d.name,
    idleFiles,
    reactions: {
      drag: normalizeReaction(reactions.drag, d.reactions.drag),
      clickLeft: normalizeReaction(reactions.clickLeft, d.reactions.clickLeft),
      clickRight: normalizeReaction(reactions.clickRight, d.reactions.clickRight),
      annoyed: normalizeReaction(reactions.annoyed, d.reactions.annoyed),
      double: normalizeReaction(reactions.double, d.reactions.double),
    },
    minDisplay,
    dizzyFiles,
    dizzyDuration,
    workingFiles: normalizeFileList(states.working, d.workingFiles),
    attentionFiles: normalizeFileList(states.attention, d.attentionFiles),
    happyFiles: normalizeFileList(states.happy, d.happyFiles),
    happyDuration,
    eyeTracking: normalizeEyeTracking(obj.eyeTracking, d.eyeTracking),
  };
}

/** 加载内置主题；任何异常都回退到硬编码默认主题，保证功能降级不崩 */
export function loadPetTheme(): PetTheme {
  try {
    return parsePetTheme(rawTheme);
  } catch {
    return DEFAULT_PET_THEME;
  }
}
