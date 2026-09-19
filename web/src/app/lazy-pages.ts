import { lazy } from 'react'

/**
 * 工作区页面的路由级代码分割边界。
 *
 * 边界取舍：
 * - `LoginPage` 刻意不在此处 —— 它是未鉴权用户的首次绘制内容，懒加载会在登录表单
 *   出现前引入一次额外 chunk 往返，属懒加载最不该出现的位置，故保持同步导入。
 * - 其余 8 个工作区页面全部按需加载，把 mpegts.js 等重依赖从首屏 chunk 剥离。
 *
 * 各页面均为具名导出，故以 `.then` 适配成 `default`。
 */

export const LivePage = lazy(() =>
  import('@/features/live/LivePage').then((m) => ({ default: m.LivePage })),
)

export const CamerasPage = lazy(() =>
  import('@/features/cameras/CamerasPage').then((m) => ({ default: m.CamerasPage })),
)

export const TasksPage = lazy(() =>
  import('@/features/tasks/TasksPage').then((m) => ({ default: m.TasksPage })),
)

export const AlgorithmsPage = lazy(() =>
  import('@/features/algorithms/AlgorithmsPage').then((m) => ({ default: m.AlgorithmsPage })),
)

export const PersonnelPage = lazy(() =>
  import('@/features/personnel/PersonnelPage').then((m) => ({ default: m.PersonnelPage })),
)

export const AlarmsPage = lazy(() =>
  import('@/features/alarms/AlarmsPage').then((m) => ({ default: m.AlarmsPage })),
)

export const OplogPage = lazy(() =>
  import('@/features/oplog/OplogPage').then((m) => ({ default: m.OplogPage })),
)

export const SettingsPage = lazy(() =>
  import('@/features/system/SettingsPage').then((m) => ({ default: m.SettingsPage })),
)

/**
 * 预取默认落地页（`live`）。
 *
 * 鉴权通过后工作区入场动画约 500ms，在此期间发起预取可让 chunk 提前就绪，
 * 使用户感知不到懒加载带来的额外往返。
 */
export function preloadDefaultTab(): void {
  void import('@/features/live/LivePage')
}
