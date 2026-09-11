import { type ClassValue, clsx } from 'clsx'
import { twMerge } from 'tailwind-merge'

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

/**
 * 跨安全上下文剪贴板写入工具函数
 *
 * 机制说明：
 * 1. 优先使用现代异步 Clipboard API (navigator.clipboard.writeText)，仅在 HTTPS 或 localhost 安全上下文中可用；
 * 2. 边缘计算与工业网关场景（如通过局域网 HTTP IP 访问）下，window.isSecureContext 为 false 且 navigator.clipboard 为 undefined，
 *    或现代 API 权限受阻时，优雅回退到基于 document.execCommand('copy') 的隐藏 textarea 方案；
 * 3. 元素设置固定定位和防滚动属性，避免视觉抖动或干扰视口。
 */
export async function copyToClipboard(text: string): Promise<boolean> {
  if (!text) {
    return false
  }

  // 1. 尝试现代异步 Clipboard API (必须处于安全上下文且 API 存在)
  if (
    typeof window !== 'undefined' &&
    window.isSecureContext &&
    typeof navigator !== 'undefined' &&
    navigator.clipboard?.writeText
  ) {
    try {
      await navigator.clipboard.writeText(text)
      return true
    } catch {
      // 现代 API 失败时，继续尝试传统回退方案
    }
  }

  // 2. 回退到传统的 document.execCommand('copy') 方案
  if (typeof document === 'undefined') {
    return false
  }

  try {
    const textArea = document.createElement('textarea')
    textArea.value = text

    // 样式防护：确保不在视口中可见，不触发页面滚动，不影响布局
    textArea.style.position = 'fixed'
    textArea.style.top = '-9999px'
    textArea.style.left = '-9999px'
    textArea.style.opacity = '0'
    textArea.style.pointerEvents = 'none'
    textArea.setAttribute('readonly', '')

    document.body.appendChild(textArea)
    textArea.focus({ preventScroll: true })
    textArea.select()
    textArea.setSelectionRange(0, textArea.value.length)

    const successful = document.execCommand('copy')
    document.body.removeChild(textArea)
    return successful
  } catch {
    return false
  }
}
