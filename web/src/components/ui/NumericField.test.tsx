// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { renderToString } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { NumericField } from './NumericField'

afterEach(cleanup)

function getInput(label: string): HTMLInputElement {
  return screen.getByLabelText(label) as HTMLInputElement
}

describe('NumericField 编辑期草稿', () => {
  it('清空后可重输完整数值，中途不被强制回写，也不派发回调', () => {
    const onChange = vi.fn()
    render(<NumericField label="保留天数" value={7} min={1} max={365} onChange={onChange} />)
    const input = getInput('保留天数')

    expect(input.value).toBe('7')

    fireEvent.change(input, { target: { value: '' } })
    expect(input.value).toBe('')
    expect(onChange).not.toHaveBeenCalled()

    fireEvent.change(input, { target: { value: '3' } })
    expect(input.value).toBe('3')
    fireEvent.change(input, { target: { value: '30' } })
    expect(input.value).toBe('30')
    expect(onChange).not.toHaveBeenCalled()

    fireEvent.blur(input)
    expect(onChange).toHaveBeenCalledWith(30)
  })

  it('停留键盘中间态时不派发，取消编辑后回到上次合法值', () => {
    const onChange = vi.fn()
    render(<NumericField label="阈值" value={0.65} min={0} max={1} onChange={onChange} />)
    const input = getInput('阈值')

    for (const draft of ['-', '.', '-.']) {
      fireEvent.change(input, { target: { value: draft } })
      expect(input.value).toBe(draft)
      expect(onChange).not.toHaveBeenCalled()
    }

    fireEvent.blur(input)
    expect(input.value).toBe('0.65')
    expect(onChange).not.toHaveBeenCalled()
  })

  it('整型字段拒绝小数草稿，取整后回写', () => {
    const onChange = vi.fn()
    render(
      <NumericField
        label="端口"
        type="integer"
        value={5060}
        min={1}
        max={65535}
        onChange={onChange}
      />,
    )
    const input = getInput('端口')

    fireEvent.change(input, { target: { value: '12.5' } })
    expect(input.value).toBe('12.5')
    fireEvent.blur(input)
    expect(onChange).not.toHaveBeenCalled()
    expect(input.value).toBe('5060')

    fireEvent.change(input, { target: { value: '8080' } })
    fireEvent.blur(input)
    expect(onChange).toHaveBeenCalledWith(8080)
    expect(input.value).toBe('8080')
  })

  it('输入模式跟随数值语义', () => {
    render(
      <>
        <NumericField label="整数" type="integer" value={1} onChange={() => {}} />
        <NumericField label="小数" value={0.5} onChange={() => {}} />
      </>,
    )
    expect(getInput('整数').getAttribute('inputMode')).toBe('numeric')
    expect(getInput('小数').getAttribute('inputMode')).toBe('decimal')
    expect(getInput('整数').getAttribute('type')).toBe('text')
    expect(getInput('整数').getAttribute('autoComplete')).toBe('off')
  })

  it('不把语义边界落到 DOM，也不宣告 spinbutton 角色', () => {
    render(
      <NumericField
        label="端口"
        type="integer"
        value={5060}
        min={1}
        max={65535}
        onChange={() => {}}
      />,
    )
    const input = getInput('端口')

    expect(input.hasAttribute('min')).toBe(false)
    expect(input.hasAttribute('max')).toBe(false)
    expect(input.getAttribute('role')).toBeNull()
  })
})

