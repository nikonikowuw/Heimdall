import { describe, expect, it } from 'vitest'
import { validateAuthInput } from './validation'

const loginInput = {
  username: 'operator',
  password: 'secret',
  confirmPassword: '',
  setupRequired: false,
}

describe('validateAuthInput', () => {
  it('登录模式只要求账号与密码', () => {
    expect(validateAuthInput(loginInput)).toBeNull()
    // 登录模式不校验确认密码与密码长度：老账号的短密码必须仍可登录
    expect(validateAuthInput({ ...loginInput, password: '1', confirmPassword: '2' })).toBeNull()
  })

  it('账号与密码的错误存在优先级，先账号后密码', () => {
    expect(validateAuthInput({ ...loginInput, username: '   ', password: '' })).toEqual({
      field: 'username',
      messageKey: 'usernameRequired',
    })
    expect(validateAuthInput({ ...loginInput, password: '' })).toEqual({
      field: 'password',
      messageKey: 'passwordRequired',
    })
  })

  it('账号比较忽略首尾空白', () => {
    expect(validateAuthInput({ ...loginInput, username: '  operator  ' })).toBeNull()
  })

  it('开箱初始化校验密码长度、确认输入与一致性', () => {
    const setup = { ...loginInput, setupRequired: true }

    expect(validateAuthInput({ ...setup, password: '12345' })).toEqual({
      field: 'password',
      messageKey: 'passwordLengthError',
    })
    expect(validateAuthInput({ ...setup, password: '123456', confirmPassword: '' })).toEqual({
      field: 'confirmPassword',
      messageKey: 'confirmPasswordRequired',
    })
    expect(validateAuthInput({ ...setup, password: '123456', confirmPassword: '123457' })).toEqual({
      field: 'confirmPassword',
      messageKey: 'passwordMismatch',
    })
    // 恰好 6 位视为合法下限
    expect(
      validateAuthInput({ ...setup, password: '123456', confirmPassword: '123456' }),
    ).toBeNull()
  })
})
