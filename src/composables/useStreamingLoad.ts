import { ref, shallowRef, triggerRef, onUnmounted, getCurrentInstance, type Ref, type ShallowRef } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

interface StreamError {
  message: string
}

export interface UseStreamingLoadReturn<TChunk, TDone> {
  /** 累积的数据项（chunk 是 batch，items 是 flat 数组） */
  items: ShallowRef<TChunk[]>
  /** done 事件的 payload；未完成时为 null */
  done: Ref<TDone | null>
  /** 错误对象；正常时为 null */
  error: Ref<Error | null>
  /** 正在加载中（start 调用后直到收到 done 或 error） */
  loading: Ref<boolean>
  /**
   * 启动一次流式加载。会自动清理上一次的 listener。
   * - `commandSuffix`: 后端命令名（不含 `_stream`，例如 `'load_session'`）
   * - `topic`: 事件 topic（与后端约定，如 `'session'`）
   * - `args`: 透传给后端命令的参数；`requestId` 由 composable 自动生成
   */
  start: (
    commandSuffix: string,
    topic: string,
    args: Record<string, unknown>
  ) => Promise<void>
  /** 主动放弃当前流（清理监听 + reset 状态）。不会通知后端，后端继续跑完丢弃 emit */
  cancel: () => void
}

/**
 * 必须在组件 `setup()` 同步阶段调用，依赖 `onUnmounted` 自动清理监听器。
 * 在非 setup 环境（store/外层模块）下使用时需自行调用 cancel()。
 */
export function useStreamingLoad<TChunk, TDone>(): UseStreamingLoadReturn<TChunk, TDone> {
  const items = shallowRef<TChunk[]>([])
  const done = ref<TDone | null>(null) as Ref<TDone | null>
  const error = ref<Error | null>(null)
  const loading = ref(false)

  let unlistenChunk: UnlistenFn | null = null
  let unlistenDone: UnlistenFn | null = null
  let unlistenError: UnlistenFn | null = null
  let currentRequestId: string | null = null

  function cleanupListeners() {
    if (unlistenChunk) { unlistenChunk(); unlistenChunk = null }
    if (unlistenDone) { unlistenDone(); unlistenDone = null }
    if (unlistenError) { unlistenError(); unlistenError = null }
    currentRequestId = null
  }

  function cancel() {
    cleanupListeners()
    loading.value = false
  }

  async function start(
    commandSuffix: string,
    topic: string,
    args: Record<string, unknown>
  ): Promise<void> {
    // 1. 清掉上一次的 listener 并重置状态
    cleanupListeners()
    items.value = []
    done.value = null
    error.value = null
    loading.value = true

    // 2. 生成 request_id 并订阅事件
    const id = crypto.randomUUID()
    currentRequestId = id
    const chunkEvent = `${topic}:${id}:chunk`
    const doneEvent = `${topic}:${id}:done`
    const errorEvent = `${topic}:${id}:error`

    const [chunkUn, doneUn, errUn] = await Promise.all([
      listen<TChunk[]>(chunkEvent, (ev) => {
        if (currentRequestId !== id) return
        // shallowRef + 原地 push + triggerRef：O(B) per chunk，避免 concat 整列表 O(N²) 累积
        items.value.push(...ev.payload)
        triggerRef(items)
      }),
      listen<TDone>(doneEvent, (ev) => {
        if (currentRequestId !== id) return
        done.value = ev.payload
        loading.value = false
        cleanupListeners()
      }),
      listen<StreamError>(errorEvent, (ev) => {
        if (currentRequestId !== id) return
        error.value = new Error(ev.payload.message)
        loading.value = false
        cleanupListeners()
      }),
    ])

    // 三个 listener 注册期间如果 start/cancel/unmount 抢跑了，立即丢弃
    if (currentRequestId !== id) {
      chunkUn(); doneUn(); errUn()
      return
    }
    unlistenChunk = chunkUn
    unlistenDone = doneUn
    unlistenError = errUn

    // 3. 调用后端，注意必须等 listener 注册完毕后再 invoke
    try {
      await invoke(`${commandSuffix}_stream`, { requestId: id, ...args })
    } catch (e: unknown) {
      if (currentRequestId !== id) return
      error.value = e instanceof Error ? e : new Error(String(e))
      loading.value = false
      cleanupListeners()
    }
  }

  if (getCurrentInstance()) {
    onUnmounted(() => {
      cancel()
    })
  }

  return { items, done, error, loading, start, cancel }
}
