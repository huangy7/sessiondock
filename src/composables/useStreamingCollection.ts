import { ref, shallowRef, triggerRef, watch, type Ref, type ShallowRef } from 'vue'
import { useStreamingLoad } from './useStreamingLoad'

export interface UseStreamingCollectionOptions {
  /** 后端命令名（不含 `_stream`），如 `'scan_projects'` */
  command: string
  /** 事件 topic（与后端约定），通常等同于 command */
  topic: string
  /** 显示 loading 指示器的延迟阈值；<= 此值完成的快路径完全沉默；默认 300ms */
  slowThresholdMs?: number
}

export interface StreamingRefreshOptions {
  /**
   * 静默模式（后台定时刷新/文件监听触发）：已有数据时绝不清空列表、
   * 绝不显示 loading pill，新数据到达后原子替换。仅影响展示层，
   * done with 0 chunk 的真清空语义保持不变（数据确实为空时仍会清空）。
   */
  silent?: boolean
}

export interface UseStreamingCollectionReturn<TItem, TDone> {
  /** 累计数据项；在数据到达前保留上一次的 items，新数据原子替换 */
  items: ShallowRef<TItem[]>
  /** done 事件 payload；refresh 启动时重置为 null */
  done: Ref<TDone | null>
  /** 错误对象 */
  error: Ref<Error | null>
  /** refresh 全程置位的忙碌标志，用于覆盖空状态门控 */
  isRefreshing: Ref<boolean>
  /** 仅在 >slowThresholdMs 仍未收到 chunk 时变 true；快路径永不显示 */
  showLoadingIndicator: Ref<boolean>
  /** 当前已累计 item 数（item 级别，与分组无关） */
  totalItems: Ref<number>
  /** 启动一次 refresh；await 直到 done 或 error 之一到达 */
  refresh: (args?: Record<string, unknown>, opts?: StreamingRefreshOptions) => Promise<void>
  /** 主动取消当前 refresh */
  cancel: () => void
}

/**
 * 流式列表加载通用编排：包装 useStreamingLoad，提供
 * - 慢加载阈值控制的 loading 指示器（避免快路径下 pill 闪烁）
 * - 数据原子替换语义（CLI 切换等场景旧列表保持显示，新数据到达瞬时 swap）
 * - done with 0 chunk 才真清空（区分"加载失败保留旧数据" vs "确实空"）
 * - error 时若一条 chunk 未收到则不动 items（隐式回滚到上次成功状态）
 *
 * **生命周期使用规范**：
 * - 组件 setup 同步阶段调用：依赖 `onUnmounted` 自动 cancel（推荐）
 * - 模块级 / store 风格调用（如 useSessions 单例 store）：getCurrentInstance 返回 null，
 *   `onUnmounted` 注册被跳过；listener 不会自动清理。但 useStreamingLoad.start 内部每次
 *   都会先 cleanupListeners()，所以**不会**累积 listener，单例 store 整个进程生命周期内不需
 *   要清理也安全。代价是失去 unmount 时取消能力——若调用方场景需要主动 cancel 必须自管。
 */
