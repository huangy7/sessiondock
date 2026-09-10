import type { EyeTrackingConfig } from "./petTheme";

export interface EyeTargets {
  eyes: Element | null;
  body: Element | null;
  shadow: Element | null;
}

/** 在子树根（内联 SVG 所在层）内查找眼睛跟随目标元素 */
export function attachEyeTargets(
  rootEl: Element | null,
  ids: EyeTrackingConfig["ids"],
): EyeTargets | null {
  if (!rootEl) return null;
  return {
    eyes: rootEl.querySelector(`#${ids.eyes}`),
    body: rootEl.querySelector(`#${ids.body}`),
    shadow: rootEl.querySelector(`#${ids.shadow}`),
  };
}

const quantize = (v: number) => Math.round(v * 2) / 2;

/**
 * 应用眼睛/身体/影子偏移（对齐原项目 applyEyeMove 公式）。
 * 注意必须写 CSS style.transform 而非 transform 属性：
 * WKWebView 中 transform 呈现属性的变更不会触发 SVG 内置的
 * `transition: transform 0.2s ease-out`，运动会变成瞬时跳变（不丝滑）；
 * CSS 属性变更才能正常走过渡。
 */
export function applyEyeMove(
  targets: EyeTargets,
  dx: number,
  dy: number,
  cfg: EyeTrackingConfig,
): void {
  if (targets.eyes) {
    (targets.eyes as SVGElement).style.transform = `translate(${dx}px, ${dy}px)`;
  }
  const bdx = quantize(dx * cfg.bodyScale);
  const bdy = quantize(dy * cfg.bodyScale);
  if (targets.body) {
    (targets.body as SVGElement).style.transform = `translate(${bdx}px, ${bdy}px)`;
  }
  if (targets.shadow) {
    const scaleX = 1 + Math.abs(bdx) * cfg.shadowStretch;
    const shiftX = quantize(bdx * cfg.shadowShift);
    (targets.shadow as SVGElement).style.transform =
      `translate(${shiftX}px, 0px) scale(${scaleX}, 1)`;
  }
}
