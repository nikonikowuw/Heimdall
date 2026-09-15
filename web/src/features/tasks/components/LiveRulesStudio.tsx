import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react'
import { Activity, ArrowLeft, Camera as CameraIcon, Layers, Pencil, Save, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { LivePlayer } from '@/features/live/components/LivePlayer'
import { algorithmApi, taskApi } from '@/lib/api'
import { telemetryStore } from '@/lib/telemetryStore'
import type {
  AlgoManifest,
  Camera,
  DetectionLineDirection,
  DetectionPoint,
  StreamMode,
  TaskAlgorithmInstanceDto,
  TaskConfigDto,
} from '@/types'
import { useUndoableState } from '../hooks/use-undoable-state'
import { ActivityZonesSection } from './ActivityZonesSection'
import { AlgoParamDrawer } from './AlgoParamDrawer'
import { AlgoSandboxDrawer } from './AlgoSandboxDrawer'
import { AlgorithmInstanceItem, AlgorithmRack } from './AlgorithmRack'
import { extractTargetClasses } from '../algoMetadata'
import { RulePropertiesPanel } from './RulePropertiesPanel'
import { StudioToolIsland } from './StudioToolIsland'
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

/** 各工具的画板操作提示（复用既有 tools.hud* 文案） */
const HUD_HINT_KEY: Record<ToolMode, string> = {
  select: 'tools.hudSelect',
  roi: 'tools.hudRoi',
  polygon: 'tools.hudPolygon',
  rect: 'tools.hudRect',
  precrop: 'tools.hudRect',
  line: 'tools.hudLine',
  mask: 'tools.hudMask',
}

const DRAW_TOOLS: ReadonlySet<ToolMode> = new Set<ToolMode>([
  'roi',
  'polygon',
  'rect',
  'precrop',
  'line',
  'mask',
])

function clamp01(value: number): number {
  return Math.max(0, Math.min(1, value))
}

function round4(value: number): number {
  return Number(value.toFixed(4))
}

/** 规则在画面上的标注锚点：多边形取顶点均值，绊线取中点 */
function getRuleAnchor(rule: ExtendedRule): DetectionPoint | null {
  if (rule.points.length < 2) return null
  if (rule.role === 'line') {
    const [a, b] = rule.points
    return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 }
  }
  const sum = rule.points.reduce((acc, p) => ({ x: acc.x + p.x, y: acc.y + p.y }), { x: 0, y: 0 })
  return { x: sum.x / rule.points.length, y: sum.y / rule.points.length }
}

interface ResolvedPoint {
  x: number
  y: number
  /** 命中的吸附顶点（用于吸附高亮反馈） */
  snapped: DetectionPoint | null
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

  // 空间防区（可撤销/重做，历史有界）
  const {
    value: rules,
    set: setRules,
    undo: undoRuleEdit,
    redo: redoRuleEdit,
    reset: resetRules,
    beginGesture,
    endGesture,
    canUndo,
    canRedo,
  } = useUndoableState<ExtendedRule[]>([])

  const [selectedRuleId, setSelectedRuleId] = useState<string | null>(null)
  const [isPanelOpen, setIsPanelOpen] = useState<boolean>(true)
  const [tool, setTool] = useState<ToolMode>('select')
  const [currentPoints, setCurrentPoints] = useState<DetectionPoint[]>([])
  const [draftCursor, setDraftCursor] = useState<{
    cursor: DetectionPoint | null
    snapped: DetectionPoint | null
  }>({ cursor: null, snapped: null })
  const [snapEnabled, setSnapEnabled] = useState(true)

  // 运动门控与防抖
  const [motionGateEnabled, setMotionGateEnabled] = useState<boolean>(true)
  const [motionGateThreshold, setMotionGateThreshold] = useState<number>(25)

  // 保存与反馈状态
  const [isSaving, setIsSaving] = useState(false)
  const [saveToast, setSaveToast] = useState<string | null>(null)
  const toastTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(
    () => () => {
      if (toastTimerRef.current) clearTimeout(toastTimerRef.current)
    },
    [],
  )

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
  // 拖拽结束后紧接着的 click 不应被当作“点空白取消选中”
  const suppressClickRef = useRef(false)
  const [stageSize, setStageSize] = useState<{ width: number; height: number } | null>(null)

  // 真实遥测：活跃航迹与运动门控状态由后端 WS 推送
  const telemetry = useSyncExternalStore(
    useCallback(
      (onStoreChange) => telemetryStore.subscribe(camera.cameraId, onStoreChange),
      [camera.cameraId],
    ),
    useCallback(() => telemetryStore.getTelemetry(camera.cameraId), [camera.cameraId]),
    () => undefined,
  )

  // 动态自适应画板尺寸（严格保持摄像头物理宽高比，杜绝画面拉伸导致的标定视觉失真）
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
              classes: extractTargetClasses(actVer),
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
    let isMounted = true
    taskApi
      .getTask(camera.cameraId)
      .then((dto) => {
        if (!isMounted) return
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
        resetRules(extRules)
        setSelectedRuleId(extRules.length > 0 ? extRules[0].id : null)
        // 新建任务尚无任何防区时，直接切到绘制工具，进入工作台即可下笔
        setTool(extRules.length === 0 ? 'roi' : 'select')
      })
      .catch(() => {
        if (isMounted) {
          setTaskName(camera.name || `Task-${camera.cameraId}`)
        }
      })

