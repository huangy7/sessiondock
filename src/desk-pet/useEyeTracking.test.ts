import { describe, expect, it } from "vitest";
import { DEFAULT_PET_THEME } from "./petTheme";
import { applyEyeMove, attachEyeTargets, type EyeTargets } from "./useEyeTracking";

const CFG = DEFAULT_PET_THEME.eyeTracking;

function makeTargets(): EyeTargets & { eyes: Element; body: Element; shadow: Element } {
  return {
    eyes: document.createElementNS("http://www.w3.org/2000/svg", "g"),
    body: document.createElementNS("http://www.w3.org/2000/svg", "g"),
    shadow: document.createElementNS("http://www.w3.org/2000/svg", "g"),
  };
}

describe("applyEyeMove", () => {
  it("眼睛按原值平移，身体按 bodyScale 量化，影子平移+拉伸", () => {
    // dx=2.55, dy=1.5（钳制后的最大偏移）
    // eyes: translate(2.55, 1.5)
    // body: bdx = quantize(2.55*0.33)=quantize(0.8415)=1, bdy = quantize(1.5*0.33)=quantize(0.495)=0.5
    // shadow: shiftX = quantize(1*0.3)=0.5, scaleX = 1+1*0.15=1.15
    const t = makeTargets();
    applyEyeMove(t, 2.55, 1.5, CFG);
    expect((t.eyes as SVGElement).style.transform).toBe("translate(2.55px, 1.5px)");
    expect((t.body as SVGElement).style.transform).toBe("translate(1px, 0.5px)");
    expect((t.shadow as SVGElement).style.transform).toBe("translate(0.5px, 0px) scale(1.15, 1)");
  });

  it("零偏移时身体影子回归初始形态", () => {
    const t = makeTargets();
    applyEyeMove(t, 0, 0, CFG);
    expect((t.eyes as SVGElement).style.transform).toBe("translate(0px, 0px)");
    expect((t.body as SVGElement).style.transform).toBe("translate(0px, 0px)");
    expect((t.shadow as SVGElement).style.transform).toBe("translate(0px, 0px) scale(1, 1)");
  });

  it("负向偏移影子向反方向平移且拉伸对称", () => {
    const t = makeTargets();
    applyEyeMove(t, -2.55, -1.5, CFG);
    expect((t.body as SVGElement).style.transform).toBe("translate(-1px, -0.5px)");
    expect((t.shadow as SVGElement).style.transform).toBe("translate(-0.5px, 0px) scale(1.15, 1)");
  });

  it("目标元素缺失时不抛异常", () => {
    expect(() =>
      applyEyeMove({ eyes: null, body: null, shadow: null }, 1, 1, CFG),
    ).not.toThrow();
  });

  it("部分目标缺失时只操作存在的元素", () => {
    // mini-idle 场景：没有 shadow-js
    const eyes = document.createElementNS("http://www.w3.org/2000/svg", "g");
    const body = document.createElementNS("http://www.w3.org/2000/svg", "g");
    applyEyeMove({ eyes, body, shadow: null }, 2.55, 1.5, CFG);
    expect((eyes as SVGElement).style.transform).toBe("translate(2.55px, 1.5px)");
    expect((body as SVGElement).style.transform).toBe("translate(1px, 0.5px)");
  });
});

describe("attachEyeTargets", () => {
  function makeLayerWithSvg(inner: string): HTMLElement {
    const div = document.createElement("div");
    div.innerHTML = inner;
    return div;
  }

  it("rootEl 为 null 时返回 null", () => {
    expect(attachEyeTargets(null, CFG.ids)).toBeNull();
  });

  it("在内联 SVG 片段的子树内按 id 查找眼睛/身体/影子元素", () => {
    const layer = makeLayerWithSvg(
      '<svg viewBox="0 0 500 500"><g id="body-js"><g id="eyes-js"></g></g><ellipse id="shadow-js"></ellipse></svg>',
    );
    const targets = attachEyeTargets(layer, CFG.ids);
    expect(targets).not.toBeNull();
    expect(targets?.eyes?.id).toBe("eyes-js");
    expect(targets?.body?.id).toBe("body-js");
    expect(targets?.shadow?.id).toBe("shadow-js");
  });

  it("缺失的 id 对应 null（mini-idle 无 shadow-js）", () => {
    const layer = makeLayerWithSvg('<svg><g id="eyes-js"></g></svg>');
    const targets = attachEyeTargets(layer, CFG.ids);
    expect(targets?.eyes?.id).toBe("eyes-js");
    expect(targets?.body).toBeNull();
    expect(targets?.shadow).toBeNull();
  });
});
