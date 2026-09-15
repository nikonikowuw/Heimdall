import { useCallback, useMemo, useReducer } from 'react'
import {
  createHistory,
  historyReducer,
  type HistoryAction,
  type HistorySnapshot,
} from './ruleHistory'

export interface UndoableState<T> {
  /** 当前状态 */
  value: T
  /**
   * 更新状态。默认入栈可撤销；`history: false` 用于拖拽中间帧，
   * 需配合 `beginGesture` / `endGesture` 保证一次手势只产生一条历史。
   */
  set: (next: T | ((prev: T) => T), options?: { history?: boolean }) => void
  beginGesture: () => void
  endGesture: () => void
  undo: () => void
  redo: () => void
  /** 重置状态并清空历史（如首次加载远端配置） */
  reset: (next: T) => void
  canUndo: boolean
  canRedo: boolean
}

/**
 * 带撤销/重做能力的受控状态。
 * 历史栈有界，见 `ruleHistory.ts`；拖拽等连续手势通过 gesture API 合并为单条历史。
 */
export function useUndoableState<T>(initial: T): UndoableState<T> {
  const reducer = useCallback(
    (state: HistorySnapshot<T>, action: HistoryAction<T>) => historyReducer(state, action),
    [],
  )
  const [snapshot, dispatch] = useReducer(reducer, initial, createHistory)

  const set = useCallback(
    (next: T | ((prev: T) => T), options?: { history?: boolean }) => {
      dispatch({ type: options?.history === false ? 'replace' : 'commit', next })
    },
    [dispatch],
  )
  const beginGesture = useCallback(() => dispatch({ type: 'gestureStart' }), [dispatch])
  const endGesture = useCallback(() => dispatch({ type: 'gestureEnd' }), [dispatch])
  const undo = useCallback(() => dispatch({ type: 'undo' }), [dispatch])
  const redo = useCallback(() => dispatch({ type: 'redo' }), [dispatch])
  const reset = useCallback((next: T) => dispatch({ type: 'reset', next }), [dispatch])

  return useMemo(
    () => ({
      value: snapshot.present,
      set,
      beginGesture,
      endGesture,
      undo,
      redo,
      reset,
      canUndo: snapshot.past.length > 0,
      canRedo: snapshot.future.length > 0,
    }),
    [
      snapshot.present,
      snapshot.past.length,
      snapshot.future.length,
      set,
      beginGesture,
      endGesture,
      undo,
      redo,
      reset,
    ],
  )
}
