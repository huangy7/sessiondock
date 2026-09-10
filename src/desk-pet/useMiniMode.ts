import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalPosition } from "@tauri-apps/api/window";
import { ref, type Ref } from "vue";
import {
  clampMiniY,
  exitRestingX,
  miniFrameX,
  miniPeekX,
  miniVisibleWidth,
  NORMAL_SIZE,
  snapEdgeForDrop,
  type MiniEdge,
} from "./miniMode";

export interface MiniMode {
  active: Ref<boolean>;
  edge: Ref<MiniEdge | null>;
  peeking: Ref<boolean>;
  /** 拖拽松手后调用：若应吸附则进入 mini 并返回 true */
  enterIfSnapped(windowX: number, windowY: number): Promise<boolean>;
  /** 点击跳出 / 退出 mini：抛物线跳回内推后的落点并持久化 */
  exit(): Promise<void>;
  /** 取消进行中的跳出抛物线动画（动画中按下拖拽时调用，防 setPosition 打架） */
  cancelExitAnimation(): void;
  /** mini 中拖拽开始：恢复全尺寸，返回吸附前位置供拖拽重新锚定 */
  beginDragOut(): Promise<{ x: number; y: number }>;
  dispose(): void;
}

export function useMiniMode(): MiniMode {
  const active = ref(false);
  const edge = ref<MiniEdge | null>(null);
  const peeking = ref(false);
  let preMiniX = 0;
  let preMiniY = 0;
  let baseX = 0;
  let baseY = 0;
  let unlistenHover: UnlistenFn | null = null;
  let disposed = false;
  let exitAnimToken = 0;

  void listen<boolean>("desk-pet-mini-hover", (e) => {
    if (active.value && !disposed) {
      peeking.value = e.payload;
      const targetX = e.payload ? miniPeekX(baseX, edge.value ?? "right") : baseX;
      void getCurrentWindow().setPosition(new LogicalPosition(targetX, baseY));
    }
  }).then((fn) => {
    // 卸载后才 resolve 的 listener 立即反注册，防泄漏
    if (disposed) {
      fn();
    } else {
      unlistenHover = fn;
    }
  });

  function screenRect() {
    // availLeft/availTop 未列入 TS DOM lib，但 WebKit/Chromium 均支持；缺省兜底 0
    const s = window.screen as Screen & { availLeft?: number; availTop?: number };
    const left = s.availLeft ?? 0;
    const top = s.availTop ?? 0;
    return {
      left,
      top,
      right: left + s.availWidth,
      bottom: top + s.availHeight,
    };
  }

  async function enterIfSnapped(windowX: number, windowY: number): Promise<boolean> {
    const scr = screenRect();
    const snapped = snapEdgeForDrop(windowX, NORMAL_SIZE, scr.left, scr.right);
    if (!snapped) return false;
    preMiniX = windowX;
    preMiniY = windowY;
    // 先置状态再做窗口几何 await，缩小「已吸附但 active 仍为 false」的窗口期
    edge.value = snapped;
    active.value = true;
    peeking.value = false;
    const win = getCurrentWindow();
    const x = miniFrameX(snapped, NORMAL_SIZE, scr.left, scr.right);
    const y = clampMiniY(windowY, scr.top, scr.bottom);
    baseX = x;
    baseY = y;
    await win.setPosition(new LogicalPosition(x, y));
    await invoke("start_desk_pet_mini_hover", {
      edge: snapped,
      yTop: y,
      yBottom: y + NORMAL_SIZE,
      zoneWidth: miniVisibleWidth(snapped, NORMAL_SIZE),
    });
    return true;
  }

  const EXIT_JUMP_DURATION = 350;
  const EXIT_JUMP_PEAK = 40;

  /** 抛物线跳出动画：350ms，峰值 40px 向上（对齐原项目 JUMP 参数） */
  function animateExitParabola(
    fromX: number,
    fromY: number,
    toX: number,
    toY: number,
  ): Promise<void> {
    const token = ++exitAnimToken;
    const win = getCurrentWindow();
    const start = performance.now();
    return new Promise((resolve) => {
      const step = () => {
        if (disposed || token !== exitAnimToken) {
          resolve();
          return;
        }
        const t = Math.min(1, (performance.now() - start) / EXIT_JUMP_DURATION);
        const eased = t * (2 - t);
        const x = Math.round(fromX + (toX - fromX) * eased);
        const y = Math.round(fromY + (toY - fromY) * eased - 4 * EXIT_JUMP_PEAK * t * (1 - t));
        void win.setPosition(new LogicalPosition(x, y));
        if (t >= 1) {
          resolve();
          return;
        }
        requestAnimationFrame(step);
      };
      requestAnimationFrame(step);
    });
  }

  async function exit(): Promise<void> {
    if (!active.value) return;
    active.value = false;
    peeking.value = false;
    await invoke("stop_desk_pet_mini_hover");
    const win = getCurrentWindow();
    // 动画起点取窗口实际位置（可能在 peek 探出位 ±25px），避免首帧瞬移
    const [pos, factor] = await Promise.all([win.outerPosition(), win.scaleFactor()]);
    const fromX = pos.x / factor;
    const fromY = pos.y / factor;
    const scr = screenRect();
    const targetX = exitRestingX(preMiniX, NORMAL_SIZE, scr.left, scr.right);
    const targetY = preMiniY;
    await animateExitParabola(fromX, fromY, targetX, targetY);
    // 持久化内推后的落点，避免重启后回到贴边位置
    void invoke("save_desk_pet_position");
  }

  function cancelExitAnimation(): void {
    exitAnimToken++;
  }

  async function beginDragOut(): Promise<{ x: number; y: number }> {
    if (active.value) {
      // 拖出路径瞬时恢复（拖拽会重新锚定到光标下），不做抛物线动画
      exitAnimToken++;
      active.value = false;
      peeking.value = false;
      await invoke("stop_desk_pet_mini_hover");
      await getCurrentWindow().setPosition(new LogicalPosition(preMiniX, preMiniY));
    }
    return { x: preMiniX, y: preMiniY };
  }

  function dispose(): void {
    disposed = true;
    unlistenHover?.();
    // 双保险：mini 激活时窗口被关闭也能停止 Rust 轮询（Rust 侧另有窗口存活锚定）
    void invoke("stop_desk_pet_mini_hover");
  }

  return { active, edge, peeking, enterIfSnapped, exit, cancelExitAnimation, beginDragOut, dispose };
}
