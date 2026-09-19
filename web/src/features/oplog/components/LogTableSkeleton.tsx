import type { ReactElement } from 'react'

interface LogTableSkeletonProps {
  columnsCount?: number
  rowsCount?: number
  compact?: boolean
}

export function LogTableSkeleton({
  columnsCount = 8,
  rowsCount = 7,
  compact = false,
}: LogTableSkeletonProps): ReactElement {
  const rowHeightClass = compact ? 'py-2' : 'py-3.5'

  return (
    <>
      {Array.from({ length: rowsCount }).map((_, rowIndex) => (
        <tr
          key={`skeleton-row-${rowIndex}`}
          className="border-b border-[var(--border)] transition-colors"
        >
          {Array.from({ length: columnsCount }).map((__, colIndex) => {
            // 生成不同宽度的骨架条块，营造自然真实感
            const widthPatterns = ['w-16', 'w-24', 'w-12', 'w-20', 'w-32', 'w-14', 'w-10', 'w-28']
            const widthClass = widthPatterns[(rowIndex + colIndex) % widthPatterns.length]

            return (
              <td key={`skeleton-cell-${colIndex}`} className={`px-4 ${rowHeightClass}`}>
                <div
                  className={`h-3.5 ${widthClass} animate-pulse rounded bg-[var(--border-strong)]/40`}
                />
              </td>
            )
          })}
        </tr>
      ))}
    </>
  )
}