describe('NumericField 失焦收敛 — 必填模式', () => {
  it('空串失焦回退上次合法值且不派发', () => {
    const onChange = vi.fn()
    render(<NumericField label="前录制" value={10} min={5} max={30} onChange={onChange} />)
    const input = getInput('前录制')

    fireEvent.change(input, { target: { value: '   ' } })
    fireEvent.blur(input)

    expect(input.value).toBe('10')
    expect(onChange).not.toHaveBeenCalled()
  })

  it('不可解析的非空草稿回退上次合法值且不派发', () => {
    const onChange = vi.fn()
    render(<NumericField label="前录制" value={10} min={5} max={30} onChange={onChange} />)
    const input = getInput('前录制')

    fireEvent.change(input, { target: { value: 'abc' } })
    fireEvent.blur(input)

    expect(input.value).toBe('10')
    expect(onChange).not.toHaveBeenCalled()
  })

  it('可解析且越界时收敛到边界并显示边界值', () => {
    const onChange = vi.fn()
    render(<NumericField label="单文件上限" value={300} min={30} max={3600} onChange={onChange} />)
    const input = getInput('单文件上限')

    fireEvent.change(input, { target: { value: '99999' } })
    fireEvent.blur(input)
    expect(onChange).toHaveBeenLastCalledWith(3600)
    expect(input.value).toBe('3600')

    fireEvent.change(input, { target: { value: '5' } })
    fireEvent.blur(input)
    expect(onChange).toHaveBeenLastCalledWith(30)
    expect(input.value).toBe('30')
  })

  it('区间内的可解析值原样派发', () => {
    const onChange = vi.fn()
    render(<NumericField label="保留天数" value={7} min={1} max={365} onChange={onChange} />)
    const input = getInput('保留天数')

    fireEvent.change(input, { target: { value: '120' } })
    fireEvent.blur(input)

    expect(onChange).toHaveBeenCalledWith(120)
    expect(input.value).toBe('120')
  })

  it('回车与失焦走同一条收敛路径', () => {
    const onChange = vi.fn()
    render(<NumericField label="保留天数" value={7} min={1} max={365} onChange={onChange} />)
    const input = getInput('保留天数')

    fireEvent.change(input, { target: { value: '0' } })
    fireEvent.keyDown(input, { key: 'Enter' })

    expect(onChange).toHaveBeenCalledWith(1)
    expect(input.value).toBe('1')
  })
})

describe('NumericField 失焦收敛 — 可空模式', () => {
  it('空串失焦派发 null 且界面保持空白', () => {
    const onChange = vi.fn()
    render(
      <NumericField
        label="子网前缀"
        type="integer"
        emptyBehavior="null"
        value={24}
        min={1}
        max={32}
        onChange={onChange}
      />,
    )
    const input = getInput('子网前缀')

    fireEvent.change(input, { target: { value: '' } })
    fireEvent.blur(input)

    expect(onChange).toHaveBeenCalledWith(null)
    expect(input.value).toBe('')
  })

  it('不可解析的非空草稿回退而不是降级为 null（笔误不等于清空意图）', () => {
    const onChange = vi.fn()
    render(
      <NumericField
        label="子网前缀"
        type="integer"
        emptyBehavior="null"
        value={24}
        min={1}
        max={32}
        onChange={onChange}
      />,
    )
    const input = getInput('子网前缀')

    fireEvent.change(input, { target: { value: '--' } })
    fireEvent.blur(input)

    expect(onChange).not.toHaveBeenCalled()
    expect(input.value).toBe('24')
  })

  it('可解析的 0 收敛到下界后派发，不被折叠为 null', () => {
    const onChange = vi.fn()
    render(
      <NumericField
        label="子网前缀"
        type="integer"
        emptyBehavior="null"
        value={24}
        min={1}
        max={32}
        onChange={onChange}
      />,
    )
    const input = getInput('子网前缀')

    fireEvent.change(input, { target: { value: '0' } })
    fireEvent.blur(input)

    expect(onChange).toHaveBeenCalledWith(1)
    expect(input.value).toBe('1')
  })

  it('无区间的可空字段原样派发，不误收敛', () => {
    const onChange = vi.fn()
    render(
      <NumericField
        label="Metric"
        type="integer"
        emptyBehavior="null"
        value={null}
        onChange={onChange}
      />,
    )
    const input = getInput('Metric')

    expect(input.value).toBe('')
    fireEvent.change(input, { target: { value: '100' } })
    fireEvent.blur(input)
    expect(onChange).toHaveBeenCalledWith(100)
    expect(input.value).toBe('100')
  })

  it('value 为 null 时不可解析草稿失焦恢复为空串', () => {
    const onChange = vi.fn()
    render(
      <NumericField
        label="Metric"
        type="integer"
        emptyBehavior="null"
        value={null}
        onChange={onChange}
      />,
    )
    const input = getInput('Metric')

    fireEvent.change(input, { target: { value: 'xyz' } })
    fireEvent.blur(input)

    expect(onChange).not.toHaveBeenCalled()
    expect(input.value).toBe('')
  })
})

