/**
 * 有界撤销/重做历史栈（纯逻辑，便于单测）。
 *
 * 设计要点：
 * - `commit` 用于一次性变更（新增/删除规则、改方向），立即入栈；
 * - `replace` 用于拖拽等连续手势的中间帧，不入栈；
 * - 手势开始 (`gestureStart`) 记录基线，手势结束 (`gestureEnd`) 才入栈一次，
 *   避免一次拖拽产生上百条历史记录；
 * - 历史长度有上限，超出后丢弃最旧记录，内存有界。
 */

export const HISTORY_LIMIT = 50

export interface HistorySnapshot<T> {
  past: T[]
  present: T
  future: T[]
  /** 连续手势的起始基线；非空表示手势进行中 */
  gestureBase: T | null
}

export type HistoryAction<T> =
  | { type: 'commit'; next: T | ((prev: T) => T) }
  | { type: 'replace'; next: T | ((prev: T) => T) }
  | { type: 'gestureStart' }
  | { type: 'gestureEnd' }
  | { type: 'undo' }
  | { type: 'redo' }
  | { type: 'reset'; next: T }

export function createHistory<T>(present: T): HistorySnapshot<T> {
  return { past: [], present, future: [], gestureBase: null }
}

function resolve<T>(next: T | ((prev: T) => T), prev: T): T {
  return typeof next === 'function' ? (next as (prev: T) => T)(prev) : next
}

function pushBounded<T>(past: T[], snapshot: T): T[] {
  const next =
    past.length >= HISTORY_LIMIT ? past.slice(past.length - HISTORY_LIMIT + 1) : [...past]
  next.push(snapshot)
  return next
}

export function historyReducer<T>(
  state: HistorySnapshot<T>,
  action: HistoryAction<T>,
): HistorySnapshot<T> {
  switch (action.type) {
    case 'commit': {
      const next = resolve(action.next, state.present)
      if (Object.is(next, state.present)) return state
      return {
        past: pushBounded(state.past, state.present),
        present: next,
        future: [],
        gestureBase: null,
      }
    }
    case 'replace': {
      const next = resolve(action.next, state.present)
      if (Object.is(next, state.present)) return state
      return { ...state, present: next }
    }
    case 'gestureStart':
      return state.gestureBase === null ? { ...state, gestureBase: state.present } : state
    case 'gestureEnd': {
      if (state.gestureBase === null) return state
      if (Object.is(state.gestureBase, state.present)) return { ...state, gestureBase: null }
      return {
        past: pushBounded(state.past, state.gestureBase),
        present: state.present,
        future: [],
        gestureBase: null,
      }
    }
    case 'undo': {
      if (state.past.length === 0) return state
      const previous = state.past[state.past.length - 1]
      return {
        past: state.past.slice(0, -1),
        present: previous,
        future: [state.present, ...state.future],
        gestureBase: null,
      }
    }
    case 'redo': {
      if (state.future.length === 0) return state
      const [next, ...rest] = state.future
      return {
        past: pushBounded(state.past, state.present),
        present: next,
        future: rest,
        gestureBase: null,
      }
    }
    case 'reset':
      return createHistory(action.next)
  }
}
