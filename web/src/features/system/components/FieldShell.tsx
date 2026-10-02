import React from 'react'

export interface FieldShellProps {
  label: string
  children: React.ReactNode
}

/** 表单字段排版外壳：负责统一标签与间距，输入控件由 children 承担 */
export function FieldShell({ label, children }: FieldShellProps): React.ReactElement {
  return (
    <div>
      <label className="mb-1.5 block text-[12px] font-medium text-[var(--text-muted)]">
        {label}
      </label>
      {children}
    </div>
  )
}
