export interface AuthFieldError {
  field: 'username' | 'password' | 'confirmPassword'
  messageKey:
    | 'usernameRequired'
    | 'passwordRequired'
    | 'passwordLengthError'
    | 'confirmPasswordRequired'
    | 'passwordMismatch'
}

/**
 * 校验登录或开箱初始化表单输入，返回首个出错字段及对应国际化 key。
 *
 * 判定顺序即交互语义：先账号后密码，开箱模式先长度后确认；调用方据此聚焦首个出错字段。
 */
export function validateAuthInput(input: {
  username: string
  password: string
  confirmPassword: string
  setupRequired: boolean
}): AuthFieldError | null {
  if (!input.username.trim()) {
    return { field: 'username', messageKey: 'usernameRequired' }
  }
  if (!input.password) {
    return { field: 'password', messageKey: 'passwordRequired' }
  }
  if (input.setupRequired) {
    if (input.password.length < 6) {
      return { field: 'password', messageKey: 'passwordLengthError' }
    }
    if (!input.confirmPassword) {
      return { field: 'confirmPassword', messageKey: 'confirmPasswordRequired' }
    }
    if (input.password !== input.confirmPassword) {
      return { field: 'confirmPassword', messageKey: 'passwordMismatch' }
    }
  }
  return null
}
