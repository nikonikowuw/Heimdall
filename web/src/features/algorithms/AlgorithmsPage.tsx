import React, { useCallback, useEffect, useMemo, useState } from 'react'
import { motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { algorithmApi } from '@/lib/api'
import { motionTokens } from '@/lib/motionTokens'
import type { AlgorithmItem, AlgorithmStats } from '@/types'
import { AlgoCardGrid } from './components/AlgoCardGrid'
import { AlgoFilterBar } from './components/AlgoFilterBar'
import { AlgoStatsHeader } from './components/AlgoStatsHeader'
import { SchemaModal } from './components/SchemaModal'
import { UploadModal } from './components/UploadModal'
import { VersionsDrawer } from './components/VersionsDrawer'

export const AlgorithmsPage: React.FC = () => {
  const { t } = useTranslation('algo')

  const [stats, setStats] = useState<AlgorithmStats | null>(null)
  const [algorithms, setAlgorithms] = useState<AlgorithmItem[]>([])
  const [isLoading, setIsLoading] = useState(false)

  // 筛选过滤状态
  const [searchQuery, setSearchQuery] = useState('')
  const [typeFilter, setTypeFilter] = useState('all')
  const [originFilter, setOriginFilter] = useState<'all' | 'builtin' | 'custom'>('all')

  // 模态框与抽屉状态
  const [selectedAlgoIdForVersions, setSelectedAlgoIdForVersions] = useState<string | null>(null)
  const [selectedAlgoIdForSchema, setSelectedAlgoIdForSchema] = useState<string | null>(null)
  const [isUploadModalOpen, setIsUploadModalOpen] = useState(false)

  const selectedAlgoForVersions = useMemo(() => {
    if (!selectedAlgoIdForVersions) return null
    return algorithms.find((a) => a.algorithmId === selectedAlgoIdForVersions) ?? null
  }, [algorithms, selectedAlgoIdForVersions])

  const selectedAlgoForSchema = useMemo(() => {
    if (!selectedAlgoIdForSchema) return null
    return algorithms.find((a) => a.algorithmId === selectedAlgoIdForSchema) ?? null
  }, [algorithms, selectedAlgoIdForSchema])

  const loadData = useCallback(async () => {
    setIsLoading(true)
    try {
      const [statsData, listData] = await Promise.all([
        algorithmApi.getStats(),
        algorithmApi.list({
          page: 1,
          pageSize: 100,
          keyword: searchQuery.trim() || undefined,
          algorithmType: typeFilter !== 'all' ? typeFilter : undefined,
          isBuiltin:
            originFilter === 'builtin' ? true : originFilter === 'custom' ? false : undefined,
        }),
      ])
      setStats(statsData)
      setAlgorithms(listData.items)
    } catch {
      // 容错降级
    } finally {
      setIsLoading(false)
    }
  }, [searchQuery, typeFilter, originFilter])

  useEffect(() => {
    loadData()
  }, [loadData])

  const filteredAlgorithms = useMemo(() => {
    return algorithms.filter((algo) => {
      if (searchQuery.trim()) {
        const q = searchQuery.toLowerCase()
        const matchId = algo.algorithmId.toLowerCase().includes(q)
        const matchName = algo.name.toLowerCase().includes(q)
        if (!matchId && !matchName) return false
      }
      if (typeFilter !== 'all' && algo.algorithmType !== typeFilter) {
        return false
      }
      if (originFilter === 'builtin' && !algo.isBuiltin) {
        return false
      }
      if (originFilter === 'custom' && algo.isBuiltin) {
        return false
      }
      return true
    })
  }, [algorithms, searchQuery, typeFilter, originFilter])

  return (
    <div className="flex h-full w-full flex-col space-y-6 overflow-y-auto bg-[var(--bg-primary)] p-6">
      {/* 顶部标题区 */}
      <div className="flex flex-col gap-1">
        <h1 className="text-xl font-bold tracking-tight text-[var(--text-primary)]">
          {t('title')}
        </h1>
        <p className="text-xs text-[var(--text-muted)]">{t('subtitle')}</p>
      </div>

      {/* 统计指标卡 */}
      <AlgoStatsHeader stats={stats} isLoading={isLoading} />

      {/* 搜索与过滤工具栏 */}
      <AlgoFilterBar
        searchQuery={searchQuery}
        onSearchChange={setSearchQuery}
        typeFilter={typeFilter}
        onTypeChange={setTypeFilter}
        originFilter={originFilter}
        onOriginChange={setOriginFilter}
        onRefresh={loadData}
        onOpenUpload={() => setIsUploadModalOpen(true)}
        isLoading={isLoading}
      />

      {/* 算法卡片矩阵网格 */}
      <motion.div
        layout
        transition={{ duration: motionTokens.duration.normal, ease: motionTokens.easing.smooth }}
      >
        <AlgoCardGrid
          algorithms={filteredAlgorithms}
          isLoading={isLoading}
          onManageVersions={(algo) => setSelectedAlgoIdForVersions(algo.algorithmId)}
          onViewSchema={(algo) => setSelectedAlgoIdForSchema(algo.algorithmId)}
        />
      </motion.div>

      {/* 版本管理抽屉 */}
      <VersionsDrawer
        isOpen={Boolean(selectedAlgoForVersions)}
        algorithm={selectedAlgoForVersions}
        onClose={() => setSelectedAlgoIdForVersions(null)}
        onRefresh={loadData}
      />

      {/* 归档包拖拽上传弹窗 */}
      <UploadModal
        isOpen={isUploadModalOpen}
        onClose={() => setIsUploadModalOpen(false)}
        onSuccess={loadData}
      />

      {/* 参数配置 Schema 模态框 */}
      <SchemaModal
        isOpen={Boolean(selectedAlgoForSchema)}
        algorithm={selectedAlgoForSchema}
        onClose={() => setSelectedAlgoIdForSchema(null)}
      />
    </div>
  )
}
