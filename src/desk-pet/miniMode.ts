/** 桌宠窗口常规尺寸（逻辑像素，与 Rust 侧 DESK_PET_WIDTH/HEIGHT 一致） */
export const NORMAL_SIZE = 200;
/** mini 态窗口隐藏在屏外的比例（对齐原项目 miniMode.offsetRatio） */
export const MINI_OFFSET_RATIO = 0.486;
/** hover 探出时窗口向屏内滑动的距离（对齐原项目 PEEK_OFFSET） */
export const PEEK_OFFSET = 25;
/** 吸附判定的边缘余量比例（对齐原项目 mEdge = width * 0.25） */
export const EDGE_MARGIN_RATIO = 0.25;
/** 拖拽松手时的吸附容差（逻辑像素，对齐原项目 SNAP_TOLERANCE） */
export const SNAP_TOLERANCE = 30;

/** 跳出 mini 时的防重吸附内推距离（对齐原项目 100px） */
export const EXIT_NUDGE = 100;

export type MiniEdge = "left" | "right";

/** 拖拽松手位置是否应吸附（对齐原项目公式）。左右都满足时优先右。 */
export function snapEdgeForDrop(
  windowX: number,
  windowWidth: number,
  screenLeft: number,
  screenRight: number,
): MiniEdge | null {
  const margin = Math.round(windowWidth * EDGE_MARGIN_RATIO);
  if (windowX >= screenRight - windowWidth + margin - SNAP_TOLERANCE) return "right";
  if (windowX <= screenLeft - margin + SNAP_TOLERANCE) return "left";
  return null;
}

/** mini 窗口 x：窗口保持原尺寸，按 offsetRatio 藏入屏外（对齐原项目公式） */
export function miniFrameX(
  edge: MiniEdge,
  windowWidth: number,
  screenLeft: number,
  screenRight: number,
): number {
  return edge === "right"
    ? screenRight - Math.round(windowWidth * (1 - MINI_OFFSET_RATIO))
    : screenLeft - Math.round(windowWidth * MINI_OFFSET_RATIO);
}

/** mini 态可见条宽度（hover 热区宽度） */
export function miniVisibleWidth(edge: MiniEdge, windowWidth: number): number {
  return Math.round(
    windowWidth * (edge === "right" ? 1 - MINI_OFFSET_RATIO : MINI_OFFSET_RATIO),
  );
}

/** peek 探出时的窗口 x（向屏内滑动 PEEK_OFFSET） */
export function miniPeekX(baseX: number, edge: MiniEdge): number {
  return edge === "left" ? baseX + PEEK_OFFSET : baseX - PEEK_OFFSET;
}

/**
 * 跳出 mini 的落点 x：吸附前位置若仍在吸附触发区，内推约 100px（对齐原项目公式）
 */
export function exitRestingX(
  preMiniX: number,
  windowWidth: number,
  screenLeft: number,
  screenRight: number,
): number {
  const margin = Math.round(windowWidth * EDGE_MARGIN_RATIO);
  let x = preMiniX;
  if (x >= screenRight - windowWidth + margin - SNAP_TOLERANCE) {
    x = screenRight - windowWidth + margin - EXIT_NUDGE;
  }
  if (x <= screenLeft - margin + SNAP_TOLERANCE) {
    x = screenLeft - margin + SNAP_TOLERANCE + EXIT_NUDGE;
  }
  return x;
}

/** mini 窗口 y 钳制在屏幕垂直范围内（窗口为 NORMAL_SIZE 高） */
export function clampMiniY(y: number, screenTop: number, screenBottom: number): number {
  const maxY = Math.max(screenTop, screenBottom - NORMAL_SIZE);
  return Math.min(Math.max(y, screenTop), maxY);
}