describe('NumericField 外部值同步', () => {
  it('非有限外部值（NaN / Infinity）渲染为空且不进入渲染循环', () => {
    // 站点可能把 `Math.round(undefined * 100)` 这类 NaN 传进来；渲染期同步若用 `!==`
    // 比较，NaN 与自身不相等会让「外部值已变化」永远为真，触发 Too many re-renders。
    const onChange = vi.fn()
    const { rerender } = render(
      <NumericField label="水位" value={Number.NaN} onChange={onChange} />,
    )
    const input = getInput('水位')

    expect(input.value).toBe('')

    rerender(<NumericField label="水位" value={Number.POSITIVE_INFINITY} onChange={onChange} />)
    expect(getInput('水位').value).toBe('')

    rerender(<NumericField label="水位" value={0.15} onChange={onChange} />)
    expect(getInput('水位').value).toBe('0.15')
  })

  it('非编辑态外部值变化时同步显示', () => {
    const { rerender } = render(
      <NumericField label="保留天数" value={7} min={1} max={365} onChange={() => {}} />,
    )
    expect(getInput('保留天数').value).toBe('7')

    rerender(<NumericField label="保留天数" value={30} min={1} max={365} onChange={() => {}} />)
    expect(getInput('保留天数').value).toBe('30')
  })

  it('编辑中外部值变化不打断用户草稿', () => {
    const { rerender } = render(
      <NumericField label="保留天数" value={7} min={1} max={365} onChange={() => {}} />,
    )
    const input = getInput('保留天数')

    fireEvent.focus(input)
    fireEvent.change(input, { target: { value: '12' } })

    rerender(<NumericField label="保留天数" value={30} min={1} max={365} onChange={() => {}} />)
    expect(input.value).toBe('12')

    fireEvent.blur(input)
    expect(input.value).toBe('12')
  })

  it('编辑中途收敛后，外部回传同值不会把显示改回旧值', () => {
    const { rerender } = render(
      <NumericField label="保留天数" value={7} min={1} max={365} onChange={() => {}} />,
    )
    const input = getInput('保留天数')

    fireEvent.focus(input)
    fireEvent.change(input, { target: { value: '30' } })
    fireEvent.blur(input)
    expect(input.value).toBe('30')

    rerender(<NumericField label="保留天数" value={30} min={1} max={365} onChange={() => {}} />)
    expect(input.value).toBe('30')
  })
})

describe('NumericField 服务端渲染', () => {
  it('初始草稿来自格式化而不触碰 DOM，SSR 可渲染且展示与受控值一致', () => {
    const html = renderToString(
      <NumericField
        label="保留天数"
        type="integer"
        value={7}
        min={1}
        max={365}
        onChange={() => {}}
      />,
    )

    expect(html).toContain('aria-label="保留天数"')
    expect(html).toContain('value="7"')
    expect(html).toContain('type="text"')
  })

  it('可空字段的 null 在 SSR 下渲染为空值', () => {
    const html = renderToString(
      <NumericField
        label="Metric"
        type="integer"
        emptyBehavior="null"
        value={null}
        onChange={() => {}}
      />,
    )

    expect(html).toContain('value=""')
  })
})

describe('NumericField 透传与可访问名称', () => {
  it('className 与其余原生属性透传到 input，disabled 生效', () => {
    render(
      <NumericField
        label="保留天数"
        value={7}
        disabled
        placeholder="0=不限"
        className="w-full font-mono"
        onChange={() => {}}
      />,
    )
    const input = getInput('保留天数')

    expect(input.className).toBe('w-full font-mono')
    expect(input.disabled).toBe(true)
    expect(input.getAttribute('placeholder')).toBe('0=不限')
  })

  it('外部 onFocus 仍然触发，且不干扰编辑态切换', () => {
    const onFocus = vi.fn()
    render(
      <NumericField
        label="保留天数"
        value={7}
        min={1}
        max={365}
        onFocus={onFocus}
        onChange={() => {}}
      />,
    )
    const input = getInput('保留天数')

    fireEvent.focus(input)
    expect(onFocus).toHaveBeenCalledTimes(1)
  })
})
