import { describe, expect, it } from 'vitest'
import { ALGO_PACKAGE_RECOMMENDED_MAX_BYTES, validateAlgoPackage } from './uploadValidation'

describe('validateAlgoPackage', () => {
  it('接受后端沙箱支持的全部归档格式', () => {
    for (const name of ['model.tar.gz', 'model.tgz', 'model.tar', 'model.zip']) {
      expect(validateAlgoPackage(name, 1024)).toBeNull()
    }
  })

  it('大小写与首尾空白不敏感', () => {
    expect(validateAlgoPackage('  Model.TAR.GZ ', 1024)).toBeNull()
  })

  it('拒绝非白名单格式（拖拽投递会绕过 accept 属性）', () => {
    expect(validateAlgoPackage('model.7z', 1024)).toBe('unsupportedFormat')
    expect(validateAlgoPackage('model.tar.gz.exe', 1024)).toBe('unsupportedFormat')
    expect(validateAlgoPackage('model', 1024)).toBe('unsupportedFormat')
    expect(validateAlgoPackage('', 1024)).toBe('unsupportedFormat')
  })

  it('拒绝空文件', () => {
    expect(validateAlgoPackage('model.zip', 0)).toBe('emptyFile')
  })

  it('超过推荐体积上限时提示', () => {
    expect(validateAlgoPackage('model.zip', ALGO_PACKAGE_RECOMMENDED_MAX_BYTES + 1)).toBe(
      'tooLarge',
    )
    expect(validateAlgoPackage('model.zip', ALGO_PACKAGE_RECOMMENDED_MAX_BYTES)).toBeNull()
  })

  it('格式优先于体积判定，便于给出更准确的原因', () => {
    expect(validateAlgoPackage('model.txt', ALGO_PACKAGE_RECOMMENDED_MAX_BYTES + 1)).toBe(
      'unsupportedFormat',
    )
  })
})
