<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { loadPetTheme } from "../desk-pet/petTheme";
import { createPetStateMachine, type PetState } from "../desk-pet/petStateMachine";
import { usePetInteraction } from "../desk-pet/usePetInteraction";
import { useMiniMode } from "../desk-pet/useMiniMode";
import { createSessionStateAggregator } from "../desk-pet/sessionStateAggregator";
import type { PtySessionInfo, PtyStatusChangedPayload } from "../types/pty";
import { applyEyeMove, attachEyeTargets, type EyeTargets } from "../desk-pet/useEyeTracking";
import {
  fetchSvgMarkup,
  IDLE_FALLBACK_URL,
  INLINE_SVG_URLS,
  MINI_IDLE_URL,
  MINI_PEEK_URL,
  petAssetUrl,
} from "../desk-pet/petAssets";

const theme = loadPetTheme();

const mini = useMiniMode();

function reactionDuration(state: PetState): number {
  const base = (() => {
    switch (state) {
      case "poke-left":
        return theme.reactions.clickLeft.duration;
      case "poke-right":
        return theme.reactions.clickRight.duration;
      case "annoyed":
        return theme.reactions.annoyed.duration;
      case "flail":
        return theme.reactions.double.duration;
      default:
        return theme.minDisplay;
    }
  })();
  return Math.max(theme.minDisplay, base);
}

function urlForState(state: PetState): string {
  if (mini.active.value) {
    return mini.peeking.value ? MINI_PEEK_URL : MINI_IDLE_URL;
  }
  let file: string | undefined;
  switch (state) {
    case "idle":
      file = theme.idleFiles[Math.floor(Math.random() * theme.idleFiles.length)];
      break;
    case "poke-left":
      file = theme.reactions.clickLeft.files[0];
      break;
    case "poke-right":
      file = theme.reactions.clickRight.files[0];
      break;
    case "annoyed":
      file = theme.reactions.annoyed.files[0];
      break;
    case "flail": {
      const candidates = theme.reactions.double.files;
      file = candidates[Math.floor(Math.random() * candidates.length)];
      break;
    }
    case "drag-left":
    case "drag-right":
      file = theme.reactions.drag.files[0];
      break;
    case "dizzy":
      file = theme.dizzyFiles[0];
      break;
    case "working":
      file = theme.workingFiles[0];
      break;
    case "attention":
      file = theme.attentionFiles[0];
      break;
    case "happy":
      file = theme.happyFiles[0];
      break;
  }
  return (file !== undefined && petAssetUrl(file)) || IDLE_FALLBACK_URL;
}

const machine = createPetStateMachine({
  durations: {
    "poke-left": reactionDuration("poke-left"),
    "poke-right": reactionDuration("poke-right"),
    annoyed: reactionDuration("annoyed"),
    flail: reactionDuration("flail"),
    dizzy: Math.max(theme.minDisplay, theme.dizzyDuration),
    happy: Math.max(theme.minDisplay, theme.happyDuration),
  },
});

const currentState = ref<PetState>(machine.state);

// 双缓冲交叉淡入淡出：新图 load 完成后淡入，旧图淡出后移除
interface Layer {
  id: number;
  url: string;
  /** 眼睛跟随状态的 SVG 走内联通道（v-html 注入，SVG DOM 直接可操作） */
  kind: "img" | "inline";
  /** kind === "inline" 时的 SVG 文本 */
  markup?: string;
  visible: boolean;
}
const layers = ref<Layer[]>([]);
let layerSeq = 0;
/** 乱序竞态守卫：仅最后一次 swapTo 的 probe 回调生效 */
let swapSeq = 0;
/** 卸载后屏蔽 in-flight probe 回调与 setTimeout */
let unmounted = false;
/** idle 兜底图也加载失败时显示 CSS 占位圆 */
const idleAssetBroken = ref(false);

