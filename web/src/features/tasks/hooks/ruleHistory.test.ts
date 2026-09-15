import { describe, expect, it } from 'vitest'
import { createHistory, historyReducer, HISTORY_LIMIT, type HistorySnapshot } from './ruleHistory'

function reduce<T>(
  state: HistorySnapshot<T>,
  ...actions: Parameters<typeof historyReducer<T>>[1][]
) {
  return actions.reduce((acc, action) => historyReducer(acc, action), state)
}

describe('ruleHistory', () => {
  it('starts empty and exposes present value', () => {
    const state = createHistory(['a'])
    expect(state.present).toEqual(['a'])
    expect(state.past).toEqual([])
    expect(state.future).toEqual([])
  })

  it('commits a change onto the past stack and clears redo', () => {
    const state = reduce(
      createHistory(['a']),
      { type: 'commit', next: ['a', 'b'] },
      { type: 'undo' },
      { type: 'commit', next: ['a', 'c'] },
    )
    expect(state.present).toEqual(['a', 'c'])
    expect(state.past).toEqual([['a']])
    expect(state.future).toEqual([])
  })

  it('no-ops when the committed value is identical', () => {
    const state = createHistory(['a'])
    const current = state.present
    expect(historyReducer(state, { type: 'commit', next: current })).toBe(state)
  })

  it('supports functional updates', () => {
    const state = historyReducer(createHistory(['a']), {
      type: 'commit',
      next: (prev) => [...prev, 'b'],
    })
    expect(state.present).toEqual(['a', 'b'])
  })

  it('undo restores the previous value and redo replays it', () => {
    const committed = reduce(
      createHistory(['a']),
      { type: 'commit', next: ['a', 'b'] },
      { type: 'commit', next: ['a', 'b', 'c'] },
    )
    const undone = historyReducer(committed, { type: 'undo' })
    expect(undone.present).toEqual(['a', 'b'])
    expect(undone.future).toEqual([['a', 'b', 'c']])

    const redone = historyReducer(undone, { type: 'redo' })
    expect(redone.present).toEqual(['a', 'b', 'c'])
    expect(redone.future).toEqual([])
  })

  it('ignores undo/redo when the respective stack is empty', () => {
    const state = createHistory(['a'])
    expect(historyReducer(state, { type: 'undo' })).toBe(state)
    expect(historyReducer(state, { type: 'redo' })).toBe(state)
  })

  it('collapses a whole drag gesture into a single history entry', () => {
    const start = createHistory(['v0'])
    const dragging = reduce(
      start,
      { type: 'gestureStart' },
      { type: 'replace', next: ['v1'] },
      { type: 'replace', next: ['v2'] },
      { type: 'replace', next: ['v3'] },
    )
    expect(dragging.past).toEqual([])
    expect(dragging.present).toEqual(['v3'])

    const finished = historyReducer(dragging, { type: 'gestureEnd' })
    expect(finished.past).toEqual([['v0']])
    expect(finished.gestureBase).toBeNull()

    const undone = historyReducer(finished, { type: 'undo' })
    expect(undone.present).toEqual(['v0'])
  })

  it('drops a gesture that never changed the value', () => {
    const state = reduce(createHistory(['v0']), { type: 'gestureStart' }, { type: 'gestureEnd' })
    expect(state.past).toEqual([])
  })

  it('ignores nested gestureStart and gestureEnd without a gesture', () => {
    const started = historyReducer(createHistory(['v0']), { type: 'gestureStart' })
    const startedAgain = historyReducer(started, { type: 'gestureStart' })
    expect(startedAgain.gestureBase).toBe(started.gestureBase)

    const lonely = historyReducer(createHistory(['v0']), { type: 'gestureEnd' })
    expect(lonely.past).toEqual([])
  })

  it('keeps the history bounded', () => {
    let state = createHistory(0)
    for (let i = 1; i <= HISTORY_LIMIT + 20; i += 1) {
      state = historyReducer(state, { type: 'commit', next: i })
    }
    expect(state.past).toHaveLength(HISTORY_LIMIT)
    expect(state.present).toBe(HISTORY_LIMIT + 20)
    // 最旧的历史已被丢弃，仅保留最近 HISTORY_LIMIT 条
    expect(state.past[0]).toBe(20)
  })

  it('reset replaces value and clears both stacks', () => {
    const state = reduce(
      createHistory(['a']),
      { type: 'commit', next: ['a', 'b'] },
      { type: 'undo' },
      { type: 'reset', next: ['loaded'] },
    )
    expect(state).toEqual({ past: [], present: ['loaded'], future: [], gestureBase: null })
  })
})
