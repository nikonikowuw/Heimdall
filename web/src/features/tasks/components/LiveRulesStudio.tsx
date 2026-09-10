import React, { useCallback, useEffect, useRef, useState } from 'react'
import {
  ArrowLeft,
  Check,
  Crop,
  Hexagon,
  Magnet,
  MousePointer2,
  Save,
  ShieldAlert,
  Slash,
  Square,
  Video,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { LivePlayer } from '@/features/live/components/LivePlayer'
import { algorithmApi, taskApi } from '@/lib/api'
import type {
  AlgoManifest,
  Camera,
  DetectionLineDirection,
  DetectionPoint,
  DetectionRuleRole,
  TaskConfigDto,
} from '@/types'
import { AlgoSandboxDrawer } from './AlgoSandboxDrawer'
import { AlgoSettingsSidebar } from './AlgoSettingsSidebar'
import { RuleInspectorSidebar } from './RuleInspectorSidebar'
import {
  DEFAULT_ALGO_PACKAGES,
  ExtendedRule,
  getDefaultRuleName,
  getInitialRuleColor,
  getLineMarkerEnd,
  getRuleTheme,
  getToolTheme,
  isPointInPolygon,
  isPointNearLine,
  ToolMode,
} from './rulesStudioTypes'

export interface LiveRulesStudioProps {
  camera: Camera
  onBack?: () => void
  onNavigateToAlgorithms?: () => void
}

export function LiveRulesStudio({
  camera,
  onBack,
  onNavigateToAlgorithms,
}: LiveRulesStudioProps): React.ReactElement {
  const { t } = useTranslation('task')
  const [taskConfig, setTaskConfig] = useState<TaskConfigDto | null>(null)
  const [rules, setRules] = useState<ExtendedRule[]>([])
  const [selectedRuleId, setSelectedRuleId] = useState<string | null>(null)
  const [tool, setTool] = useState<ToolMode>('select')
  const [currentPoints, setCurrentPoints] = useState<DetectionPoint[]>([])
  const [cursorPos, setCursorPos] = useState<DetectionPoint | null>(null)
  const [snapEnabled, setSnapEnabled] = useState(true)
  const [isArmed, setIsArmed] = useState(true)
  const [availableAlgos, setAvailableAlgos] = useState<AlgoManifest[]>([])
  const [isAlgoDrawerOpen, setIsAlgoDrawerOpen] = useState(false)
  const [selectedAlgoForConfig, setSelectedAlgoForConfig] = useState<AlgoManifest | null>(null)
  const [isSaving, setIsSaving] = useState(false)
  const [saveToast, setSaveToast] = useState<string | null>(null)

  // 算法选择与目标感知配置
  const [selectedAlgoId, setSelectedAlgoId] = useState<string>('general_detection')
  const [globalTargetClasses, setGlobalTargetClasses] = useState<string[]>(['person', 'car'])
  const [confidenceThreshold, setConfidenceThreshold] = useState<number>(0.5)
  const [motionGateEnabled, setMotionGateEnabled] = useState<boolean>(true)
  const [motionGateThreshold, setMotionGateThreshold] = useState<number>(25)

  const activeAlgo =
    availableAlgos.find((a) => a.algorithmId === selectedAlgoId) ||
    availableAlgos[0] ||
    DEFAULT_ALGO_PACKAGES[0]

  const handleAlgoChange = (newAlgoId: string) => {
    setSelectedAlgoId(newAlgoId)
    const found = availableAlgos.find((a) => a.algorithmId === newAlgoId)
    if (found && found.classes && found.classes.length > 0) {
      const valid = globalTargetClasses.filter((c) => found.classes.includes(c))
      setGlobalTargetClasses(valid.length > 0 ? valid : found.classes.slice(0, 3))
    }
  }

  // 顶点拖拽状态
  const [draggingVertex, setDraggingVertex] = useState<{
    ruleId: string
    pointIndex: number
  } | null>(null)

  // 图形整体平移状态
  const [draggingShape, setDraggingShape] = useState<{
    ruleId: string
    startCursor: DetectionPoint
    initialPoints: DetectionPoint[]
  } | null>(null)

  const stageRef = useRef<HTMLDivElement>(null)

  // 加载系统已入库/激活的算法列表
  useEffect(() => {
    let isMounted = true
    algorithmApi
      .list()
      .then((algoRes) => {
        if (!isMounted) return
        let list: AlgoManifest[] = []
        if (algoRes.items.length > 0) {
          list = algoRes.items.map((item) => {
            const actVer = item.versions.find((v) => v.isActive) || item.versions[0]
            return {
              algorithmId: item.algorithmId,
              name: item.name,
              version: item.activeVersion || (actVer ? actVer.version : '1.0.0'),
              description: item.description,
              algorithmType: item.algorithmType,
              category: item.algorithmType || 'detection',
              supportedPlatforms: actVer ? [actVer.platformId] : ['macos-arm64'],
              alarmTypeId: item.alarmTypeId,
              author: item.isBuiltin ? 'System' : 'Custom',
              classes: ['person', 'car', 'bicycle', 'motorcycle'],
            }
          })
        } else {
          list = DEFAULT_ALGO_PACKAGES
        }

        setAvailableAlgos(list)
        if (list.length > 0) {
          setSelectedAlgoId((prev) => {
            if (prev && list.some((p) => p.algorithmId === prev)) return prev
            return list[0].algorithmId
          })
        }
      })
      .catch(() => {
        if (isMounted) setAvailableAlgos(DEFAULT_ALGO_PACKAGES)
      })

    return () => {
      isMounted = false
    }
  }, [])

  // 加载选定摄像头的任务布防配置
  useEffect(() => {
    taskApi
      .getTask(camera.cameraId)
      .then((dto) => {
        setTaskConfig(dto)
        setIsArmed(dto.desiredEnabled)

        const instances = dto.algorithmInstances ?? []
        if (instances.length > 0) {
          const activeInst = instances.find((i) => i.enabled) || instances[0]
          setSelectedAlgoId(activeInst.algorithmId)
        } else if (dto.algorithmId) {
          setSelectedAlgoId(dto.algorithmId)
        }

        if (dto.motionGate) {
          setMotionGateEnabled(dto.motionGate.enabled)
          if (dto.motionGate.threshold !== undefined) {
            setMotionGateThreshold(dto.motionGate.threshold)
          }
        }

        let roiIdx = 0
        const extRules: ExtendedRule[] = dto.rules.map((r, idx) => {
          const color = getInitialRuleColor(r.role, roiIdx)
          if (r.role === 'roi') {
            roiIdx += 1
          }

          return {
            ...r,
            lineDirection:
              r.lineDirection ||
              (r as unknown as { line_direction?: DetectionLineDirection }).line_direction ||
              'both',
            id: `rule_${idx}_${Date.now()}`,
            name: getDefaultRuleName(r.role, idx + 1),
            visible: true,
            boundAlgo: selectedAlgoId,
            targetClasses: ['person', 'car'],
            color,
          }
        })
        setRules(extRules)
        if (extRules.length > 0) {
          setSelectedRuleId(extRules[0].id)
        }
      })
      .catch(() => {})
  }, [camera, selectedAlgoId])

  // 删除规则
  const deleteRule = useCallback((ruleId: string) => {
    setRules((prev) => prev.filter((r) => r.id !== ruleId))
    setSelectedRuleId((prev) => (prev === ruleId ? null : prev))
  }, [])

  // 获取归一化鼠标相对坐标 [0.0, 1.0]
  const getNormalizedPoint = useCallback(
    (
      e:
        | React.MouseEvent<HTMLElement | SVGElement>
        | MouseEvent
        | { clientX: number; clientY: number },
      skipPoint?: { ruleId: string; pointIndex: number },
    ): DetectionPoint => {
      if (!stageRef.current) return { x: 0, y: 0 }
      const rect = stageRef.current.getBoundingClientRect()
      let x = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width))
      let y = Math.max(0, Math.min(1, (e.clientY - rect.top) / rect.height))

      // 磁吸吸附到其他既有顶点
      if (snapEnabled) {
        const threshold = 0.015
        for (const rule of rules) {
          if (!rule.visible) continue
          for (let i = 0; i < rule.points.length; i++) {
            if (skipPoint && rule.id === skipPoint.ruleId && i === skipPoint.pointIndex) {
              continue
            }
            const pt = rule.points[i]
            if (Math.hypot(pt.x - x, pt.y - y) < threshold) {
              x = pt.x
              y = pt.y
              break
            }
          }
        }
      }

      return { x: Number(x.toFixed(4)), y: Number(y.toFixed(4)) }
    },
    [snapEnabled, rules],
  )

  // 完成并提交绘制点集
  const finishDrawingPoints = useCallback(
    (points: DetectionPoint[], activeTool: ToolMode = tool) => {
      if (activeTool === 'line') {
        if (points.length < 2) {
          setCurrentPoints([])
          return
        }
      } else if (points.length < 3) {
        setCurrentPoints([])
        return
      }

      let role: DetectionRuleRole = 'roi'
      if (activeTool === 'line') {
        role = 'line'
      } else if (activeTool === 'mask') {
        role = 'mask'
      }

      const existingRoiCount = rules.filter((r) => r.role === 'roi').length
      const assignedColor = getInitialRuleColor(role, existingRoiCount)

      const newRule: ExtendedRule = {
        id: `rule_${Date.now()}`,
        name: getDefaultRuleName(role, rules.length + 1),
        role,
        lineDirection: role === 'line' ? 'both' : undefined,
        points: [...points],
        visible: true,
        boundAlgo: 'general_detection',
        targetClasses: ['person', 'car'],
        color: assignedColor,
      }

      setRules((prev) => [...prev, newRule])
      setSelectedRuleId(newRule.id)
      setCurrentPoints([])
      setTool('select')
    },
    [rules, tool],
  )

  // 完成当前绘制
  const finishDrawing = useCallback(() => {
    finishDrawingPoints(currentPoints, tool)
  }, [currentPoints, finishDrawingPoints, tool])

  // 键盘快捷键响应
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return

      if (e.key === 'v' || e.key === 'V') {
        setTool('select')
        setCurrentPoints([])
      }
      if (e.key === 'r' || e.key === 'R' || e.key === 'p' || e.key === 'P') {
        setTool('roi')
        setCurrentPoints([])
      }
      if (e.key === 'l' || e.key === 'L') {
        setTool('line')
        setCurrentPoints([])
      }
      if (e.key === 'm' || e.key === 'M') {
        setTool('mask')
        setCurrentPoints([])
      }
      if (e.key === 'b' || e.key === 'B') {
        setTool('rect')
        setCurrentPoints([])
      }
      if (e.key === 'c' || e.key === 'C') {
        setTool('precrop')
        setCurrentPoints([])
      }
      if (e.key === 'Enter') {
        finishDrawing()
      }
      if (e.key === 'Escape') {
        if (isAlgoDrawerOpen) {
          setIsAlgoDrawerOpen(false)
          return
        }
        if (currentPoints.length > 0) {
          setCurrentPoints([])
        } else if (tool !== 'select') {
          setTool('select')
        } else if (onBack) {
          onBack()
        }
      }
      if (e.key === 'Backspace' || e.key === 'Delete') {
        if (currentPoints.length > 0) {
          e.preventDefault()
          setCurrentPoints((prev) => prev.slice(0, -1))
          return
        }
        if (selectedRuleId && tool === 'select') {
          deleteRule(selectedRuleId)
        }
      }
    }

    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [tool, currentPoints, selectedRuleId, onBack, isAlgoDrawerOpen, finishDrawing, deleteRule])

  // 鼠标在画布按下 (MouseDown)
  const handleStageMouseDown = (e: React.MouseEvent<HTMLDivElement>) => {
    if (e.button !== 0) return

    const pt = getNormalizedPoint(e)

    if (tool === 'select' && selectedRuleId) {
      const rule = rules.find((r) => r.id === selectedRuleId)
      if (rule && rule.visible) {
        const hit =
          rule.role === 'line'
            ? isPointNearLine(pt, rule.points[0], rule.points[1])
            : isPointInPolygon(pt, rule.points)
        if (hit) {
          setDraggingShape({
            ruleId: rule.id,
            startCursor: pt,
            initialPoints: rule.points.map((p) => ({ ...p })),
          })
        }
      }
    }
  }

  // 全局平滑拖拽监听器
  useEffect(() => {
    if (!draggingVertex && !draggingShape) return

    const handleGlobalMouseMove = (e: MouseEvent) => {
      if (draggingVertex) {
        const pt = getNormalizedPoint(e, draggingVertex)
        setRules((prev) =>
          prev.map((r) => {
            if (r.id !== draggingVertex.ruleId) return r
            const pts = [...r.points]
            pts[draggingVertex.pointIndex] = pt
            return { ...r, points: pts }
          }),
        )
      } else if (draggingShape) {
        const pt = getNormalizedPoint(e)
        const dx = pt.x - draggingShape.startCursor.x
        const dy = pt.y - draggingShape.startCursor.y
        setRules((prev) =>
          prev.map((r) => {
            if (r.id !== draggingShape.ruleId) return r
            const movedPts = draggingShape.initialPoints.map((p) => ({
              x: Math.max(0, Math.min(1, Number((p.x + dx).toFixed(4)))),
              y: Math.max(0, Math.min(1, Number((p.y + dy).toFixed(4)))),
            }))
            return { ...r, points: movedPts }
          }),
        )
      }
    }

    const handleGlobalMouseUp = () => {
      setDraggingVertex(null)
      setDraggingShape(null)
    }

    window.addEventListener('mousemove', handleGlobalMouseMove)
    window.addEventListener('mouseup', handleGlobalMouseUp)
    return () => {
      window.removeEventListener('mousemove', handleGlobalMouseMove)
      window.removeEventListener('mouseup', handleGlobalMouseUp)
    }
  }, [draggingVertex, draggingShape, getNormalizedPoint])

  const handleStageMouseMove = (e: React.MouseEvent<HTMLDivElement>) => {
    if (draggingVertex || draggingShape) return
    const pt = getNormalizedPoint(e)
    setCursorPos(pt)
  }

  const handleStageMouseUp = () => {
    if (draggingVertex) setDraggingVertex(null)
    if (draggingShape) setDraggingShape(null)
  }

  const handleStageClick = (e: React.MouseEvent<HTMLDivElement>) => {
    if (tool === 'select') return

    const pt = getNormalizedPoint(e)

    if (tool === 'rect' || tool === 'precrop') {
      if (currentPoints.length === 0) {
        setCurrentPoints([pt])
      } else {
        const p1 = currentPoints[0]
        const p2 = pt
        if (Math.hypot(p2.x - p1.x, p2.y - p1.y) < 0.005) return
        const x1 = Math.min(p1.x, p2.x)
        const y1 = Math.min(p1.y, p2.y)
        const x2 = Math.max(p1.x, p2.x)
        const y2 = Math.max(p1.y, p2.y)
        const rectPoints: DetectionPoint[] = [
          { x: x1, y: y1 },
          { x: x2, y: y1 },
          { x: x2, y: y2 },
          { x: x1, y: y2 },
        ]
        finishDrawingPoints(rectPoints, tool)
      }
      return
    }

    if (tool === 'line') {
      if (currentPoints.length === 0) {
        setCurrentPoints([pt])
      } else {
        if (Math.hypot(pt.x - currentPoints[0].x, pt.y - currentPoints[0].y) < 0.005) return
        finishDrawingPoints([currentPoints[0], pt], 'line')
      }
      return
    }

    if (tool === 'polygon' || tool === 'roi' || tool === 'mask') {
      if (currentPoints.length === 0) {
        setCurrentPoints([pt])
      } else {
        if (currentPoints.length >= 3) {
          const first = currentPoints[0]
          if (Math.hypot(first.x - pt.x, first.y - pt.y) < 0.035) {
            finishDrawing()
            return
          }
        }
        const last = currentPoints[currentPoints.length - 1]
        if (Math.hypot(last.x - pt.x, last.y - pt.y) < 0.005) return
        setCurrentPoints((prev) => [...prev, pt])
      }
    }
  }

  const handleStageDoubleClick = (e: React.MouseEvent<HTMLDivElement>) => {
    e.preventDefault()
    e.stopPropagation()
    if (currentPoints.length >= 3 && (tool === 'polygon' || tool === 'mask' || tool === 'roi')) {
      finishDrawing()
    }
  }

  const handleContextMenu = (e: React.MouseEvent<HTMLDivElement>) => {
    e.preventDefault()
    if (currentPoints.length > 0) {
      setCurrentPoints([])
    } else if (tool !== 'select') {
      setTool('select')
    }
  }

  const handleSave = async () => {
    setIsSaving(true)

    try {
      const currentInstances = taskConfig?.algorithmInstances ?? []
      const selectedInstance = currentInstances.find(
        (instance) => instance.algorithmId === selectedAlgoId,
      )
      const nextInstance = {
        ...selectedInstance,
        algorithmId: selectedAlgoId,
        analysisFps: selectedInstance?.analysisFps ?? 10,
        algoParams: { confidenceThreshold, targetClasses: globalTargetClasses },
        enabled: isArmed,
      }

      const payloadDto: TaskConfigDto = {
        cameraId: camera.cameraId,
        name: taskConfig?.name || camera.name || `Task-${camera.cameraId}`,
        desiredEnabled: isArmed,
        rules: rules.map((r) => ({
          role: r.role,
          lineDirection: r.lineDirection,
          points: r.points,
        })),
        motionGate: {
          enabled: motionGateEnabled,
          threshold: motionGateThreshold,
          contourArea: 100,
          keepaliveIntervalMs: 2000,
        },
        algorithmInstances: selectedAlgoId
          ? [
              ...currentInstances.filter((instance) => instance.algorithmId !== selectedAlgoId),
              nextInstance,
            ]
          : currentInstances,
      }

      const updatedTask = await taskApi.updateTask(camera.cameraId, payloadDto)
      setTaskConfig(updatedTask)

      setSaveToast(t('footer.saveSuccess'))
      setTimeout(() => setSaveToast(null), 3000)
    } catch (err) {
      setSaveToast(`保存失败: ${err instanceof Error ? err.message : String(err)}`)
      setTimeout(() => setSaveToast(null), 4000)
    } finally {
      setIsSaving(false)
    }
  }

  const cloneRule = (ruleId: string) => {
    const target = rules.find((r) => r.id === ruleId)
    if (!target) return
    const cloned: ExtendedRule = {
      ...target,
      id: `rule_${Date.now()}`,
      name: `${target.name} (副本)`,
      points: target.points.map((p) => ({
        x: Math.min(1, p.x + 0.03),
        y: Math.min(1, p.y + 0.03),
      })),
    }
    setRules((prev) => [...prev, cloned])
    setSelectedRuleId(cloned.id)
  }

  const handleUpdateRule = (ruleId: string, partial: Partial<ExtendedRule>) => {
    setRules((prev) => prev.map((r) => (r.id === ruleId ? { ...r, ...partial } : r)))
  }

  const selectedRule = rules.find((r) => r.id === selectedRuleId)
  const isLineOrPolyReadyToClose =
    (tool === 'line' && currentPoints.length >= 2) || currentPoints.length >= 3

  return (
    <div className="flex h-full w-full flex-col overflow-hidden bg-[var(--bg-primary)] font-sans text-[var(--text-primary)] select-none">
      {/* 顶部 Header */}
      <header className="frosted-glass z-30 flex h-14 shrink-0 items-center justify-between border-b border-[var(--border)] px-4 shadow-xs">
        <div className="flex items-center gap-3">
          {onBack && (
            <button
              type="button"
              onClick={onBack}
              className="flex items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-2.5 py-1.5 text-xs font-medium text-[var(--text-secondary)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
              title={`${t('studio.backToChannels', { defaultValue: '返回通道列表' })} (Esc)`}
            >
              <ArrowLeft className="h-4 w-4" />
              <span className="hidden sm:inline">
                {t('studio.backToChannels', { defaultValue: '返回' })}
              </span>
            </button>
          )}

          <div className="h-5 w-[1px] bg-[var(--border)]" />

          <div className="flex items-center gap-2">
            <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-[var(--accent-soft)] text-[var(--accent)]">
              <Video className="h-4 w-4" />
            </div>
            <div className="flex items-center gap-1.5">
              <span
                className="max-w-[180px] truncate text-sm font-bold text-[var(--text-primary)]"
                title={camera.name || camera.cameraId}
              >
                {camera.name || camera.cameraId}
              </span>
              <span className="font-mono text-xs text-[var(--text-muted)]">
                ({camera.cameraId})
              </span>
            </div>
          </div>

          <div className="hidden items-center gap-1.5 font-mono text-xs md:flex">
            <span className="rounded bg-[var(--bg-secondary)] px-1.5 py-0.5 font-semibold text-[var(--accent)]">
              {camera.lastCodec?.toUpperCase() || 'H.264'}
            </span>
            <span className="text-[var(--text-muted)]">
              {camera.lastWidth && camera.lastHeight
                ? `${camera.lastWidth}x${camera.lastHeight}`
                : '1080P'}
            </span>
            <span className="text-emerald-500">
              {camera.lastFps ? camera.lastFps.toFixed(1) : '25.0'} fps
            </span>
          </div>

          <div className="hidden h-5 w-[1px] bg-[var(--border)] sm:block" />

          {/* 布防总闸 Toggle */}
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => setIsArmed(!isArmed)}
              className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
                isArmed ? 'bg-rose-500' : 'bg-slate-400'
              }`}
              title={isArmed ? t('status.armed') : t('status.disarmed')}
            >
              <span
                className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow-lg ring-0 transition duration-200 ease-in-out ${
                  isArmed ? 'translate-x-4' : 'translate-x-0'
                }`}
              />
            </button>
            <span
              className={`font-mono text-xs font-semibold ${
                isArmed ? 'text-rose-500' : 'text-[var(--text-muted)]'
              }`}
            >
              {isArmed ? t('status.armed') : t('status.disarmed')}
            </span>
          </div>
        </div>

        {/* 右侧：规则计数 + 保存主操作 */}
        <div className="flex items-center gap-3">
          {saveToast && (
            <span className="animate-fade-in font-mono text-xs font-semibold text-emerald-500">
              {saveToast}
            </span>
          )}

          <div className="hidden items-center gap-1.5 text-xs text-[var(--text-muted)] sm:flex">
            <span>{t('card.geometryRules')}:</span>
            <span className="font-mono font-bold text-[var(--accent)]">{rules.length}</span>
          </div>

          <button
            type="button"
            onClick={handleSave}
            disabled={isSaving}
            className="flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 active:scale-95 disabled:opacity-50"
          >
            <Save className="h-4 w-4" />
            <span>{isSaving ? t('footer.saving') : t('footer.saveTask')}</span>
          </button>
        </div>
      </header>

      {/* 主标定工作区：三段式布局 */}
      <div className="relative flex flex-1 overflow-hidden">
        {/* 左侧：算法模型与目标感知 */}
        <AlgoSettingsSidebar
          availableAlgos={availableAlgos}
          selectedAlgoId={selectedAlgoId}
          activeAlgo={activeAlgo}
          onAlgoChange={handleAlgoChange}
          onOpenAlgoDrawer={() => {
            setSelectedAlgoForConfig(activeAlgo)
            setIsAlgoDrawerOpen(true)
          }}
          globalTargetClasses={globalTargetClasses}
          onToggleTargetClass={(cls) =>
            setGlobalTargetClasses((prev) =>
              prev.includes(cls) ? prev.filter((c) => c !== cls) : [...prev, cls],
            )
          }
          onSelectAllClasses={() => setGlobalTargetClasses([...activeAlgo.classes])}
          onClearAllClasses={() => setGlobalTargetClasses([])}
          confidenceThreshold={confidenceThreshold}
          onConfidenceThresholdChange={setConfidenceThreshold}
          motionGateEnabled={motionGateEnabled}
          onMotionGateEnabledChange={setMotionGateEnabled}
        />

        {/* 中央：实时流互动舞台 */}
        <main className="relative flex flex-1 flex-col overflow-hidden bg-[var(--bg-primary)]">
          {/* 顶部悬浮绘制工具栏 */}
          <div className="frosted-glass absolute top-4 left-1/2 z-20 flex -translate-x-1/2 items-center gap-1 rounded-full border border-[var(--border)] p-1.5 whitespace-nowrap shadow-xl">
            <button
              type="button"
              onClick={() => {
                setTool('select')
                setCurrentPoints([])
              }}
              className={`flex shrink-0 items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-semibold whitespace-nowrap transition-all ${
                tool === 'select'
                  ? 'bg-[var(--accent)] text-white shadow-xs'
                  : 'text-[var(--text-secondary)] hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]'
              }`}
              title={`${t('tools.select')} (V)`}
            >
              <MousePointer2 className="h-4 w-4 shrink-0" />
              <span>{t('tools.select')}</span>
              <kbd className="font-mono text-[10px] opacity-70">V</kbd>
            </button>

            <span className="mx-0.5 h-4 w-[1px] shrink-0 bg-[var(--border)]" />

            {/* 多边形防区 */}
            <button
              type="button"
              onClick={() => {
                setTool('roi')
                setCurrentPoints([])
              }}
              className={`flex shrink-0 items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-semibold whitespace-nowrap transition-all ${
                tool === 'roi' || tool === 'polygon'
                  ? 'bg-cyan-500 text-white shadow-xs'
                  : 'text-[var(--text-secondary)] hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]'
              }`}
              title={`${t('tools.roi')} (R)`}
            >
              <Hexagon className="h-4 w-4 shrink-0" />
              <span>{t('tools.roi')}</span>
              <kbd className="font-mono text-[10px] opacity-70">R</kbd>
            </button>

            {/* 越界绊线 */}
            <button
              type="button"
              onClick={() => {
                setTool('line')
                setCurrentPoints([])
              }}
              className={`flex shrink-0 items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-semibold whitespace-nowrap transition-all ${
                tool === 'line'
                  ? 'bg-emerald-500 text-white shadow-xs'
                  : 'text-[var(--text-secondary)] hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]'
              }`}
              title={`${t('tools.line')} (L)`}
            >
              <Slash className="h-4 w-4 shrink-0" />
              <span>{t('tools.line')}</span>
              <kbd className="font-mono text-[10px] opacity-70">L</kbd>
            </button>

            {/* 屏蔽遮罩 */}
            <button
              type="button"
              onClick={() => {
                setTool('mask')
                setCurrentPoints([])
              }}
              className={`flex shrink-0 items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-semibold whitespace-nowrap transition-all ${
                tool === 'mask'
                  ? 'bg-slate-700 text-white shadow-xs'
                  : 'text-[var(--text-secondary)] hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]'
              }`}
              title={`${t('tools.mask')} (M)`}
            >
              <ShieldAlert className="h-4 w-4 shrink-0" />
              <span>{t('tools.mask')}</span>
              <kbd className="font-mono text-[10px] opacity-70">M</kbd>
            </button>

            {/* 矩形框 */}
            <button
              type="button"
              onClick={() => {
                setTool('rect')
                setCurrentPoints([])
              }}
              className={`flex shrink-0 items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-semibold whitespace-nowrap transition-all ${
                tool === 'rect'
                  ? 'bg-sky-600 text-white shadow-xs'
                  : 'text-[var(--text-secondary)] hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]'
              }`}
              title={`${t('tools.rect')} (B)`}
            >
              <Square className="h-4 w-4 shrink-0" />
              <span>{t('tools.rect')}</span>
              <kbd className="font-mono text-[10px] opacity-70">B</kbd>
            </button>

            {/* 局部特写 */}
            <button
              type="button"
              onClick={() => {
                setTool('precrop')
                setCurrentPoints([])
              }}
              className={`flex shrink-0 items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-semibold whitespace-nowrap transition-all ${
                tool === 'precrop'
                  ? 'bg-amber-500 text-white shadow-xs'
                  : 'text-[var(--text-secondary)] hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]'
              }`}
              title={`${t('tools.precrop')} (C)`}
            >
              <Crop className="h-4 w-4 shrink-0" />
              <span>{t('tools.precrop')}</span>
              <kbd className="font-mono text-[10px] opacity-70">C</kbd>
            </button>

            <span className="mx-0.5 h-4 w-[1px] shrink-0 bg-[var(--border)]" />

            <button
              type="button"
              onClick={() => setSnapEnabled(!snapEnabled)}
              className={`flex h-7 w-7 shrink-0 items-center justify-center rounded-full transition-colors ${
                snapEnabled
                  ? 'bg-[var(--accent-soft)] text-[var(--accent)]'
                  : 'text-[var(--text-muted)] hover:text-[var(--text-primary)]'
              }`}
              title={
                snapEnabled
                  ? t('studio.snapEnabled', { defaultValue: '顶点自动磁吸已开启 (S)' })
                  : t('studio.snapDisabled', { defaultValue: '顶点自动磁吸已关闭 (S)' })
              }
            >
              <Magnet className="h-3.5 w-3.5" />
            </button>

            {isLineOrPolyReadyToClose && (
              <button
                type="button"
                onClick={finishDrawing}
                className="ml-1 flex shrink-0 animate-pulse items-center gap-1 rounded-full bg-emerald-500 px-3.5 py-1.5 text-xs font-semibold whitespace-nowrap text-white shadow-xs transition-all hover:bg-emerald-600"
              >
                <Check className="h-4 w-4" />
                <span>{t('tools.closePolygon')}</span>
              </button>
            )}
          </div>

          {/* 交互视口区 */}
          <div className="flex flex-1 items-center justify-center overflow-hidden p-4">
            <div
              ref={stageRef}
              onMouseDown={handleStageMouseDown}
              onMouseMove={handleStageMouseMove}
              onMouseUp={handleStageMouseUp}
              onClick={handleStageClick}
              onDoubleClick={handleStageDoubleClick}
              onContextMenu={handleContextMenu}
              onMouseLeave={() => setCursorPos(null)}
              className={`relative max-h-full max-w-full overflow-hidden rounded-2xl border border-[var(--border)] bg-black/95 shadow-2xl select-none ${
                tool !== 'select' ? 'cursor-crosshair' : 'cursor-default'
              }`}
              style={{
                width: '100%',
                height: 'auto',
                maxWidth: '1280px',
                aspectRatio:
                  camera.lastWidth && camera.lastHeight
                    ? `${camera.lastWidth} / ${camera.lastHeight}`
                    : '16 / 9',
              }}
            >
              {/* 真实子码流播放器 */}
              <LivePlayer
                cameraId={camera.cameraId}
                cameraName={camera.name}
                stream="sub"
                className="pointer-events-none h-full w-full"
              />

              {/* 矢量绘制与标定交互 SVG 覆盖层 */}
              <svg
                className="pointer-events-auto absolute inset-0 h-full w-full"
                viewBox="0 0 100 100"
                preserveAspectRatio="none"
              >
                <defs>
                  <marker
                    id="line-arrow-a-to-b"
                    markerWidth="8"
                    markerHeight="8"
                    refX="4"
                    refY="4"
                    orient="auto"
                  >
                    <path d="M 1 1 L 7 4 L 1 7 Z" fill="#10b981" />
                  </marker>
                  <marker
                    id="line-arrow-b-to-a"
                    markerWidth="8"
                    markerHeight="8"
                    refX="4"
                    refY="4"
                    orient="auto-start-reverse"
                  >
                    <path d="M 1 1 L 7 4 L 1 7 Z" fill="#10b981" />
                  </marker>
                </defs>

                {/* 既有布防规则渲染 */}
                {rules.map((rule, ruleIdx) => {
                  if (!rule.visible || rule.points.length < 2) return null
                  const isSelected = rule.id === selectedRuleId
                  const theme = getRuleTheme(rule, ruleIdx)

                  if (rule.role === 'line') {
                    const p1 = rule.points[0]
                    const p2 = rule.points[1]
                    return (
                      <g
                        key={rule.id}
                        onClick={(e) => {
                          e.stopPropagation()
                          setSelectedRuleId(rule.id)
                        }}
                      >
                        <line
                          x1={`${p1.x * 100}%`}
                          y1={`${p1.y * 100}%`}
                          x2={`${p2.x * 100}%`}
                          y2={`${p2.y * 100}%`}
                          stroke={isSelected ? theme.selectedStroke : theme.stroke}
                          strokeWidth={isSelected ? '2.5' : '1.5'}
                          vectorEffect="non-scaling-stroke"
                          markerEnd={getLineMarkerEnd(rule.lineDirection)}
                          className={tool === 'select' ? 'cursor-move' : 'cursor-pointer'}
                          onMouseDown={(e) => {
                            if (tool === 'select') {
                              e.stopPropagation()
                              setSelectedRuleId(rule.id)
                              const pt = getNormalizedPoint(e)
                              setDraggingShape({
                                ruleId: rule.id,
                                startCursor: pt,
                                initialPoints: rule.points.map((p) => ({ ...p })),
                              })
                            }
                          }}
                        />
                      </g>
                    )
                  }

                  const ptsStr = rule.points.map((p) => `${p.x * 100},${p.y * 100}`).join(' ')

                  return (
                    <g
                      key={rule.id}
                      onClick={(e) => {
                        e.stopPropagation()
                        setSelectedRuleId(rule.id)
                      }}
                    >
                      <polygon
                        points={ptsStr}
                        fill={theme.fill}
                        stroke={isSelected ? theme.selectedStroke : theme.stroke}
                        strokeWidth={isSelected ? '2.5' : '1.5'}
                        vectorEffect="non-scaling-stroke"
                        className={tool === 'select' ? 'cursor-move' : 'cursor-pointer'}
                        onMouseDown={(e) => {
                          if (tool === 'select') {
                            e.stopPropagation()
                            setSelectedRuleId(rule.id)
                            const pt = getNormalizedPoint(e)
                            setDraggingShape({
                              ruleId: rule.id,
                              startCursor: pt,
                              initialPoints: rule.points.map((p) => ({ ...p })),
                            })
                          }
                        }}
                      />
                    </g>
                  )
                })}

                {/* 绘制中的交互图形与橡皮筋引导线 */}
                {currentPoints.length > 0 &&
                  (() => {
                    const activeToolTheme = getToolTheme(
                      tool,
                      rules.filter((r) => r.role === 'roi').length,
                    )
                    return (
                      <g className="pointer-events-none">
                        {(tool === 'rect' || tool === 'precrop') && cursorPos && (
                          <>
                            <rect
                              x={Math.min(currentPoints[0].x, cursorPos.x) * 100}
                              y={Math.min(currentPoints[0].y, cursorPos.y) * 100}
                              width={Math.abs(cursorPos.x - currentPoints[0].x) * 100}
                              height={Math.abs(cursorPos.y - currentPoints[0].y) * 100}
                              fill={activeToolTheme.fill}
                              stroke={activeToolTheme.stroke}
                              strokeWidth={1.5}
                              strokeDasharray="4 3"
                              vectorEffect="non-scaling-stroke"
                            />
                            <line
                              x1={currentPoints[0].x * 100}
                              y1={currentPoints[0].y * 100}
                              x2={cursorPos.x * 100}
                              y2={cursorPos.y * 100}
                              stroke={activeToolTheme.stroke}
                              strokeWidth={1}
                              strokeDasharray="2 2"
                              opacity={0.5}
                              vectorEffect="non-scaling-stroke"
                            />
                          </>
                        )}

                        {(tool === 'polygon' || tool === 'roi' || tool === 'mask') &&
                          currentPoints.length >= 2 &&
                          cursorPos && (
                            <polygon
                              points={[
                                ...currentPoints.map((p) => `${p.x * 100},${p.y * 100}`),
                                `${cursorPos.x * 100},${cursorPos.y * 100}`,
                              ].join(' ')}
                              fill={activeToolTheme.fill}
                              stroke="none"
                            />
                          )}

                        {tool !== 'rect' && tool !== 'precrop' && currentPoints.length >= 2 && (
                          <polyline
                            points={currentPoints.map((p) => `${p.x * 100},${p.y * 100}`).join(' ')}
                            fill="none"
                            stroke={activeToolTheme.stroke}
                            strokeWidth={1.5}
                            vectorEffect="non-scaling-stroke"
                          />
                        )}

                        {tool !== 'rect' && tool !== 'precrop' && cursorPos && (
                          <line
                            x1={currentPoints[currentPoints.length - 1].x * 100}
                            y1={currentPoints[currentPoints.length - 1].y * 100}
                            x2={cursorPos.x * 100}
                            y2={cursorPos.y * 100}
                            stroke={activeToolTheme.stroke}
                            strokeWidth={1.5}
                            strokeDasharray="4 3"
                            vectorEffect="non-scaling-stroke"
                          />
                        )}

                        {(tool === 'polygon' || tool === 'roi' || tool === 'mask') &&
                          currentPoints.length >= 2 &&
                          cursorPos && (
                            <line
                              x1={cursorPos.x * 100}
                              y1={cursorPos.y * 100}
                              x2={currentPoints[0].x * 100}
                              y2={currentPoints[0].y * 100}
                              stroke={activeToolTheme.stroke}
                              strokeWidth={1}
                              strokeDasharray="3 3"
                              opacity={0.4}
                              vectorEffect="non-scaling-stroke"
                            />
                          )}
                      </g>
                    )
                  })()}
              </svg>

              {/* 精致 HTML 矢量控制点与交互层 */}
              <div className="pointer-events-none absolute inset-0 h-full w-full overflow-hidden">
                {tool === 'select' &&
                  rules.map((rule, ruleIdx) => {
                    if (rule.id !== selectedRuleId || !rule.visible) return null
                    const theme = getRuleTheme(rule, ruleIdx)

                    return rule.points.map((pt, idx) => {
                      const isBeingDragged =
                        draggingVertex?.ruleId === rule.id && draggingVertex?.pointIndex === idx

                      return (
                        <div
                          key={`handle-${rule.id}-${idx}`}
                          style={{ left: `${pt.x * 100}%`, top: `${pt.y * 100}%` }}
                          onMouseDown={(e) => {
                            e.stopPropagation()
                            setDraggingVertex({ ruleId: rule.id, pointIndex: idx })
                          }}
                          className={`pointer-events-auto absolute -translate-x-1/2 -translate-y-1/2 select-none ${
                            isBeingDragged ? 'z-30 cursor-grabbing' : 'z-20 cursor-grab'
                          }`}
                        >
                          <div
                            className={`rounded-full border-2 shadow-md ${theme.handleBg} ${
                              isBeingDragged
                                ? 'h-3 w-3 ring-4'
                                : 'h-2.5 w-2.5 hover:h-3 hover:w-3 hover:ring-2'
                            }`}
                          />
                        </div>
                      )
                    })
                  })}

                {currentPoints.map((pt, idx) => {
                  const activeToolTheme = getToolTheme(
                    tool,
                    rules.filter((r) => r.role === 'roi').length,
                  )
                  const isFirst = idx === 0
                  const canCloseOnFirst = isFirst && currentPoints.length >= 3
                  const isHoveredFirst =
                    canCloseOnFirst &&
                    cursorPos &&
                    Math.hypot(pt.x - cursorPos.x, pt.y - cursorPos.y) < 0.035

                  return (
                    <div
                      key={`draw-pt-${idx}`}
                      style={{ left: `${pt.x * 100}%`, top: `${pt.y * 100}%` }}
                      onClick={
                        canCloseOnFirst
                          ? (e) => {
                              e.stopPropagation()
                              finishDrawing()
                            }
                          : undefined
                      }
                      className={`absolute flex -translate-x-1/2 -translate-y-1/2 items-center justify-center select-none ${
                        canCloseOnFirst
                          ? 'pointer-events-auto z-30 cursor-pointer'
                          : 'pointer-events-none z-20'
                      }`}
                    >
                      {isHoveredFirst && (
                        <div
                          className="absolute h-5 w-5 rounded-full border ring-2"
                          style={{
                            borderColor: activeToolTheme.stroke,
                            backgroundColor: activeToolTheme.fill,
                          }}
                        />
                      )}

                      <div
                        className="h-2 w-2 rounded-full border border-black/70 shadow-xs"
                        style={{
                          backgroundColor: activeToolTheme.stroke,
                        }}
                      />

                      <span className="py-0.2 pointer-events-none absolute -top-4 left-1/2 -translate-x-1/2 rounded bg-black/80 px-1 font-mono text-[10px] font-bold text-white shadow-xs backdrop-blur-xs">
                        {idx + 1}
                      </span>

                      {isHoveredFirst && (
                        <span
                          className="pointer-events-none absolute -bottom-6 left-1/2 -translate-x-1/2 rounded px-2 py-0.5 text-xs font-semibold whitespace-nowrap text-white shadow-md"
                          style={{ backgroundColor: activeToolTheme.stroke }}
                        >
                          点击闭合
                        </span>
                      )}
                    </div>
                  )
                })}

                {tool !== 'select' &&
                  cursorPos &&
                  (() => {
                    const activeToolTheme = getToolTheme(
                      tool,
                      rules.filter((r) => r.role === 'roi').length,
                    )
                    return (
                      <div
                        style={{
                          left: `${cursorPos.x * 100}%`,
                          top: `${cursorPos.y * 100}%`,
                          backgroundColor: activeToolTheme.stroke,
                        }}
                        className="pointer-events-none absolute h-1.5 w-1.5 -translate-x-1/2 -translate-y-1/2 rounded-full border border-black/70 shadow-xs"
                      />
                    )
                  })()}
              </div>
            </div>
          </div>

          {/* 舞台底部交互指引 HUD */}
          <div className="frosted-glass pointer-events-none absolute bottom-4 left-1/2 z-20 flex -translate-x-1/2 items-center gap-2 rounded-full border border-[var(--border)] bg-[var(--bg-surface)]/85 px-4 py-1.5 text-xs font-medium whitespace-nowrap text-[var(--text-secondary)] shadow-xl backdrop-blur-md">
            {(tool === 'roi' || tool === 'polygon') && <span>{t('tools.hudRoi')}</span>}
            {tool === 'line' && <span>{t('tools.hudLine')}</span>}
            {tool === 'mask' && <span>{t('tools.hudMask')}</span>}
            {(tool === 'rect' || tool === 'precrop') && <span>{t('tools.hudRect')}</span>}
            {tool === 'select' && <span>{t('tools.hudSelect')}</span>}
          </div>
        </main>

        {/* 右侧：图层列表与属性检查器 */}
        <RuleInspectorSidebar
          rules={rules}
          selectedRuleId={selectedRuleId}
          selectedRule={selectedRule}
          activeAlgo={activeAlgo}
          globalTargetClasses={globalTargetClasses}
          onSelectRule={setSelectedRuleId}
          onToggleRuleVisibility={(ruleId) =>
            setRules((prev) =>
              prev.map((r) => (r.id === ruleId ? { ...r, visible: !r.visible } : r)),
            )
          }
          onDeleteRule={deleteRule}
          onCloneRule={cloneRule}
          onUpdateRule={handleUpdateRule}
        />
      </div>

      {/* 算法模型与运行元信息抽屉 */}
      <AlgoSandboxDrawer
        isOpen={isAlgoDrawerOpen}
        algo={selectedAlgoForConfig}
        onClose={() => setIsAlgoDrawerOpen(false)}
        onNavigateToAlgorithms={onNavigateToAlgorithms}
      />
    </div>
  )
}
