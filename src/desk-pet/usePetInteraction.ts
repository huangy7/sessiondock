import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, PhysicalPosition } from "@tauri-apps/api/window";
import {
  createClickAccumulator,
  type ClickAccumulator,
  type PetStateMachine,
} from "./petStateMachine";
import type { MiniMode } from "./useMiniMode";

const DRAG_THRESHOLD_PX = 3;

export interface PetInteractionHandlers {
  onPointerdown: (e: PointerEvent) => void;
  onPointermove: (e: PointerEvent) => void;
  onPointerup: (e: PointerEvent) => void;
}

/**
 * 桌宠窗口级交互：
 * - pointerdown 记录起点并预取窗口原点/缩放因子
 * - pointermove 超阈值进入拖拽，rAF 节流移动窗口，按方向驱动 drag-left/right
 * - pointerup 无位移则计入点击（2 连击 poke/annoyed，4 连击 flail），拖拽结束保存位置
 * - mini 感知：mini 中点击跳出、拖拽超阈值立即恢复全尺寸拖出、松手检测边缘吸附
 */
export function usePetInteraction(
  machine: PetStateMachine,
  mini: MiniMode,
): {
  handlers: PetInteractionHandlers;
} {
  let lastSide: "left" | "right" = "right";
  const accumulator: ClickAccumulator = createClickAccumulator({
    windowMs: 400,
    onReaction: (count) => {
      machine.dispatch({ type: "click", count, side: lastSide });
    },
  });
  let activePointerId: number | null = null;
  let startScreenX = 0;
  let startScreenY = 0;
  let originX = 0; // 窗口原点（物理像素）
  let originY = 0;
  let scaleFactor = 1;
  let dragging = false;
  let rafId: number | null = null;
  let latestMove: PointerEvent | null = null;
  /** mini 拖出恢复全尺寸期间：拖拽锚点尚未重置，跳过窗口移动 */
  let reanchoring = false;

  function onPointerdown(e: PointerEvent): void {
    if (activePointerId !== null || e.button !== 0) return;
    // 跳出抛物线动画进行中按下：取消动画，避免 rAF setPosition 与拖拽打架
    mini.cancelExitAnimation();
    activePointerId = e.pointerId;
    (e.currentTarget as Element | null)?.setPointerCapture?.(e.pointerId);
    startScreenX = e.screenX;
    startScreenY = e.screenY;
    dragging = false;
    // mini 态拖出时 origin 会被 beginDragOut 重新锚定，此处预取无意义
    if (mini.active.value) return;
    const win = getCurrentWindow();
    void Promise.all([win.outerPosition(), win.scaleFactor()]).then(([pos, factor]) => {
      originX = pos.x;
      originY = pos.y;
      scaleFactor = factor;
    });
  }

  function applyMove(): void {
    rafId = null;
    const e = latestMove;
    if (e === null || activePointerId === null) return;
    const dx = e.screenX - startScreenX;
    const dy = e.screenY - startScreenY;
    if (!dragging) {
      if (Math.abs(dx) < DRAG_THRESHOLD_PX && Math.abs(dy) < DRAG_THRESHOLD_PX) return;
      dragging = true;
      accumulator.reset();
      if (mini.active.value) {
        // mini 拖出：恢复全尺寸与吸附前位置后，以当前光标为起点重新锚定拖拽
        reanchoring = true;
        const anchorScreenX = e.screenX;
        const anchorScreenY = e.screenY;
        void mini.beginDragOut().then((pos) => {
          void getCurrentWindow()
            .scaleFactor()
            .then((factor) => {
              scaleFactor = factor;
              originX = pos.x * factor;
              originY = pos.y * factor;
              startScreenX = anchorScreenX;
              startScreenY = anchorScreenY;
              reanchoring = false;
            });
        });
        return;
      }
    }
    if (reanchoring) return;
    machine.dispatch({
      type: "dragMove",
      direction: dx < 0 ? "left" : "right",
    });
    void getCurrentWindow().setPosition(
      new PhysicalPosition(
        Math.round(originX + dx * scaleFactor),
        Math.round(originY + dy * scaleFactor),
      ),
    );
  }

  function onPointermove(e: PointerEvent): void {
    if (e.pointerId !== activePointerId) return;
    latestMove = e;
    if (rafId === null) {
      rafId = requestAnimationFrame(applyMove);
    }
  }

  function onPointerup(e: PointerEvent): void {
    if (e.pointerId !== activePointerId) return;
    activePointerId = null;
    if (rafId !== null) {
      cancelAnimationFrame(rafId);
      rafId = null;
    }
    latestMove = null;
    if (dragging) {
      dragging = false;
      machine.dispatch({ type: "dragEnd" });
      void invoke("save_desk_pet_position");
      const win = getCurrentWindow();
      void win.outerPosition().then(async (pos) => {
        const factor = await win.scaleFactor();
        const snapped = await mini.enterIfSnapped(pos.x / factor, pos.y / factor);
        if (snapped) {
          // mini 位置不覆盖已记忆的可见位置：吸附前位置已在 save 时持久化
        }
      });
      return;
    }
    // mini 中点击（未触发拖拽）→ 跳出恢复全尺寸与吸附前位置
    if (mini.active.value) {
      void mini.exit();
      return;
    }
    lastSide = e.clientX < window.innerWidth / 2 ? "left" : "right";
    accumulator.registerClick();
  }

  return { handlers: { onPointerdown, onPointermove, onPointerup } };
}
