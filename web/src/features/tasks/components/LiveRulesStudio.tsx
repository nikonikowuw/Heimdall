import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  Activity,
  ArrowLeft,
  Camera as CameraIcon,
  Check,
  Columns,
  Hexagon,
  Layers,
  Magnet,
  Maximize2,
  MousePointer2,
  PanelRightClose,
  PanelRightOpen,
  Pencil,
  Ratio,
  Save,
  ShieldAlert,
  Slash,
  Trash2,
  X,
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
  StreamMode,
  TaskAlgorithmInstanceDto,
  TaskConfigDto,
} from '@/types'
import { ActivityZonesSection } from './ActivityZonesSection'
import { AlgoParamDrawer } from './AlgoParamDrawer'
import { AlgoSandboxDrawer } from './AlgoSandboxDrawer'
import { AlgorithmInstanceItem, AlgorithmRack } from './AlgorithmRack'
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

  const [taskName, setTaskName] = useState<string>('')
  const [streamMode, setStreamMode] = useState<StreamMode>(camera.streamMode || 'auto')
  const [isEditingTaskName, setIsEditingTaskName] = useState<boolean>(false)
  const [isArmed, setIsArmed] = useState<boolean>(true)

  // 计算当前画布视频流实际应拉取的码流类型：
  // 1. 若配置为 'main'，使用主码流预览
  // 2. 若配置为 'auto' 且摄像头未配置子码流 (subRtspUrl 为空)，自适应回退到主码流预览
  // 3. 否则请求子码流预览
  const effectivePreviewStream: 'main' | 'sub' =
    streamMode === 'main' || (streamMode === 'auto' && !camera.subRtspUrl?.trim()) ? 'main' : 'sub'

  // 算法池与多实例状态
  const [availableAlgos, setAvailableAlgos] = useState<AlgoManifest[]>([])
  const [activeInstances, setActiveInstances] = useState<Record<string, AlgorithmInstanceItem>>({})
  const [paramDrawerAlgo, setParamDrawerAlgo] = useState<AlgoManifest | null>(null)
  const [isAlgoSandboxOpen, setIsAlgoSandboxOpen] = useState(false)
  const [sandboxAlgo] = useState<AlgoManifest | null>(null)

  // 空间防区与标定模式
  const [rules, setRules] = useState<ExtendedRule[]>([])
  const [selectedRuleId, setSelectedRuleId] = useState<string | null>(null)
  const [isCalibrating, setIsCalibrating] = useState<boolean>(false)
  const [isPanelOpen, setIsPanelOpen] = useState<boolean>(true)
  const [layoutMode, setLayoutMode] = useState<'overlay' | 'docked'>('overlay')
  const [fitMode, setFitMode] = useState<'fill' | 'fit'>('fill')
  const [tool, setTool] = useState<ToolMode>('select')
  const [currentPoints, setCurrentPoints] = useState<DetectionPoint[]>([])
  const [cursorPos, setCursorPos] = useState<DetectionPoint | null>(null)
  const [snapEnabled, setSnapEnabled] = useState(true)

  // 运动门控与防抖
  const [motionGateEnabled, setMotionGateEnabled] = useState<boolean>(true)
  const [motionGateThreshold, setMotionGateThreshold] = useState<number>(25)

  // 保存与反馈状态
  const [isSaving, setIsSaving] = useState(false)
  const [saveToast, setSaveToast] = useState<string | null>(null)

  // 交互拖拽状态
  const [draggingVertex, setDraggingVertex] = useState<{
    ruleId: string
    pointIndex: number
  } | null>(null)
  const [draggingShape, setDraggingShape] = useState<{
    ruleId: string
    startCursor: DetectionPoint
    initialPoints: DetectionPoint[]
  } | null>(null)

  const containerRef = useRef<HTMLDivElement>(null)
  const stageRef = useRef<HTMLDivElement>(null)
  const [stageSize, setStageSize] = useState<{ width: number; height: number } | null>(null)

  // 动态自适应画板尺寸（在保证 16:9 比例前提下最大化撑满可用黑底工作视口）
  useEffect(() => {
    const container = containerRef.current
    if (!container) return

    const updateSize = () => {
      const style = window.getComputedStyle(container)
      const padX = (parseFloat(style.paddingLeft) || 0) + (parseFloat(style.paddingRight) || 0)
      const padY = (parseFloat(style.paddingTop) || 0) + (parseFloat(style.paddingBottom) || 0)
      const availableW = Math.max(0, container.clientWidth - padX)
      const availableH = Math.max(0, container.clientHeight - padY)

      if (availableW <= 0 || availableH <= 0) return

      const targetAspect =
        camera.lastWidth && camera.lastHeight ? camera.lastWidth / camera.lastHeight : 16 / 9

      if (availableW / availableH > targetAspect) {
        // 容器空间更宽：高度撑满可用垂直空间，宽度按比例缩放
        setStageSize({
          width: Math.floor(availableH * targetAspect),
          height: Math.floor(availableH),
        })
      } else {
        // 容器空间更高：宽度撑满可用水平空间，高度按比例缩放
        setStageSize({
          width: Math.floor(availableW),
          height: Math.floor(availableW / targetAspect),
        })
      }
    }

    updateSize()
    const ro = new ResizeObserver(updateSize)
    ro.observe(container)
    return () => ro.disconnect()
  }, [camera.lastWidth, camera.lastHeight])

  // 1. 加载系统入库与激活的算法列表
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
            const schemaObj = (actVer?.configSchema as Record<string, unknown>) || {}
            const propertiesObj =
              (schemaObj.properties as Record<string, Record<string, unknown>>) || {}
            const targetClassesProp =
              propertiesObj.target_classes || propertiesObj.allowed_classes || propertiesObj.classes
            const enumClasses = (targetClassesProp?.items as Record<string, unknown>)?.enum as
              string[] | undefined
            const rawClasses = (actVer?.manifestRaw as Record<string, unknown>)?.classes as
              string[] | undefined
            const detectedClasses =
              enumClasses && enumClasses.length > 0
                ? enumClasses
                : rawClasses && rawClasses.length > 0
                  ? rawClasses
                  : []

            return {
              algorithmId: item.algorithmId,
              name: item.name,
              version: item.activeVersion || (actVer ? actVer.version : '1.0.0'),
              description: item.description,
              algorithmType: item.algorithmType,
              category: item.algorithmType || 'detection',
              supportedPlatforms: actVer ? [actVer.platformId] : ['linux-rknn'],
              alarmTypeId: item.alarmTypeId,
              author: item.isBuiltin ? 'System' : 'Custom',
              classes: detectedClasses,
              configSchema: schemaObj,
            }
          })
        } else {
          list = DEFAULT_ALGO_PACKAGES
        }

        setAvailableAlgos(list)
      })
      .catch(() => {
        if (isMounted) setAvailableAlgos(DEFAULT_ALGO_PACKAGES)
      })

    return () => {
      isMounted = false
    }
  }, [])

  // 2. 加载选定摄像头的任务布防配置
  useEffect(() => {
    taskApi
      .getTask(camera.cameraId)
      .then((dto) => {
        setTaskName(dto.name || camera.name || `Task-${camera.cameraId}`)
        setIsArmed(dto.desiredEnabled)
        if (dto.streamMode) {
          setStreamMode(dto.streamMode)
        }

        // 恢复所有已绑定的算法实例
        const instancesMap: Record<string, AlgorithmInstanceItem> = {}
        const dtoList = dto.algorithmInstances ?? []
        for (const inst of dtoList) {
          instancesMap[inst.algorithmId] = {
            algorithmId: inst.algorithmId,
            analysisFps: inst.analysisFps ?? 10,
            algoParams: (inst.algoParams as Record<string, unknown>) ?? {},
            enabled: inst.enabled ?? true,
          }
        }

        // 兼容单实例旧字段
        if (dto.algorithmId && !instancesMap[dto.algorithmId]) {
          instancesMap[dto.algorithmId] = {
            algorithmId: dto.algorithmId,
            analysisFps: dto.analysisFps ?? 10,
            algoParams: dto.algoParams ?? {},
            enabled: true,
          }
        }

        setActiveInstances(instancesMap)

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
            name: getDefaultRuleName(r.role, idx + 1, t),
            visible: true,
            color,
          }
        })
        setRules(extRules)
        if (extRules.length > 0) {
          setSelectedRuleId(extRules[0].id)
        }
      })
      .catch(() => {
        setTaskName(camera.name || `Task-${camera.cameraId}`)
      })
  }, [camera, t])

  // 3. 算法启闭切换操作
  const handleToggleAlgo = (algoId: string) => {
    setActiveInstances((prev) => {
      const existing = prev[algoId]
      if (existing) {
        return {
          ...prev,
          [algoId]: {
            ...existing,
            enabled: !existing.enabled,
          },
        }
      }

      // 首次激活：自动根据算法 schema 初始化官方推荐默认参数
      const found = availableAlgos.find((a) => a.algorithmId === algoId)
      const initialParams: Record<string, unknown> = {}
      if (found?.configSchema?.properties) {
        const props = found.configSchema.properties as Record<string, Record<string, unknown>>
        for (const [k, p] of Object.entries(props)) {
          if (p.default !== undefined) {
            initialParams[k] = p.default
          }
        }
      }
      if (found?.classes && found.classes.length > 0) {
        initialParams.target_classes = [...found.classes]
        initialParams.targetClasses = [...found.classes]
      }
      if (
        initialParams.confidence_threshold === undefined &&
        initialParams.confidenceThreshold === undefined
      ) {
        initialParams.confidence_threshold = 0.45
        initialParams.confidenceThreshold = 0.45
      }

      return {
        ...prev,
        [algoId]: {
          algorithmId: algoId,
          analysisFps: 10,
          algoParams: initialParams,
          enabled: true,
        },
      }
    })
  }

  // 4. 打开算法调参抽屉
  const handleOpenParams = (algo: AlgoManifest) => {
    setParamDrawerAlgo(algo)
  }

  // 5. 保存调参抽屉修改
  const handleSaveDrawerParams = (newParams: Record<string, unknown>) => {
    if (!paramDrawerAlgo) return
    const algoId = paramDrawerAlgo.algorithmId
    setActiveInstances((prev) => {
      const existing = prev[algoId]
      if (!existing) return prev
      return {
        ...prev,
        [algoId]: {
          ...existing,
          algoParams: newParams,
        },
      }
    })
  }

  const handleFpsChange = (newFps: number) => {
    if (!paramDrawerAlgo) return
    const algoId = paramDrawerAlgo.algorithmId
    setActiveInstances((prev) => {
      const existing = prev[algoId]
      if (!existing) return prev
      return {
        ...prev,
        [algoId]: {
          ...existing,
          analysisFps: newFps,
        },
      }
    })
  }

  // 6. 矢量点归一化映射与磁吸
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

  // 7. 完成并生成规则
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
        name: getDefaultRuleName(role, rules.length + 1, t),
        role,
        lineDirection: role === 'line' ? 'both' : undefined,
        points: [...points],
        visible: true,
        color: assignedColor,
      }

      setRules((prev) => [...prev, newRule])
      setSelectedRuleId(newRule.id)
      setCurrentPoints([])
      setTool('select')
    },
    [rules, t, tool],
  )

  const finishDrawing = useCallback(() => {
    finishDrawingPoints(currentPoints, tool)
  }, [currentPoints, finishDrawingPoints, tool])

  // 8. 键盘快捷键监听
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
        setIsCalibrating(true)
      }
      if (e.key === 'l' || e.key === 'L') {
        setTool('line')
        setCurrentPoints([])
        setIsCalibrating(true)
      }
      if (e.key === 'm' || e.key === 'M') {
        setTool('mask')
        setCurrentPoints([])
        setIsCalibrating(true)
      }
      if (e.key === 'Enter') {
        finishDrawing()
      }
      if (e.key === 'Escape') {
        if (paramDrawerAlgo) {
          setParamDrawerAlgo(null)
          return
        }
        if (currentPoints.length > 0) {
          setCurrentPoints([])
        } else if (tool !== 'select') {
          setTool('select')
        } else if (isCalibrating) {
          setIsCalibrating(false)
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
          setRules((prev) => prev.filter((r) => r.id !== selectedRuleId))
          setSelectedRuleId(null)
        }
      }
    }

    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [tool, currentPoints, selectedRuleId, onBack, paramDrawerAlgo, isCalibrating, finishDrawing])

  // 9. 画布拖拽监听
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

    if (tool === 'line') {
      if (currentPoints.length === 0) {
        setCurrentPoints([pt])
      } else {
        finishDrawingPoints([currentPoints[0], pt], 'line')
      }
    } else {
      if (currentPoints.length >= 3) {
        const first = currentPoints[0]
        const dist = Math.hypot(first.x - pt.x, first.y - pt.y)
        if (dist < 0.03) {
          finishDrawing()
          return
        }
      }
      setCurrentPoints((prev) => [...prev, pt])
    }
  }

  // 10. 保存布防任务配置
  const handleSave = async () => {
    setIsSaving(true)

    try {
      // 聚合所有启用的算法实例
      const payloadInstances: TaskAlgorithmInstanceDto[] = Object.values(activeInstances).map(
        (item) => ({
          algorithmId: item.algorithmId,
          analysisFps: item.analysisFps,
          algoParams: item.algoParams,
          enabled: item.enabled,
        }),
      )

      const finalName = taskName.trim() || camera.name || `Task-${camera.cameraId}`

      const payloadDto: TaskConfigDto = {
        cameraId: camera.cameraId,
        name: finalName,
        desiredEnabled: isArmed,
        streamMode,
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
        algorithmInstances: payloadInstances,
      }

      const updated = await taskApi.updateTask(camera.cameraId, payloadDto)
      setTaskName(updated.name || finalName)
      if (updated.streamMode) {
        setStreamMode(updated.streamMode)
      }
      setSaveToast(t('footer.saveSuccess', { defaultValue: '任务配置已保存并生效！' }))
      setTimeout(() => setSaveToast(null), 3000)
    } catch {
      setSaveToast(t('footer.saveFailed', { defaultValue: '保存失败，请检查网络或后端状态' }))
      setTimeout(() => setSaveToast(null), 3000)
    } finally {
      setIsSaving(false)
    }
  }

  const selectedRule = useMemo(() => {
    return rules.find((r) => r.id === selectedRuleId)
  }, [rules, selectedRuleId])

  return (
    <div className="flex h-full w-full max-w-full flex-1 flex-col overflow-hidden rounded-2xl border border-[var(--border)] bg-[var(--bg-primary)] text-[var(--text-primary)]">
      {/* 顶部综合态势导航栏 (UniFi / Verkada 风格 Header) */}
      <header className="frosted-glass relative z-20 flex h-14 shrink-0 items-center justify-between border-b border-[var(--border)] px-4 sm:px-6">
        {/* 左侧：返回 + 摄像头通道身份 + 原地编辑任务名 */}
        <div className="flex items-center gap-3">
          {onBack && (
            <button
              type="button"
              onClick={onBack}
              title={t('actions.backToTasks', { defaultValue: '返回任务列表' })}
              className="flex h-8 w-8 items-center justify-center rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] transition-colors hover:border-[var(--accent)] hover:text-[var(--accent)]"
            >
              <ArrowLeft className="h-4 w-4" />
            </button>
          )}

          {/* 摄像头通道信息 */}
          <div className="flex items-center gap-2 border-l border-[var(--border)] pl-3 text-xs">
            <span className="flex items-center gap-1.5 font-bold text-[var(--text-primary)]">
              <CameraIcon className="h-4 w-4 text-[var(--accent)]" />
              <span>{camera.name || camera.cameraId}</span>
            </span>
            <span className="hidden text-[var(--text-muted)] md:inline">·</span>
            <span className="hidden font-mono text-[11px] text-[var(--text-secondary)] md:inline">
              {camera.rtspUrl || '1080P RTSP'}
            </span>
            <span className="inline-flex items-center gap-1 rounded-full border border-emerald-500/20 bg-emerald-500/10 px-2 py-0.5 font-mono text-[10px] font-semibold text-emerald-400">
              <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-emerald-400" />
              ONLINE 1080P
            </span>
          </div>

          <span className="hidden h-4 w-[1px] bg-[var(--border)] sm:inline" />

          {/* 任务名称：支持原地点击快速改名 */}
          <div className="flex items-center gap-1.5 text-xs">
            <span className="text-[var(--text-muted)]">
              {t('taskName', { defaultValue: '任务名称:' })}
            </span>
            {isEditingTaskName ? (
              <input
                autoFocus
                type="text"
                value={taskName}
                onChange={(e) => setTaskName(e.target.value)}
                onBlur={() => setIsEditingTaskName(false)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') setIsEditingTaskName(false)
                }}
                className="rounded-lg border border-[var(--accent)] bg-[var(--bg-surface)] px-2 py-0.5 text-xs font-semibold text-[var(--text-primary)] ring-1 ring-[var(--accent)] outline-none"
              />
            ) : (
              <button
                type="button"
                onClick={() => setIsEditingTaskName(true)}
                className="group flex items-center gap-1 rounded-lg px-1.5 py-0.5 text-xs font-bold text-[var(--text-primary)] hover:bg-[var(--bg-secondary)]"
                title={t('clickToEditName', { defaultValue: '点击可原地快速修改任务名称' })}
              >
                <span>{taskName || `Task-${camera.cameraId}`}</span>
                <Pencil className="h-3 w-3 text-[var(--text-muted)] opacity-60 group-hover:text-[var(--accent)] group-hover:opacity-100" />
              </button>
            )}
          </div>
        </div>

        {/* 右侧：布局切换 + 全局布防开关 + 保存主操作 */}
        <div className="flex items-center gap-3 sm:gap-4">
          {saveToast && (
            <span className="animate-fade-in font-mono text-xs font-semibold text-emerald-400">
              {saveToast}
            </span>
          )}

          {/* 画面比例模式：铺满无黑边 (Fill) vs 等比保真 (Fit) */}
          <div className="flex items-center rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] p-0.5 text-xs">
            <button
              type="button"
              onClick={() => setFitMode('fill')}
              className={`flex items-center gap-1 rounded-md px-2 py-1 font-medium transition-colors ${
                fitMode === 'fill'
                  ? 'bg-[var(--accent)] text-white shadow-2xs'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
              title={t('studio.fitModeFillTitle', {
                defaultValue: '铺满全视口：画面撑满视口，消除上下左右所有空隙黑边',
              })}
            >
              <Maximize2 className="h-3.5 w-3.5" />
              <span className="hidden xl:inline">
                {t('studio.fitModeFill', { defaultValue: '撑满无留白' })}
              </span>
            </button>
            <button
              type="button"
              onClick={() => setFitMode('fit')}
              className={`flex items-center gap-1 rounded-md px-2 py-1 font-medium transition-colors ${
                fitMode === 'fit'
                  ? 'bg-[var(--accent)] text-white shadow-2xs'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
              title={t('studio.fitModeFitTitle', {
                defaultValue: '等比保真：严格保持原始 16:9 物理像素比例',
              })}
            >
              <Ratio className="h-3.5 w-3.5" />
              <span className="hidden xl:inline">
                {t('studio.fitModeFit', { defaultValue: '等比保真' })}
              </span>
            </button>
          </div>

          {/* 视图模式切换：沉浸全屏画板 (UniFi 浮动) vs 双栏停靠 (Verkada 并排) */}
          <div className="flex items-center rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] p-0.5 text-xs">
            <button
              type="button"
              onClick={() => setLayoutMode('overlay')}
              className={`flex items-center gap-1 rounded-md px-2 py-1 font-medium transition-colors ${
                layoutMode === 'overlay'
                  ? 'bg-[var(--accent)] text-white shadow-2xs'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
              title={t('studio.layoutOverlayTitle', {
                defaultValue: '沉浸全景画板 (UniFi 风格，16:9 全幅撑满视口，消除上下黑边)',
              })}
            >
              <Layers className="h-3.5 w-3.5" />
              <span className="hidden sm:inline">
                {t('studio.layoutOverlay', { defaultValue: '沉浸全屏' })}
              </span>
            </button>
            <button
              type="button"
              onClick={() => setLayoutMode('docked')}
              className={`flex items-center gap-1 rounded-md px-2 py-1 font-medium transition-colors ${
                layoutMode === 'docked'
                  ? 'bg-[var(--accent)] text-white shadow-2xs'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
              title={t('studio.layoutDockedTitle', {
                defaultValue: '双栏并排视图 (Verkada 风格，左侧视频与硬件遥测，右侧参数停靠)',
              })}
            >
              <Columns className="h-3.5 w-3.5" />
              <span className="hidden sm:inline">
                {t('studio.layoutDocked', { defaultValue: '双栏并排' })}
              </span>
            </button>
          </div>

          {/* AI 分析码流选择切换 */}
          <div className="hidden items-center rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] p-0.5 text-xs sm:flex">
            <button
              type="button"
              onClick={() => setStreamMode('main')}
              className={`flex items-center gap-1 rounded-md px-2 py-1 font-medium transition-colors ${
                streamMode === 'main'
                  ? 'bg-cyan-500 font-semibold text-black shadow-2xs'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
              title={t('streamMode.mainDesc', {
                defaultValue: '全高清原图硬件下采样，小目标与远距离识别最清晰，快照零延迟',
              })}
            >
              <span>{t('cardStream.main', { defaultValue: '主码流·高清' })}</span>
            </button>
            <button
              type="button"
              onClick={() => setStreamMode('sub')}
              className={`flex items-center gap-1 rounded-md px-2 py-1 font-medium transition-colors ${
                streamMode === 'sub'
                  ? 'bg-amber-500 font-semibold text-black shadow-2xs'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
              title={t('streamMode.subDesc', {
                defaultValue: '低码率子流推理，节约 VPU 算力，适合超多路密集布防',
              })}
            >
              <span>{t('cardStream.sub', { defaultValue: '子码流·低能耗' })}</span>
            </button>
            <button
              type="button"
              onClick={() => setStreamMode('auto')}
              className={`flex items-center gap-1 rounded-md px-2 py-1 font-medium transition-colors ${
                streamMode === 'auto'
                  ? 'bg-[var(--accent)] font-semibold text-white shadow-2xs'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
              title={t('streamMode.autoDesc', {
                defaultValue: '自动探活子码流，若无子流或不可用则自适应降级主码流',
              })}
            >
              <span>{t('cardStream.auto', { defaultValue: '自动码流' })}</span>
            </button>
          </div>

          {/* 全局布防总开关 */}
          <div className="flex items-center gap-2">
            <span className="text-xs font-medium text-[var(--text-secondary)]">
              {isArmed
                ? t('status.armed', { defaultValue: '已布防' })
                : t('status.disarmed', { defaultValue: '已撤防' })}
            </span>
            <button
              type="button"
              onClick={() => setIsArmed(!isArmed)}
              className={`relative inline-flex h-6 w-11 shrink-0 cursor-pointer items-center rounded-full transition-colors ${
                isArmed ? 'bg-[var(--accent)]' : 'bg-zinc-700'
              }`}
            >
              <span
                className={`inline-block h-4 w-4 transform rounded-full bg-white shadow-md transition-transform ${
                  isArmed ? 'translate-x-6' : 'translate-x-1'
                }`}
              />
            </button>
          </div>

          {/* 保存任务按钮 */}
          <button
            type="button"
            onClick={handleSave}
            disabled={isSaving}
            className="flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 active:scale-95 disabled:opacity-50"
          >
            <Save className="h-4 w-4" />
            <span>
              {isSaving
                ? t('footer.saving', { defaultValue: '正在保存...' })
                : t('footer.saveTask', { defaultValue: '保存配置' })}
            </span>
          </button>

          {/* 展开/收起右侧控制侧边栏 */}
          <button
            type="button"
            onClick={() => setIsPanelOpen(!isPanelOpen)}
            title={
              isPanelOpen
                ? t('studio.collapsePanel', { defaultValue: '收起配置面板' })
                : t('studio.expandPanel', { defaultValue: '展开配置面板' })
            }
            className={`hidden h-8 w-8 items-center justify-center rounded-lg border transition-colors lg:flex ${
              isPanelOpen
                ? 'border-[var(--accent)] bg-[var(--accent-soft)] text-[var(--accent)]'
                : 'border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
            }`}
          >
            {isPanelOpen ? (
              <PanelRightClose className="h-4 w-4" />
            ) : (
              <PanelRightOpen className="h-4 w-4" />
            )}
          </button>
        </div>
      </header>

      {/* 主视口工作区：支持沉浸全屏 (UniFi 浮动) 与双栏并排 (Verkada 停靠) */}
      <div
        className={`relative flex min-h-0 min-w-0 flex-1 overflow-hidden ${
          layoutMode === 'docked' ? 'flex-col lg:flex-row' : ''
        }`}
      >
        {/* 视频主视区 */}
        <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
          <div
            ref={containerRef}
            className="relative flex min-h-0 min-w-0 flex-1 items-center justify-center overflow-hidden bg-black/95 p-2 sm:p-3"
          >
            <div
              ref={stageRef}
              onMouseDown={handleStageMouseDown}
              onMouseMove={handleStageMouseMove}
              onMouseUp={handleStageMouseUp}
              onClick={handleStageClick}
              className="relative flex items-center justify-center overflow-hidden rounded-xl border border-white/10 bg-black shadow-2xl"
              style={
                fitMode === 'fill'
                  ? {
                      width: '100%',
                      height: '100%',
                    }
                  : stageSize
                    ? {
                        width: `${stageSize.width}px`,
                        height: `${stageSize.height}px`,
                      }
                    : {
                        width: '100%',
                        height: '100%',
                        maxWidth: '100%',
                        maxHeight: '100%',
                        aspectRatio:
                          camera.lastWidth && camera.lastHeight
                            ? `${camera.lastWidth} / ${camera.lastHeight}`
                            : '16 / 9',
                      }
              }
            >
              {/* 实时分析源预览播放器 (主码流或子码流) */}
              <LivePlayer
                cameraId={camera.cameraId}
                cameraName={camera.name}
                stream={effectivePreviewStream}
                fitMode={fitMode === 'fill' ? 'fill' : 'contain'}
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

                {/* 既有防区规则绘制 */}
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
                      />
                    </g>
                  )
                })}

                {/* 绘制中的交互引导与点线 */}
                {currentPoints.length > 0 &&
                  (() => {
                    const activeToolTheme = getToolTheme(
                      tool,
                      rules.filter((r) => r.role === 'roi').length,
                    )
                    return (
                      <g className="pointer-events-none">
                        {tool !== 'rect' && tool !== 'precrop' && currentPoints.length >= 2 && (
                          <polyline
                            points={currentPoints.map((p) => `${p.x * 100},${p.y * 100}`).join(' ')}
                            fill="none"
                            stroke={activeToolTheme.stroke}
                            strokeWidth={1.5}
                            vectorEffect="non-scaling-stroke"
                          />
                        )}
                        {cursorPos && currentPoints.length >= 1 && (
                          <line
                            x1={`${currentPoints[currentPoints.length - 1].x * 100}%`}
                            y1={`${currentPoints[currentPoints.length - 1].y * 100}%`}
                            x2={`${cursorPos.x * 100}%`}
                            y2={`${cursorPos.y * 100}%`}
                            stroke={activeToolTheme.stroke}
                            strokeWidth={1}
                            strokeDasharray="3 3"
                            vectorEffect="non-scaling-stroke"
                          />
                        )}
                      </g>
                    )
                  })()}
              </svg>

              {/* 标定模式浮动工具栏 (位于视频上方居中) */}
              {isCalibrating && (
                <div className="frosted-glass absolute top-4 left-1/2 z-30 flex -translate-x-1/2 items-center gap-1.5 rounded-full border border-[var(--border)] bg-black/75 p-1.5 shadow-2xl backdrop-blur-md">
                  <button
                    type="button"
                    onClick={() => {
                      setTool('select')
                      setCurrentPoints([])
                    }}
                    className={`flex items-center gap-1 rounded-full px-3 py-1 text-xs font-semibold transition-all ${
                      tool === 'select'
                        ? 'bg-[var(--accent)] text-white shadow-xs'
                        : 'text-white/80 hover:text-white'
                    }`}
                  >
                    <MousePointer2 className="h-3.5 w-3.5" />
                    <span>{t('tools.select')} (V)</span>
                  </button>

                  <button
                    type="button"
                    onClick={() => {
                      setTool('roi')
                      setCurrentPoints([])
                    }}
                    className={`flex items-center gap-1 rounded-full px-3 py-1 text-xs font-semibold transition-all ${
                      tool === 'roi'
                        ? 'bg-cyan-500 text-white shadow-xs'
                        : 'text-white/80 hover:text-white'
                    }`}
                  >
                    <Hexagon className="h-3.5 w-3.5" />
                    <span>{t('tools.roi')} (P)</span>
                  </button>

                  <button
                    type="button"
                    onClick={() => {
                      setTool('line')
                      setCurrentPoints([])
                    }}
                    className={`flex items-center gap-1 rounded-full px-3 py-1 text-xs font-semibold transition-all ${
                      tool === 'line'
                        ? 'bg-emerald-500 text-white shadow-xs'
                        : 'text-white/80 hover:text-white'
                    }`}
                  >
                    <Slash className="h-3.5 w-3.5" />
                    <span>{t('tools.line')} (L)</span>
                  </button>

                  <button
                    type="button"
                    onClick={() => {
                      setTool('mask')
                      setCurrentPoints([])
                    }}
                    className={`flex items-center gap-1 rounded-full px-3 py-1 text-xs font-semibold transition-all ${
                      tool === 'mask'
                        ? 'bg-rose-500 text-white shadow-xs'
                        : 'text-white/80 hover:text-white'
                    }`}
                  >
                    <ShieldAlert className="h-3.5 w-3.5" />
                    <span>{t('tools.mask')} (M)</span>
                  </button>

                  <span className="mx-1 h-3.5 w-[1px] bg-white/20" />

                  <button
                    type="button"
                    onClick={() => setSnapEnabled(!snapEnabled)}
                    className={`flex items-center gap-1 rounded-full px-2.5 py-1 text-xs font-semibold ${
                      snapEnabled ? 'text-cyan-400' : 'text-white/50'
                    }`}
                    title={t('studio.snapMagnet', { defaultValue: '磁吸吸附' })}
                  >
                    <Magnet className="h-3.5 w-3.5" />
                  </button>

                  <button
                    type="button"
                    onClick={() => setIsCalibrating(false)}
                    className="flex items-center gap-1 rounded-full bg-white/15 px-3 py-1 text-xs font-semibold text-white hover:bg-white/25"
                  >
                    <Check className="h-3.5 w-3.5 text-emerald-400" />
                    <span>{t('studio.doneCalibration', { defaultValue: '完成标定 (ESC)' })}</span>
                  </button>
                </div>
              )}

              {/* 视频右上角：当前防区快速指示 / 属性面板 */}
              <div className="absolute top-4 right-4 z-20 flex flex-col items-end gap-2">
                {selectedRule && isCalibrating && (
                  <div className="frosted-glass flex w-64 flex-col gap-2 rounded-2xl border border-[var(--border)] bg-black/80 p-3 text-xs text-white shadow-2xl backdrop-blur-md">
                    <div className="flex items-center justify-between">
                      <span className="font-bold text-[var(--accent)]">{selectedRule.name}</span>
                      <button
                        type="button"
                        onClick={() => setSelectedRuleId(null)}
                        className="text-white/60 hover:text-white"
                      >
                        <X className="h-3.5 w-3.5" />
                      </button>
                    </div>

                    {/* 若为绊线，提供方向切换 */}
                    {selectedRule.role === 'line' && (
                      <div className="flex items-center justify-between gap-1 pt-1 font-mono text-[11px]">
                        <span>{t('inspector.lineDirection', { defaultValue: '方向:' })}</span>
                        <select
                          value={selectedRule.lineDirection || 'both'}
                          onChange={(e) => {
                            const dir = e.target.value as DetectionLineDirection
                            setRules((prev) =>
                              prev.map((r) =>
                                r.id === selectedRule.id ? { ...r, lineDirection: dir } : r,
                              ),
                            )
                          }}
                          className="rounded border border-white/20 bg-black/60 px-1.5 py-0.5 text-white outline-none"
                        >
                          <option value="both">
                            {t('inspector.dirBoth', { defaultValue: '双向 ⇄' })}
                          </option>
                          <option value="a_to_b">
                            {t('inspector.dirAtoB', { defaultValue: 'A → B' })}
                          </option>
                          <option value="b_to_a">
                            {t('inspector.dirBtoA', { defaultValue: 'B → A' })}
                          </option>
                        </select>
                      </div>
                    )}

                    <div className="flex items-center justify-between border-t border-white/10 pt-2">
                      <button
                        type="button"
                        onClick={() => {
                          setRules((prev) => prev.filter((r) => r.id !== selectedRule.id))
                          setSelectedRuleId(null)
                        }}
                        className="flex items-center gap-1 text-[11px] text-rose-400 hover:text-rose-300"
                      >
                        <Trash2 className="h-3 w-3" />
                        <span>{t('inspector.delete', { defaultValue: '删除防区' })}</span>
                      </button>
                    </div>
                  </div>
                )}
              </div>
            </div>
          </div>

          {/* 仅在双栏并排 (Docked) 模式下呈现硬件与推理指标遥测底栏 (彻底消除上下黑边无用空隙) */}
          {layoutMode === 'docked' && (
            <div className="flex shrink-0 items-center justify-between border-t border-[var(--border)] bg-[var(--bg-surface)] px-4 py-2 font-mono text-[11px] text-[var(--text-secondary)]">
              <div className="flex items-center gap-3">
                <span className="flex items-center gap-1.5 font-semibold text-emerald-400">
                  <span className="h-2 w-2 animate-pulse rounded-full bg-emerald-400" />
                  <span>
                    {t('studio.telemetryZeroCopy', {
                      defaultValue: '硬件零拷贝解码: VPU DMA-BUF',
                    })}
                  </span>
                </span>
                <span>·</span>
                <span>
                  {t('studio.telemetryInferEngine', { defaultValue: '推理核心: RKNN NPU 2.0' })}
                </span>
                <span>·</span>
                <span>
                  {t('studio.telemetryResolution', { defaultValue: '源分辨率:' })}{' '}
                  {camera.lastWidth || 1920}×{camera.lastHeight || 1080}
                </span>
              </div>
              <div className="flex items-center gap-3">
                <span className="font-semibold text-cyan-300">
                  {t('studio.telemetryLatency', { defaultValue: '流延迟:' })} ~106ms
                </span>
                <span>·</span>
                <span>{t('studio.telemetryFps', { defaultValue: '实时帧率:' })} 25.0 FPS</span>
              </div>
            </div>
          )}
        </div>

        {/* 控制面板：在 Overlay 模式下为悬浮玻璃卡片 (不挤压视频画幅)，在 Docked 模式下为右侧固定停靠栏 */}
        {isPanelOpen && (
          <aside
            className={
              layoutMode === 'overlay'
                ? 'frosted-glass animate-fade-in absolute top-3 right-3 bottom-3 z-30 flex w-80 flex-col space-y-4 overflow-y-auto rounded-2xl border border-white/15 bg-black/75 p-4 shadow-2xl backdrop-blur-xl sm:w-96'
                : 'flex w-full shrink-0 flex-col space-y-4 overflow-y-auto border-t border-[var(--border)] bg-[var(--bg-surface-solid)] p-4 lg:w-[380px] lg:border-t-0 lg:border-l xl:w-[420px] 2xl:w-[460px]'
            }
          >
            {/* 1. AI 智能检测引擎方块机架 */}
            <AlgorithmRack
              availableAlgos={availableAlgos}
              activeInstances={activeInstances}
              onToggleAlgo={handleToggleAlgo}
              onOpenParams={handleOpenParams}
            />

            <div className="h-px bg-[var(--border)]" />

            {/* 2. 空间活动防区卡片组 */}
            <ActivityZonesSection
              rules={rules}
              selectedRuleId={selectedRuleId}
              onSelectRule={setSelectedRuleId}
              onToggleRuleVisible={(ruleId) =>
                setRules((prev) =>
                  prev.map((r) => (r.id === ruleId ? { ...r, visible: !r.visible } : r)),
                )
              }
              onDeleteRule={(ruleId) => {
                setRules((prev) => prev.filter((r) => r.id !== ruleId))
                if (selectedRuleId === ruleId) setSelectedRuleId(null)
              }}
              onEnterCalibration={() => setIsCalibrating(true)}
              isCalibrating={isCalibrating}
            />

            <div className="h-px bg-[var(--border)]" />

            {/* 3. 运动检测门控卡片 */}
            <div className="space-y-2.5 rounded-2xl border border-[var(--border)] bg-[var(--bg-surface)] p-3.5">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <Activity className="h-4 w-4 text-[var(--accent)]" />
                  <span className="text-xs font-bold text-[var(--text-primary)]">
                    {t('studio.motionGateTitle', { defaultValue: '运动检测门控 (Motion Gating)' })}
                  </span>
                </div>
                <button
                  type="button"
                  onClick={() => setMotionGateEnabled(!motionGateEnabled)}
                  className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full transition-colors ${
                    motionGateEnabled ? 'bg-[var(--accent)]' : 'bg-zinc-700'
                  }`}
                >
                  <span
                    className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white shadow-md transition-transform ${
                      motionGateEnabled ? 'translate-x-4' : 'translate-x-0.5'
                    }`}
                  />
                </button>
              </div>
              <p className="text-[11px] leading-relaxed text-[var(--text-muted)]">
                {t('studio.motionGateDesc', {
                  defaultValue:
                    '画面静止无像素变动时跳过 NPU 深度推理，极大降低芯片能耗与总线发热。',
                })}
              </p>
              {motionGateEnabled && (
                <div className="space-y-1.5 border-t border-[var(--border)] pt-2">
                  <div className="flex items-center justify-between text-[11px]">
                    <span className="text-[var(--text-secondary)]">
                      {t('studio.motionGateSensitivity', { defaultValue: '灵敏度阈值:' })}
                    </span>
                    <span className="font-mono font-semibold text-[var(--accent)]">
                      {motionGateThreshold}%
                    </span>
                  </div>
                  <input
                    type="range"
                    min={5}
                    max={80}
                    step={1}
                    value={motionGateThreshold}
                    onChange={(e) => setMotionGateThreshold(Number(e.target.value))}
                    className="h-1.5 w-full cursor-pointer appearance-none rounded-lg bg-[var(--bg-secondary)] accent-[var(--accent)]"
                  />
                </div>
              )}
            </div>
          </aside>
        )}
      </div>

      {/* 算法参数独立调优抽屉 (点击算法小方块的 ⚙️ 触发) */}
      <AlgoParamDrawer
        isOpen={Boolean(paramDrawerAlgo)}
        algo={paramDrawerAlgo}
        fps={
          paramDrawerAlgo ? (activeInstances[paramDrawerAlgo.algorithmId]?.analysisFps ?? 10) : 10
        }
        onFpsChange={handleFpsChange}
        params={
          paramDrawerAlgo ? (activeInstances[paramDrawerAlgo.algorithmId]?.algoParams ?? {}) : {}
        }
        onSaveParams={handleSaveDrawerParams}
        onClose={() => setParamDrawerAlgo(null)}
      />

      {/* 算法沙箱详情只读抽屉 */}
      <AlgoSandboxDrawer
        isOpen={isAlgoSandboxOpen}
        algo={sandboxAlgo}
        onClose={() => setIsAlgoSandboxOpen(false)}
        onNavigateToAlgorithms={onNavigateToAlgorithms}
      />
    </div>
  )
}
