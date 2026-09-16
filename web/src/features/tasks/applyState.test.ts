import { describe, expect, it } from 'vitest'
import { summarizeInstanceApply, unappliedNoticeLines } from './applyState'

describe('summarizeInstanceApply', () => {
  it('全部实例生效时保持静默态势', () => {
    const summary = summarizeInstanceApply([
      { algorithmId: 'a', enabled: true, applyState: 'applied' },
      { algorithmId: 'b', enabled: true, applyState: 'applied' },
    ])

    expect(summary.tone).toBe('ok')
    expect(summary.unapplied).toEqual([])
  })

  it('缺失 applyState 视为已生效，不把未知状态渲染成故障', () => {
    const summary = summarizeInstanceApply([{ algorithmId: 'a', enabled: true }])

    expect(summary.tone).toBe('ok')
  })

  it('停用实例不参与判定', () => {
    const summary = summarizeInstanceApply([
      { algorithmId: 'a', enabled: false, applyState: 'pending', statusMessage: '等待挂载' },
      { algorithmId: 'b', enabled: true, applyState: 'applied' },
    ])

    expect(summary.tone).toBe('ok')
    expect(summary.unapplied).toEqual([])
  })

  it('存在未生效实例时返回最严重态势与可读原因', () => {
    const summary = summarizeInstanceApply([
      {
        algorithmId: 'face_recognition',
        enabled: true,
        applyState: 'failed',
        statusMessage: '创建推理 Worker 失败: 设备内存不足',
      },
      {
        algorithmId: 'smoking',
        enabled: true,
        applyState: 'pending',
        statusMessage: '等待帧边界切换',
      },
      { algorithmId: 'general_detection', enabled: true, applyState: 'applied' },
    ])

    expect(summary.tone).toBe('failed')
    expect(summary.unapplied).toEqual([
      {
        algorithmId: 'face_recognition',
        tone: 'failed',
        reason: '创建推理 Worker 失败: 设备内存不足',
      },
      { algorithmId: 'smoking', tone: 'pending', reason: '等待帧边界切换' },
    ])
  })

  it('仅有排队中实例时态势为 pending', () => {
    const summary = summarizeInstanceApply([
      { algorithmId: 'a', enabled: true, applyState: 'pending', statusMessage: '等待运行时挂载' },
    ])

    expect(summary.tone).toBe('pending')
    expect(summary.unapplied).toHaveLength(1)
  })

  it('同级未生效实例按算法标识稳定排序', () => {
    const summary = summarizeInstanceApply([
      { algorithmId: 'zeta', enabled: true, applyState: 'failed' },
      { algorithmId: 'alpha', enabled: true, applyState: 'failed' },
    ])

    expect(summary.unapplied.map((item) => item.algorithmId)).toEqual(['alpha', 'zeta'])
  })

  it('空集合与空原因都不产生噪声', () => {
    expect(summarizeInstanceApply([]).tone).toBe('ok')

    const summary = summarizeInstanceApply([
      { algorithmId: 'a', enabled: true, applyState: 'failed', statusMessage: '   ' },
    ])
    expect(summary.unapplied[0].reason).toBe('')
    expect(unappliedNoticeLines(summary)).toEqual(['a'])
  })
})

describe('unappliedNoticeLines', () => {
  it('逐行输出算法标识与服务端原因，不做前端文案拼接', () => {
    const lines = unappliedNoticeLines({
      tone: 'failed',
      unapplied: [
        { algorithmId: 'face', tone: 'failed', reason: '设备内存不足' },
        { algorithmId: 'smoking', tone: 'pending', reason: '' },
      ],
    })

    expect(lines).toEqual(['face: 设备内存不足', 'smoking'])
  })
})