function swapTo(url: string, forceRestart = false) {
  // 同 URL 短路：idle 单图主题每次 oneshotEnd 回到 idle 时避免无意义交叉淡入闪烁
  // forceRestart 时跳过短路（oneshot 同状态重触发需重建 img 层让 SVG 动画从头重播）
  if (!forceRestart) {
    const topLayer = layers.value[layers.value.length - 1];
    if (topLayer && topLayer.visible && topLayer.url === url) return;
  }

  const seq = ++swapSeq;
  // 加层：前层淡出，新层淡入，400ms 后清理前层
  const pushLayer = (layer: Omit<Layer, "id" | "visible">) => {
    if (unmounted || seq !== swapSeq) return;
    if (layer.url === IDLE_FALLBACK_URL) {
      idleAssetBroken.value = false;
    }
    const id = ++layerSeq;
    const previous = layers.value;
    previous.forEach((l) => {
      l.visible = false;
    });
    layers.value = [...previous, { ...layer, id, visible: true }];
    const previousIds = new Set(previous.map((l) => l.id));
    // 清理延迟 400ms 必须大于下方 CSS 过渡 0.25s，后台标签页定时器钳制下仍保留裕量
    setTimeout(() => {
      if (unmounted) return;
      layers.value = layers.value.filter((l) => !previousIds.has(l.id));
    }, 400);
    // 新层入 DOM 后刷新眼睛 attach（覆盖启动首层等不经 machine.onChange 的路径）
    void nextTick(refreshEyeTargets);
  };
  const onProbeError = () => {
    if (unmounted || seq !== swapSeq) return;
    if (url !== IDLE_FALLBACK_URL) {
      swapTo(IDLE_FALLBACK_URL);
    } else {
      idleAssetBroken.value = true;
    }
  };
  if (INLINE_SVG_URLS.has(url)) {
    fetchSvgMarkup(url)
      .then((markup) => pushLayer({ url, kind: "inline", markup }))
      .catch(onProbeError);
    return;
  }
  const probe = new Image();
  probe.onload = () => pushLayer({ url, kind: "img" });
  probe.onerror = onProbeError;
  probe.src = url;
}

const unsubscribeMachine = machine.onChange((state) => {
  const isRetrigger = state === currentState.value;
  currentState.value = state;
  swapTo(urlForState(state), isRetrigger);
  void nextTick(refreshEyeTargets);
});

const { handlers } = usePetInteraction(machine, mini);

let unlistenPtyStatus: UnlistenFn | null = null;

const aggregator = createSessionStateAggregator({
  onBaseChange: (base) => machine.setBaseState(base),
  onCelebrate: () => machine.dispatch({ type: "happy" }),
});

// mini 进出与 hover 探出时换图（machine 状态不变，需独立监听）
watch([mini.active, mini.peeking], () => {
  swapTo(urlForState(machine.state));
  void nextTick(refreshEyeTargets);
});

// ---- 眼睛跟随 ----
const petRootEl = ref<HTMLElement | null>(null);
let eyeTargets: EyeTargets | null = null;
let unlistenEyeMove: UnlistenFn | null = null;
let unlistenDizzy: UnlistenFn | null = null;
let lastEyeDx = 0;
let lastEyeDy = 0;

/** 当前渲染上下文是否处于主题声明的眼睛跟随状态 */
function isEyeTrackingContext(): boolean {
  if (!theme.eyeTracking.enabled) return false;
  const stateName = mini.active.value
    ? mini.peeking.value
      ? "mini-peek"
      : "mini-idle"
    : machine.state;
  return theme.eyeTracking.states.includes(stateName);
}

/**
 * 根据当前上下文 attach/detach 眼睛目标。
 * 触发点：machine 状态变化、mini 变化（换层后的下一个 tick）。
 * 必须限定在可见层子树内查询：交叉淡入期间旧层未清理，全局查 id 会命中重复副本。
 */
function refreshEyeTargets() {
  eyeTargets = null;
  if (unmounted || !isEyeTrackingContext()) return;
  const layer = petRootEl.value?.querySelector<HTMLElement>(".pet-layer.visible");
  if (!layer) return;
  eyeTargets = attachEyeTargets(layer, theme.eyeTracking.ids);
  if (eyeTargets) {
    // 恢复上次偏移，避免换层后眼睛回中跳变
    applyEyeMove(eyeTargets, lastEyeDx, lastEyeDy, theme.eyeTracking);
  }
}

