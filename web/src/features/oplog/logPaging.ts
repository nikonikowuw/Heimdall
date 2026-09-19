/**
 * 日志控制台分页档位。
 * 服务端上限：操作审计 100（MAX_LIMIT），运维事件 200；两处取交集后只暴露公共档位。
 */
export const LOG_PAGE_SIZE_OPTIONS = [20, 50, 100] as const

export const DEFAULT_LOG_PAGE_SIZE = 20