    return () => {
      isMounted = false
    }
  }, [camera, t, resetRules])

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

  // 6. 归一化坐标映射与顶点磁吸
  const resolvePoint = useCallback(
    (
      e: { clientX: number; clientY: number },
      options?: {
        skip?: { ruleId: string; pointIndex: number }
        disableSnap?: boolean
      },
    ): ResolvedPoint => {
      const stage = stageRef.current
      if (!stage) return { x: 0, y: 0, snapped: null }
      const rect = stage.getBoundingClientRect()
      let x = clamp01((e.clientX - rect.left) / rect.width)
      let y = clamp01((e.clientY - rect.top) / rect.height)

      let snapped: DetectionPoint | null = null
      if (snapEnabled && !options?.disableSnap) {
        const threshold = 0.02
        let bestDistance = threshold
        for (const rule of rules) {
          if (!rule.visible) continue
          for (let i = 0; i < rule.points.length; i += 1) {
            if (options?.skip && rule.id === options.skip.ruleId && i === options.skip.pointIndex) {
              continue
            }
            const candidate = rule.points[i]
            const distance = Math.hypot(candidate.x - x, candidate.y - y)
            if (distance < bestDistance) {
              bestDistance = distance
              snapped = candidate
            }
          }
        }
        if (snapped) {
          x = snapped.x
          y = snapped.y
        }
      }

      return { x: round4(x), y: round4(y), snapped }
    },
    [snapEnabled, rules],
  )

  // 7. 完成并生成规则
  const finishDrawingPoints = useCallback(
    (points: DetectionPoint[], activeTool: ToolMode = tool) => {
      if (activeTool === 'line') {
        if (points.length < 2) {
          setCurrentPoints([])
          setDraftCursor({ cursor: null, snapped: null })
          return
        }
      } else if (points.length < 3) {
        setCurrentPoints([])
        setDraftCursor({ cursor: null, snapped: null })
        return
      }

      const role = activeTool === 'line' ? 'line' : activeTool === 'mask' ? 'mask' : 'roi'
      const existingRoiCount = rules.filter((r) => r.role === 'roi').length

      const newRule: ExtendedRule = {
        id: `rule_${Date.now()}`,
        name: getDefaultRuleName(role, rules.length + 1, t),
        role,
        lineDirection: role === 'line' ? 'both' : undefined,
        points: [...points],
        visible: true,
        color: getInitialRuleColor(role, existingRoiCount),
      }

      setRules((prev) => [...prev, newRule])
      setSelectedRuleId(newRule.id)
      setCurrentPoints([])
      setDraftCursor({ cursor: null, snapped: null })
      setTool('select')
    },
    [rules, setRules, t, tool],
  )

  const finishDrawing = useCallback(() => {
    finishDrawingPoints(currentPoints, tool)
  }, [currentPoints, finishDrawingPoints, tool])

  const handleSelectTool = useCallback((nextTool: ToolMode) => {
    setTool(nextTool)
    setCurrentPoints([])
    setDraftCursor({ cursor: null, snapped: null })
    if (nextTool !== 'select') {
      setSelectedRuleId(null)
    }
  }, [])

  const handleCancelDrawing = useCallback(() => {
    setCurrentPoints([])
    setDraftCursor({ cursor: null, snapped: null })
    setTool('select')
  }, [])

  // 8. 键盘快捷键监听
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return

      const withMeta = e.ctrlKey || e.metaKey
      if (withMeta && (e.key === 'z' || e.key === 'Z')) {
        e.preventDefault()
        if (e.shiftKey) {
          redoRuleEdit()
        } else {
          undoRuleEdit()
        }
        setCurrentPoints([])
        setDraftCursor({ cursor: null, snapped: null })
        return
      }
      if (withMeta && (e.key === 'y' || e.key === 'Y')) {
        e.preventDefault()
        redoRuleEdit()
        setCurrentPoints([])
        setDraftCursor({ cursor: null, snapped: null })
        return
      }
      if (withMeta) return

      if (e.key === 'v' || e.key === 'V') {
        handleSelectTool('select')
      }
      if (e.key === 'r' || e.key === 'R' || e.key === 'p' || e.key === 'P') {
        handleSelectTool('roi')
      }
      if (e.key === 'l' || e.key === 'L') {
        handleSelectTool('line')
      }
      if (e.key === 'm' || e.key === 'M') {
        handleSelectTool('mask')
      }
      if (e.key === 's' || e.key === 'S') {
        setSnapEnabled((prev) => !prev)
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
          setDraftCursor({ cursor: null, snapped: null })
        } else if (tool !== 'select') {
          handleSelectTool('select')
        } else if (selectedRuleId) {
          setSelectedRuleId(null)
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
  }, [
    tool,
    currentPoints,
    selectedRuleId,
    onBack,
    paramDrawerAlgo,
    finishDrawing,
    handleSelectTool,
    setRules,
    undoRuleEdit,
    redoRuleEdit,
  ])

  // 9. 画布拖拽监听（顶点微调与整体位移，整段手势只入栈一条历史）
  useEffect(() => {
    if (!draggingVertex && !draggingShape) return

    const handleGlobalMouseMove = (e: MouseEvent) => {
      if (draggingVertex) {
        const pt = resolvePoint(e, { skip: draggingVertex })
        setRules(
          (prev) =>
            prev.map((r) => {
              if (r.id !== draggingVertex.ruleId) return r
              const pts = [...r.points]
              pts[draggingVertex.pointIndex] = { x: pt.x, y: pt.y }
              return { ...r, points: pts }
            }),
          { history: false },
        )
        return
      }

      if (draggingShape) {
        const pt = resolvePoint(e, { disableSnap: true })
        const dx = pt.x - draggingShape.startCursor.x
        const dy = pt.y - draggingShape.startCursor.y
        setRules(
          (prev) =>
            prev.map((r) => {
              if (r.id !== draggingShape.ruleId) return r
              const movedPoints = draggingShape.initialPoints.map((p) => ({
                x: round4(clamp01(p.x + dx)),
                y: round4(clamp01(p.y + dy)),
              }))
              return { ...r, points: movedPoints }
            }),
          { history: false },
        )
      }
    }

    const handleGlobalMouseUp = () => {
      if (draggingVertex || draggingShape) {
        suppressClickRef.current = true
      }
      setDraggingVertex(null)
      setDraggingShape(null)
      endGesture()
    }

    window.addEventListener('mousemove', handleGlobalMouseMove)
    window.addEventListener('mouseup', handleGlobalMouseUp)
    return () => {
      window.removeEventListener('mousemove', handleGlobalMouseMove)
      window.removeEventListener('mouseup', handleGlobalMouseUp)
    }
  }, [draggingVertex, draggingShape, resolvePoint, setRules, endGesture])

  const handleStageMouseDown = (e: React.MouseEvent<HTMLDivElement>) => {
    if (e.button !== 0 || tool !== 'select' || !selectedRuleId) return
    const { x, y } = resolvePoint(e)
    const rule = rules.find((r) => r.id === selectedRuleId)
    if (!rule || !rule.visible) return
    const pt = { x, y }
    const hit =
      rule.role === 'line'
        ? rule.points.length >= 2 && isPointNearLine(pt, rule.points[0], rule.points[1])
        : isPointInPolygon(pt, rule.points)
    if (hit) {
      beginGesture()
      setDraggingShape({
        ruleId: rule.id,
        startCursor: pt,
        initialPoints: rule.points.map((p) => ({ ...p })),
      })
    }
  }

  const handleStageMouseMove = (e: React.MouseEvent<HTMLDivElement>) => {
    if (draggingVertex || draggingShape) return
    // 仅在绘制态跟踪光标，避免选择态下每次移动都触发重渲染
    if (!DRAW_TOOLS.has(tool) || currentPoints.length === 0) return
    const resolved = resolvePoint(e)
    setDraftCursor({
      cursor: { x: resolved.x, y: resolved.y },
      snapped: resolved.snapped,
    })
  }

  const handleStageClick = (e: React.MouseEvent<HTMLDivElement>) => {
    if (suppressClickRef.current) {
      suppressClickRef.current = false
      return
    }
    if (tool === 'select') {
      setSelectedRuleId(null)
      return
    }
    const { x, y } = resolvePoint(e)
    const pt = { x, y }

    if (tool === 'line') {
      if (currentPoints.length === 0) {
        setCurrentPoints([pt])
      } else {
        finishDrawingPoints([currentPoints[0], pt], 'line')
      }
      return
    }

    if (currentPoints.length >= 3) {
      const first = currentPoints[0]
      if (Math.hypot(first.x - pt.x, first.y - pt.y) < 0.03) {
        finishDrawing()
        return
      }
    }
    setCurrentPoints((prev) => [...prev, pt])
  }

  const handleStageDoubleClick = () => {
    if (!DRAW_TOOLS.has(tool) || tool === 'line') return
    const points = [...currentPoints]
    // 双击的第二次 click 会追加一个近乎重合的顶点，闭合前先剔除
    if (points.length >= 4) {
      const last = points[points.length - 1]
      const prev = points[points.length - 2]
      if (Math.hypot(last.x - prev.x, last.y - prev.y) < 0.02) {
        points.pop()
      }
    }
    finishDrawingPoints(points, tool)
  }

  const handleVertexMouseDown = (
    e: React.MouseEvent<HTMLDivElement>,
    ruleId: string,
    pointIndex: number,
  ) => {
    e.stopPropagation()
    e.preventDefault()
    beginGesture()
    setDraggingVertex({ ruleId, pointIndex })
  }
  // 10. 规则编辑操作
  const handleUpdateRule = useCallback(
    (ruleId: string, partial: Partial<ExtendedRule>) => {
      setRules((prev) => prev.map((r) => (r.id === ruleId ? { ...r, ...partial } : r)))
    },
    [setRules],
  )

  const handleDeleteRule = useCallback(
    (ruleId: string) => {
      setRules((prev) => prev.filter((r) => r.id !== ruleId))
      setSelectedRuleId((prev) => (prev === ruleId ? null : prev))
    },
    [setRules],
  )

  const handleCloneRule = useCallback(
    (ruleId: string) => {
      const source = rules.find((r) => r.id === ruleId)
      if (!source) return
      const cloneId = `rule_${Date.now()}`
      const offset = 0.03
      const clone: ExtendedRule = {
        ...source,
        id: cloneId,
        name: `${source.name} ${t('inspector.copySuffix', { defaultValue: '副本' })}`,
        points: source.points.map((p) => ({
          x: round4(clamp01(p.x + offset)),
          y: round4(clamp01(p.y + offset)),
        })),
      }
      setRules((prev) => [...prev, clone])
      setSelectedRuleId(cloneId)
    },
    [rules, setRules, t],
  )

  const handleToggleRuleVisible = useCallback(
    (ruleId: string) => {
      setRules((prev) => prev.map((r) => (r.id === ruleId ? { ...r, visible: !r.visible } : r)))
    },
    [setRules],
  )

  // 11. 保存布防任务配置
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
          motionHoldFrames: 10,
        },
        algorithmInstances: payloadInstances,
      }

      const updated = await taskApi.updateTask(camera.cameraId, payloadDto)
      setTaskName(updated.name || finalName)
      if (updated.streamMode) {
        setStreamMode(updated.streamMode)
      }
      setSaveToast(t('footer.saveSuccess', { defaultValue: '任务配置已保存并生效！' }))
    } catch {
      setSaveToast(t('footer.saveFailed', { defaultValue: '保存失败，请检查网络或后端状态' }))
    } finally {
      if (toastTimerRef.current) clearTimeout(toastTimerRef.current)
      toastTimerRef.current = setTimeout(() => setSaveToast(null), 3000)
      setIsSaving(false)
    }
  }

  const selectedRule = useMemo(
    () => rules.find((r) => r.id === selectedRuleId),
    [rules, selectedRuleId],
  )

  const activeAlgorithmNames = useMemo(
    () =>
      Object.values(activeInstances)
        .filter((item) => item.enabled)
        .map(
          (item) =>
            availableAlgos.find((algo) => algo.algorithmId === item.algorithmId)?.name ??
            item.algorithmId,
        ),
    [activeInstances, availableAlgos],
  )

  const isDrawing = currentPoints.length > 0
  const hudHint = t(HUD_HINT_KEY[tool], { defaultValue: t('tools.hudSelect') })

  return (
    <div className="flex h-full w-full max-w-full flex-1 flex-col overflow-hidden rounded-[10px] border border-t-2 border-[var(--border-strong)] border-t-[var(--accent)] bg-[var(--bg-primary)] text-[var(--text-primary)]">
      {/* 顶部综合态势导航栏 */}
      <header className="relative z-20 flex h-16 shrink-0 items-center justify-between gap-3 border-b-2 border-[var(--border-strong)] bg-[var(--bg-surface-solid)] px-4">
        {/* 左侧：返回 + 摄像头通道身份 + 原地编辑任务名 */}
        <div className="flex min-w-0 items-center gap-3">
          {onBack && (
            <button
              type="button"
              onClick={onBack}
              title={t('actions.backToTasks', { defaultValue: '返回任务列表' })}
              aria-label={t('actions.backToTasks', { defaultValue: '返回任务列表' })}
              className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[6px] border border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-secondary)] transition-colors hover:border-[var(--accent)] hover:text-[var(--accent)]"
            >
              <ArrowLeft className="h-4 w-4" />
            </button>
          )}

          {/* 摄像头通道信息 */}
          <div className="flex min-w-0 items-center gap-2 text-xs">
            <span className="flex items-center gap-1.5 font-bold text-[var(--text-primary)]">
              <CameraIcon className="h-4 w-4 shrink-0 text-[var(--accent)]" />
              <span className="max-w-[10rem] truncate">{camera.name || camera.cameraId}</span>
            </span>
            <span className="hidden font-mono text-[11px] text-[var(--text-secondary)] lg:inline">
              {camera.lastWidth || 1920}×{camera.lastHeight || 1080}
            </span>
            <span className="hidden font-mono text-[11px] font-semibold text-cyan-400 lg:inline">
              {camera.lastCodec?.toUpperCase() || 'H264'}
            </span>
          </div>

          <span className="hidden h-4 w-px bg-[var(--border)] sm:inline" />

          {/* 任务名称：支持原地点击快速改名 */}
          <div className="hidden min-w-0 items-center gap-1.5 text-xs sm:flex">
            <span className="shrink-0 text-[var(--text-muted)]">
              {t('taskName', { defaultValue: '任务名称' })}
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
                className="min-w-0 rounded-lg border border-[var(--accent)] bg-[var(--bg-surface)] px-2 py-0.5 text-xs font-semibold text-[var(--text-primary)] ring-1 ring-[var(--accent)] outline-none"
              />
            ) : (
              <button
                type="button"
                onClick={() => setIsEditingTaskName(true)}
                className="group flex min-w-0 items-center gap-1 rounded-lg px-1.5 py-0.5 text-xs font-bold text-[var(--text-primary)] hover:bg-[var(--bg-secondary)]"
                title={t('clickToEditName', { defaultValue: '点击可原地快速修改任务名称' })}
              >
                <span className="max-w-[14rem] truncate">
                  {taskName || `Task-${camera.cameraId}`}
                </span>
                <Pencil className="h-3 w-3 shrink-0 text-[var(--text-muted)] opacity-60 group-hover:text-[var(--accent)] group-hover:opacity-100" />
              </button>
            )}
          </div>
        </div>

        {/* 右侧：码流选择 + 布防总闸 + 保存 */}
        <div className="flex shrink-0 items-center gap-3">
          {saveToast && (
            <span className="hidden font-mono text-xs font-semibold text-emerald-400 xl:inline">
              {saveToast}
            </span>
          )}

          {/* AI 分析码流选择 */}
          <div className="hidden items-center rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] p-0.5 text-xs lg:flex">
            <button
              type="button"
              onClick={() => setStreamMode('main')}
              aria-pressed={streamMode === 'main'}
              className={`rounded-md px-2 py-1 font-medium transition-colors ${
                streamMode === 'main'
                  ? 'bg-cyan-500 font-semibold text-black shadow-2xs'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
              title={t('streamMode.mainDesc', {
                defaultValue: '全高清原图硬件下采样，小目标与远距离识别最清晰，快照零延迟',
              })}
            >
              {t('cardStream.main', { defaultValue: '主码流' })}
            </button>
            <button
              type="button"
              onClick={() => setStreamMode('sub')}
              aria-pressed={streamMode === 'sub'}
              className={`rounded-md px-2 py-1 font-medium transition-colors ${
                streamMode === 'sub'
                  ? 'bg-amber-500 font-semibold text-black shadow-2xs'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
              title={t('streamMode.subDesc', {
                defaultValue: '低码率子流推理，节约 VPU 算力，适合超多路密集布防',
              })}
            >
              {t('cardStream.sub', { defaultValue: '子码流' })}
            </button>
            <button
              type="button"
              onClick={() => setStreamMode('auto')}
              aria-pressed={streamMode === 'auto'}
              className={`rounded-md px-2 py-1 font-medium transition-colors ${
                streamMode === 'auto'
                  ? 'bg-[var(--accent)] font-semibold text-white shadow-2xs'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
              title={t('streamMode.autoDesc', {
                defaultValue: '自动探活子码流，若无子流或不可用则自适应降级主码流',
              })}
            >
              {t('cardStream.auto', { defaultValue: '自动' })}
            </button>
          </div>

          {/* 全局布防总开关 */}
          <div className="flex items-center gap-2">
            <span className="hidden text-xs font-medium text-[var(--text-secondary)] sm:inline">
              {isArmed
                ? t('status.armed', { defaultValue: '已布防' })
                : t('status.disarmed', { defaultValue: '已撤防' })}
            </span>
            <button
              type="button"
              onClick={() => setIsArmed(!isArmed)}
              aria-pressed={isArmed}
              aria-label={t('studio.masterArm', { defaultValue: '通道布防总闸' })}
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
            className="flex items-center gap-1.5 rounded-[6px] bg-[var(--accent)] px-4 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 active:scale-95 disabled:opacity-50"
          >
            <Save className="h-4 w-4" />
            <span>
              {isSaving
                ? t('footer.saving', { defaultValue: '正在保存...' })
                : t('footer.saveTask', { defaultValue: '保存配置' })}
            </span>
          </button>

          {/* 展开/收起右侧配置面板 */}
          {!isPanelOpen && (
            <button
              type="button"
              onClick={() => setIsPanelOpen(true)}
              title={t('studio.expandPanel', { defaultValue: '展开配置面板' })}
              aria-label={t('studio.expandPanel', { defaultValue: '展开配置面板' })}
              className="flex h-8 w-8 items-center justify-center rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] transition-colors hover:border-[var(--accent)] hover:text-[var(--accent)]"
            >
              <Layers className="h-4 w-4" />
            </button>
          )}
        </div>
      </header>

      {/* 主工作区：画板 + 停靠式上下文配置面板 */}
      <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden lg:flex-row">
        {/* 视频主视区 */}
        <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
          <div
            ref={containerRef}
            className="relative flex min-h-0 min-w-0 flex-1 items-center justify-center overflow-hidden bg-[#06080d] p-2 sm:p-3"
          >
            {/* 画布严格按摄像头物理宽高比等比呈现，保证归一化坐标与实景像素一一对应 */}
            <div
              ref={stageRef}
              onMouseDown={handleStageMouseDown}
              onMouseMove={handleStageMouseMove}
              onClick={handleStageClick}
              onDoubleClick={handleStageDoubleClick}
              className={`relative touch-none overflow-hidden rounded-[6px] border border-white/15 bg-black shadow-2xl ${
                DRAW_TOOLS.has(tool) ? 'cursor-crosshair' : ''
              }`}
              style={
                stageSize
                  ? { width: `${stageSize.width}px`, height: `${stageSize.height}px` }
                  : {
                      width: '100%',
                      aspectRatio:
                        camera.lastWidth && camera.lastHeight
                          ? `${camera.lastWidth} / ${camera.lastHeight}`
                          : '16 / 9',
                      maxHeight: '100%',
                    }
              }
            >
              {/* 实时分析源预览播放器 (主码流或子码流)，使用 contain 保证几何标定不失真 */}
              <LivePlayer
                cameraId={camera.cameraId}
                cameraName={camera.name}
                videoCodec={camera.lastCodec}
                stream={effectivePreviewStream}
                fitMode="contain"
                className="pointer-events-none h-full w-full rounded-none border-0"
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
                    const [p1, p2] = rule.points
                    return (
                      <g
                        key={rule.id}
                        onClick={
                          tool === 'select'
                            ? (e) => {
                                e.stopPropagation()
                                setSelectedRuleId(rule.id)
                              }
                            : undefined
                        }
                      >
                        {/* 加宽的透明命中区，避免细线难以点选 */}
                        <line
                          x1={`${p1.x * 100}%`}
                          y1={`${p1.y * 100}%`}
                          x2={`${p2.x * 100}%`}
                          y2={`${p2.y * 100}%`}
                          stroke="transparent"
                          strokeWidth="14"
                          vectorEffect="non-scaling-stroke"
                          className="cursor-pointer"
                        />
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

                  const pointsAttr = rule.points.map((p) => `${p.x * 100},${p.y * 100}`).join(' ')
                  return (
                    <g
                      key={rule.id}
                      onClick={
                        tool === 'select'
                          ? (e) => {
                              e.stopPropagation()
                              setSelectedRuleId(rule.id)
                            }
                          : undefined
                      }
                    >
                      <polygon
                        points={pointsAttr}
                        fill={theme.fill}
                        stroke={isSelected ? theme.selectedStroke : theme.stroke}
                        strokeWidth={isSelected ? '2.5' : '1.5'}
                        vectorEffect="non-scaling-stroke"
                        className={tool === 'select' ? 'cursor-move' : 'cursor-pointer'}
                      />
                    </g>
                  )
                })}

                {/* 绘制中的橡皮线与引导线 */}
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
                        {draftCursor.cursor && (
                          <line
                            x1={`${currentPoints[currentPoints.length - 1].x * 100}%`}
                            y1={`${currentPoints[currentPoints.length - 1].y * 100}%`}
                            x2={`${draftCursor.cursor.x * 100}%`}
                            y2={`${draftCursor.cursor.y * 100}%`}
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

              {/* 顶点手柄层：绝对定位保证圆形手柄不随画幅比例被拉伸 */}
              {tool === 'select' &&
                rules.map((rule, ruleIdx) => {
                  if (!rule.visible || rule.id !== selectedRuleId || rule.points.length < 2) {
                    return null
                  }
                  const theme = getRuleTheme(rule, ruleIdx)
                  return rule.points.map((p, pointIndex) => (
                    <div
                      key={`${rule.id}-${pointIndex}`}
                      onMouseDown={(e) => handleVertexMouseDown(e, rule.id, pointIndex)}
                      className="absolute z-20 h-3 w-3 -translate-x-1/2 -translate-y-1/2 cursor-grab rounded-full border-2 border-black/70 shadow-xs transition-transform hover:scale-125 active:cursor-grabbing"
                      style={{
                        left: `${p.x * 100}%`,
                        top: `${p.y * 100}%`,
                        backgroundColor: theme.selectedStroke,
                      }}
                    />
                  ))
                })}

              {/* 绘制中的顶点与吸附高亮 */}
              {currentPoints.map((p, index) => (
                <div
                  key={`draft-${index}`}
                  className="pointer-events-none absolute z-20 h-2.5 w-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-black/70 bg-white"
                  style={{ left: `${p.x * 100}%`, top: `${p.y * 100}%` }}
                />
              ))}
              {draftCursor.snapped && (
                <div
                  className="pointer-events-none absolute z-20 h-5 w-5 -translate-x-1/2 -translate-y-1/2 animate-pulse rounded-full border-2 border-cyan-400"
                  style={{
                    left: `${draftCursor.snapped.x * 100}%`,
                    top: `${draftCursor.snapped.y * 100}%`,
                  }}
                />
              )}

              {/* 规则随动标注气泡：选中态提供就地快捷操作 */}
              {rules.map((rule, ruleIdx) => {
                if (!rule.visible) return null
                const anchor = getRuleAnchor(rule)
                if (!anchor) return null
                const isSelected = rule.id === selectedRuleId
                const isInteractive = isSelected && tool === 'select'

                return (
                  <div
                    key={`label-${rule.id}`}
                    onMouseDown={(e) => e.stopPropagation()}
                    onClick={(e) => e.stopPropagation()}
                    className={`absolute z-20 -translate-x-1/2 -translate-y-[calc(100%+10px)] ${
                      isInteractive ? '' : 'pointer-events-none'
                    }`}
                    style={{ left: `${anchor.x * 100}%`, top: `${anchor.y * 100}%` }}
                  >
                    <div
                      className={`flex items-center gap-1 rounded-lg border px-1.5 py-0.5 text-[11px] shadow-lg backdrop-blur-xs ${
                        isSelected
                          ? 'border-[var(--accent)]/60 bg-black/85 text-white'
                          : 'border-white/15 bg-black/65 text-white/75'
                      }`}
                    >
                      <span
                        className={`h-1.5 w-1.5 rounded-full ${getRuleTheme(rule, ruleIdx).handleBg}`}
                      />
                      <span className="max-w-[9rem] truncate font-medium">{rule.name}</span>

                      {isSelected && rule.role === 'line' && (
                        <button
                          type="button"
                          onClick={() =>
                            handleUpdateRule(rule.id, {
                              lineDirection:
                                rule.lineDirection === 'both'
                                  ? 'a_to_b'
                                  : rule.lineDirection === 'a_to_b'
                                    ? 'b_to_a'
                                    : 'both',
                            })
                          }
                          title={t('inspector.lineDirection', { defaultValue: '跨线判定方向' })}
                          className="rounded px-1 font-mono text-[10px] text-cyan-300 hover:bg-white/15"
                        >
                          {rule.lineDirection === 'both'
                            ? '⇄'
                            : rule.lineDirection === 'a_to_b'
                              ? 'A→B'
                              : 'B→A'}
                        </button>
                      )}

                      {isSelected && (
                        <button
                          type="button"
                          onClick={() => handleDeleteRule(rule.id)}
                          aria-label={t('inspector.delete', { defaultValue: '删除防区' })}
                          title={t('inspector.delete', { defaultValue: '删除防区' })}
                          className="rounded p-0.5 text-rose-300 hover:bg-rose-500/25"
                        >
                          <X className="h-3 w-3" />
                        </button>
                      )}
                    </div>
                  </div>
                )
              })}

              {/* 画板操作提示 HUD */}
              <div className="pointer-events-none absolute bottom-3 left-1/2 z-20 max-w-[calc(100%-2rem)] -translate-x-1/2">
                <span className="rounded-full border border-white/10 bg-black/70 px-3 py-1 text-center font-mono text-[11px] text-white/70 backdrop-blur-xs">
                  {hudHint}
                </span>
              </div>

              {/* 左侧常驻工具岛 */}
              <StudioToolIsland
                tool={tool}
                onToolChange={handleSelectTool}
                snapEnabled={snapEnabled}
                onToggleSnap={() => setSnapEnabled((prev) => !prev)}
                canUndo={canUndo}
                canRedo={canRedo}
                onUndo={undoRuleEdit}
                onRedo={redoRuleEdit}
                isDrawing={isDrawing}
                onCancelDrawing={handleCancelDrawing}
              />
            </div>
          </div>

          {/* 真实遥测底栏：分辨率/编码来自探活，航迹与门控状态来自后端推送 */}
          <div className="flex shrink-0 flex-wrap items-center justify-between gap-2 border-t border-cyan-500/25 bg-[#0d111a] px-4 py-2 font-mono text-[11px] text-[var(--text-secondary)]">
            <div className="flex items-center gap-3">
              <span className="flex items-center gap-1.5">
                <span className="h-2 w-2 rounded-full bg-emerald-400" />
                <span>
                  {camera.lastWidth || 1920}×{camera.lastHeight || 1080}
                </span>
              </span>
              <span>·</span>
              <span className="font-semibold text-cyan-400">
                {camera.lastCodec?.toUpperCase() || 'H264'}
              </span>
              <span>·</span>
              <span>
                {t('studio.telemetryPreviewStream', { defaultValue: '预览码流' })}:{' '}
                {effectivePreviewStream === 'main'
                  ? t('cardStream.main', { defaultValue: '主码流' })
                  : t('cardStream.sub', { defaultValue: '子码流' })}
              </span>
            </div>
            <div className="flex items-center gap-3">
              <span>
                {t('studio.telemetryTracks', { defaultValue: '活跃航迹' })}:{' '}
                <span className="font-semibold text-[var(--text-primary)]">
                  {telemetry?.activeTracks ?? 0}
                </span>
              </span>
              <span>·</span>
              <span>
                {t('studio.telemetryMotion', { defaultValue: '画面变动' })}:{' '}
                <span className="font-semibold text-[var(--text-primary)]">
                  {telemetry ? `${Math.round(telemetry.motionScore * 100)}%` : '—'}
                </span>
              </span>
              <span>·</span>
              <span className={telemetry?.isMotionGated ? 'text-amber-400' : 'text-emerald-400'}>
                {telemetry?.isMotionGated
                  ? t('studio.telemetryGated', { defaultValue: '门控待机' })
                  : t('studio.telemetryInferring', { defaultValue: '推理中' })}
              </span>
            </div>
          </div>
        </div>

        {/* 停靠式上下文配置面板：全局算力视角 / 单防区属性视角 */}
        {isPanelOpen && (
          <aside className="flex h-[44vh] min-h-0 w-full shrink-0 flex-col overflow-hidden border-t-2 border-[var(--accent)] bg-[var(--bg-primary)] lg:h-auto lg:max-h-none lg:w-[360px] lg:border-t-0 lg:border-l-2 xl:w-[400px]">
            <div className="flex h-11 shrink-0 items-center justify-between border-b border-[var(--border)] px-3.5">
              <span className="flex min-w-0 items-center gap-2 text-xs font-bold text-[var(--text-primary)]">
                {selectedRule ? (
                  <>
                    <span className="h-2.5 w-2.5 shrink-0 rounded-full bg-[var(--accent)]" />
                    <span className="truncate">{selectedRule.name}</span>
                    <span className="shrink-0 rounded-[5px] border border-[var(--border)] bg-[var(--bg-secondary)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-secondary)]">
                      {selectedRule.role.toUpperCase()}
                    </span>
                  </>
                ) : (
                  <>
                    <Layers className="h-4 w-4 text-[var(--accent)]" />
                    <span>{t('studio.panelGlobalMode', { defaultValue: '算力与防区配置' })}</span>
                  </>
                )}
              </span>
              <div className="flex shrink-0 items-center gap-1">
                {selectedRule && (
                  <button
                    type="button"
                    onClick={() => setSelectedRuleId(null)}
                    title={t('inspector.backToOverview', { defaultValue: '返回全局配置' })}
                    aria-label={t('inspector.backToOverview', { defaultValue: '返回全局配置' })}
                    className="flex h-7 w-7 items-center justify-center rounded-lg text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]"
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                )}
                <button
                  type="button"
                  onClick={() => setIsPanelOpen(false)}
                  title={t('studio.collapsePanel', { defaultValue: '收起配置面板' })}
                  aria-label={t('studio.collapsePanel', { defaultValue: '收起配置面板' })}
                  className="flex h-7 w-7 items-center justify-center rounded-lg text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              </div>
            </div>

            <div className="flex-1 space-y-4 overflow-y-auto p-3.5">
              {/* 单防区视角：属性编辑 */}
              {selectedRule && (
                <>
                  <RulePropertiesPanel
                    rule={selectedRule}
                    onUpdateRule={handleUpdateRule}
                    onCloneRule={handleCloneRule}
                    onDeleteRule={handleDeleteRule}
                    activeAlgorithmNames={activeAlgorithmNames}
                  />
                  <div className="h-px bg-[var(--border)]" />
                </>
              )}

              {/* 全局视角：算力机架 */}
              {!selectedRule && (
                <>
                  <AlgorithmRack
                    availableAlgos={availableAlgos}
                    activeInstances={activeInstances}
                    onToggleAlgo={handleToggleAlgo}
                    onOpenParams={handleOpenParams}
                  />
                  <div className="h-px bg-[var(--border)]" />
                </>
              )}

              {/* 空间活动防区列表（两种视角下均可用，便于快速切换） */}
              <ActivityZonesSection
                rules={rules}
                selectedRuleId={selectedRuleId}
                onSelectRule={setSelectedRuleId}
                onToggleRuleVisible={handleToggleRuleVisible}
                onDeleteRule={handleDeleteRule}
                onStartDrawing={handleSelectTool}
                activeTool={tool}
                activeAlgorithmNames={activeAlgorithmNames}
              />

              {/* 运动门控（通道级算力策略，仅在全局视角暴露） */}
              {!selectedRule && (
                <>
                  <div className="h-px bg-[var(--border)]" />
                  <div className="space-y-2.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-surface)] p-3.5">
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-2">
                        <Activity className="h-4 w-4 text-[var(--accent)]" />
                        <span className="text-xs font-bold text-[var(--text-primary)]">
                          {t('studio.motionGateTitle', {
                            defaultValue: '运动检测门控',
                          })}
                        </span>
                      </div>
                      <button
                        type="button"
                        onClick={() => setMotionGateEnabled(!motionGateEnabled)}
                        aria-pressed={motionGateEnabled}
                        aria-label={t('studio.motionGateTitle', {
                          defaultValue: '运动检测门控',
                        })}
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
                          '画面静止无像素变动时跳过 NPU 深度推理，显著降低芯片能耗与总线发热。',
                      })}
                    </p>
                    {motionGateEnabled && (
                      <div className="space-y-1.5 border-t border-[var(--border)] pt-2">
                        <div className="flex items-center justify-between text-[11px]">
                          <span className="text-[var(--text-secondary)]">
                            {t('studio.motionGateSensitivity', { defaultValue: '灵敏度阈值' })}
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
                          aria-label={t('studio.motionGateSensitivity', {
                            defaultValue: '灵敏度阈值',
                          })}
                          onChange={(e) => setMotionGateThreshold(Number(e.target.value))}
                          className="h-1.5 w-full cursor-pointer appearance-none rounded-lg bg-[var(--bg-secondary)] accent-[var(--accent)]"
                        />
                      </div>
                    )}
                  </div>
                </>
              )}
            </div>
          </aside>
        )}
      </div>

      {/* 算法参数独立调优抽屉 */}
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