onMounted(() => {
  document.documentElement.classList.add("desk-pet-mode");
  document.body.classList.add("desk-pet-mode");
  swapTo(urlForState(machine.state));
  void listen<[number, number]>("desk-pet-eye-move", (e) => {
    [lastEyeDx, lastEyeDy] = e.payload;
    if (eyeTargets) {
      applyEyeMove(eyeTargets, lastEyeDx, lastEyeDy, theme.eyeTracking);
    }
  }).then((fn) => {
    if (unmounted) {
      fn();
    } else {
      unlistenEyeMove = fn;
    }
  });
  void listen("desk-pet-dizzy", () => {
    machine.dispatch({ type: "dizzy" });
  }).then((fn) => {
    if (unmounted) {
      fn();
    } else {
      unlistenDizzy = fn;
    }
  });
  void listen<PtyStatusChangedPayload>("pty-status-changed", (e) => {
    aggregator.applyStatus(e.payload.sessionId, e.payload.status);
  }).then((fn) => {
    if (unmounted) {
      fn();
    } else {
      unlistenPtyStatus = fn;
    }
  });
  void invoke<PtySessionInfo[]>("list_pty_sessions")
    .then((list) => {
      if (!unmounted) {
        aggregator.applySnapshot(list);
      }
    })
    .catch(() => {
      // 快照失败从空 Map 开始，后续事件自愈
    });
  if (theme.eyeTracking.enabled) {
    void invoke("start_desk_pet_eye_track", {
      eyeRatioX: theme.eyeTracking.eyeRatioX,
      eyeRatioY: theme.eyeTracking.eyeRatioY,
      maxOffset: theme.eyeTracking.maxOffset,
    });
  }
});

onUnmounted(() => {
  unmounted = true;
  unlistenEyeMove?.();
  unlistenDizzy?.();
  unlistenPtyStatus?.();
  void invoke("stop_desk_pet_eye_track");
  unsubscribeMachine();
  machine.dispose();
  mini.dispose();
  document.documentElement.classList.remove("desk-pet-mode");
  document.body.classList.remove("desk-pet-mode");
});
</script>

<template>
  <div
    ref="petRootEl"
    class="pet-root"
    :class="{ 'facing-left': currentState === 'drag-left' || (mini.active.value && mini.edge.value === 'left') }"
    @contextmenu.prevent
    @pointerdown="handlers.onPointerdown"
    @pointermove="handlers.onPointermove"
    @pointerup="handlers.onPointerup"
    @pointercancel="handlers.onPointerup"
  >
    <div v-if="idleAssetBroken" class="pet-placeholder"></div>
    <template v-for="layer in layers" :key="layer.id">
      <div
        v-if="layer.kind === 'inline'"
        class="pet-layer"
        :class="{ visible: layer.visible }"
        v-html="layer.markup"
      ></div>
      <img
        v-else
        :src="layer.url"
        class="pet-layer"
        :class="{ visible: layer.visible }"
        draggable="false"
        alt=""
      />
    </template>
  </div>
</template>

<style>
/* 非 scoped：桌宠窗口需要全局透明背景（参照 token-widget-mode 的处理方式） */
html.desk-pet-mode,
body.desk-pet-mode,
body.desk-pet-mode #app {
  background: transparent !important;
}
</style>

<style scoped>
.pet-root {
  position: fixed;
  inset: 0;
  overflow: hidden;
  user-select: none;
  -webkit-user-select: none;
  cursor: grab;
  touch-action: none;
}
.pet-root:active {
  cursor: grabbing;
}
.pet-layer {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: contain;
  opacity: 0;
  /* 与 swapTo 中 400ms 清理 setTimeout 耦合：改此值需同步保证清理延迟大于过渡时长 */
  transition: opacity 0.25s ease-in-out;
  pointer-events: none;
}
.pet-layer.visible {
  opacity: 1;
}
/* 内联 SVG 自带 width="500" height="500" 属性，需撑满层 */
.pet-layer :deep(svg) {
  width: 100%;
  height: 100%;
  display: block;
}
.facing-left .pet-layer {
  transform: scaleX(-1);
}
.pet-placeholder {
  position: absolute;
  left: 50%;
  top: 50%;
  width: 96px;
  height: 96px;
  transform: translate(-50%, -50%);
  border-radius: 50%;
  background: radial-gradient(circle at 35% 35%, #ffb27d, #e2633a);
}
</style>
