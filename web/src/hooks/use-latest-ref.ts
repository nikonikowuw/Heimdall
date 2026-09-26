import { useEffect, useRef, type RefObject } from 'react'

/**
 * 「最新值快照」ref：把渲染期的值暴露给渲染之外的回调（rAF 循环、WS 订阅、
 * DOM 事件、effect 清理函数），而不必把这些值放进订阅的依赖数组。
 *
 * ## 为什么必须是 `effect` 同步，而不是 render 期直接赋值
 *
 * 常见但错误的写法是在渲染期直接赋值：
 *
 * ```ts
 * const ref = useRef(value)
 * ref.current = value // ❌ 渲染期写 ref
 * ```
 *
 * 这在并发渲染下不成立：React 可能开始一次渲染后又丢弃它（被更高优先级更新打断，
 * 或处于 StrictMode 的双调用中），此时 ref 已被改成「那次被丢弃的渲染」的值，
 * 而该值从未提交到 DOM。渲染必须是纯净的，写 ref 属于副作用，只能在提交后执行。
 *
 * 用 `effect` 同步语义等价（effect 在提交后运行，晚于本组件所有渲染），
 * 且不再依赖「渲染是否会被丢弃」这一实现细节。
 *
 * ## 与依赖数组的关系
 *
 * 高频变化的值（搜索关键字、通道名映射、遥测数据）若进订阅 effect 的依赖数组，
 * 每次变化都会拆建订阅。快照 ref 让订阅只依赖「订阅目标本身」，回调内读取最新值。
 * 参见 [AlarmsPage 的 LiveFilterSnapshot](../../features/alarms/AlarmsPage.tsx)，
 * 以及 `docs/nuwa/frontend/hook-guidelines.md` 的「长生命周期订阅与高频依赖」。
 *
 * ## 使用约束
 *
 * 返回的 ref 只在**渲染之外**读取才有意义。在渲染期读取它拿到的是上一次提交的值，
 * 会让组件输出与 props/state 脱节（这正是 `react-hooks/refs` 要拦截的模式）。
 */
export function useLatestRef<T>(value: T): RefObject<T> {
  const ref = useRef(value)

  useEffect(() => {
    ref.current = value
  }, [value])

  return ref
}