export function useStreamingCollection<TItem, TDone>(
  opts: UseStreamingCollectionOptions
): UseStreamingCollectionReturn<TItem, TDone> {
  const stream = useStreamingLoad<TItem, TDone>()
  const slowThresholdMs = opts.slowThresholdMs ?? 300

  const items = shallowRef<TItem[]>([])
  const done = ref<TDone | null>(null) as Ref<TDone | null>
  const error = ref<Error | null>(null)
  const isRefreshing = ref(false)
  const showLoadingIndicator = ref(false)
  const totalItems = ref(0)

  // 当前在飞 refresh 的 completion resolver；同时承担两个职责：
  // 1) cancel() 调用时 resolve 它，解除 refresh 的 await completion 挂起 → 触发 finally 清理
  // 2) 新 refresh 入口抢占式 resolve 它，让旧 refresh 立刻跑 finally 释放 watcher，避免新旧并存
  let activeResolveCompletion: (() => void) | null = null

  function cancel() {
    if (activeResolveCompletion) {
      activeResolveCompletion()
      activeResolveCompletion = null
    }
    stream.cancel()
    isRefreshing.value = false
    showLoadingIndicator.value = false
  }

  async function refresh(
    args: Record<string, unknown> = {},
    refreshOpts: StreamingRefreshOptions = {}
  ): Promise<void> {
    // 抢占：取消任何在飞的 refresh，让它的 finally 立刻清理 watches
    if (activeResolveCompletion) {
      activeResolveCompletion()
    }

    const silent = refreshOpts.silent ?? false
    isRefreshing.value = true
    done.value = null
    error.value = null
    let receivedAny = false

    const slowTimer = setTimeout(() => {
      // 静默刷新保留旧列表：后台刷新慢是常态（大目录扫描），
      // 清空列表 + pill 会造成每次定时刷新都"一闪一闪"
      if (!receivedAny && !silent) {
        // 真的慢了：清空旧 items + 显示 pill
        items.value = []
        totalItems.value = 0
        showLoadingIndicator.value = true
      }
    }, slowThresholdMs)

    let resolveCompletion!: () => void
    const completion = new Promise<void>((r) => { resolveCompletion = r })
    activeResolveCompletion = resolveCompletion

    // 标记 stream 刚被 start() 重置（length 短暂为 0）；下一次非空 chunk 来时整体 swap
    let pendingReset = false

    // 直接监听 stream 的三个反应式状态；items.length === 0 时早返回避免 start 重置造成的闪烁
    const stop1 = watch(
      stream.items,
      (streamItems) => {
        if (streamItems.length === 0) {
          // useStreamingLoad.start() 抢占时会先把 stream.items 重置为 []。
          // early-return 保留旧 items 避免抖动；用 pendingReset 标记，下一非空 chunk 触发整体 swap。
          // 真正的"显示空"由 done with !receivedAny 分支处理。
          pendingReset = true
          return
        }
        receivedAny = true
        clearTimeout(slowTimer)
        if (!silent) {
          showLoadingIndicator.value = false
        }

        if (pendingReset) {
          // 新一次 refresh 的第一条 chunk：整体 swap，避免新旧 chunk 内容长度恰好对齐时的混入
          items.value = streamItems.slice()
          pendingReset = false
        } else {
          // 正常增量：从 items.value.length 之后的元素 push 进来
          const delta = streamItems.slice(items.value.length)
          if (delta.length > 0) {
            items.value.push(...delta)
            triggerRef(items)
          }
        }
        totalItems.value = items.value.length
      },
      { flush: 'sync' }
    )

    const stop2 = watch(
      stream.done,
      (d) => {
        if (!d) return
        done.value = d
        // done 时若一条 chunk 未收到 → 真清空旧 items
        if (!receivedAny) {
          items.value = []
          totalItems.value = 0
        }
        clearTimeout(slowTimer)
        showLoadingIndicator.value = false
        resolveCompletion()
      },
      { flush: 'sync' }
    )

    const stop3 = watch(
      stream.error,
      (e) => {
        if (!e) return
        error.value = e
        // 错误时若一条 chunk 未收到 → 不动 items（保留旧数据）
        clearTimeout(slowTimer)
        showLoadingIndicator.value = false
        resolveCompletion()
      },
      { flush: 'sync' }
    )

    try {
      await stream.start(opts.command, opts.topic, args)
      await completion
    } finally {
      // 只在仍是本次 refresh 持有 slot 时清掉，避免抢断新 refresh 已注册的 resolver
      if (activeResolveCompletion === resolveCompletion) {
        activeResolveCompletion = null
      }
      clearTimeout(slowTimer)
      showLoadingIndicator.value = false
      isRefreshing.value = false
      stop1()
      stop2()
      stop3()
    }
  }

  return {
    items,
    done,
    error,
    isRefreshing,
    showLoadingIndicator,
    totalItems,
    refresh,
    cancel,
  }
}
