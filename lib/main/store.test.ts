import { describe, test, expect, beforeEach, mock } from 'bun:test'

// Mock crypto module, preserving Node's built-ins used by dependencies like `uuid`
const realCrypto = require('node:crypto')
const mockCryptoPartial = {
  ...realCrypto,
  randomBytes: mock((_size: number) => ({
    toString: mock((encoding: string) => {
      if (encoding === 'base64url') return 'mock-base64url-string'
      if (encoding === 'hex') return 'mock-hex-string'
      return 'mock-string'
    }),
  })),
  createHash: mock(() => ({
    update: mock(() => ({
      digest: mock(() => 'mock-hash-digest'),
    })),
  })),
  randomUUID: mock(() => 'mock-uuid-123'),
}

mock.module('crypto', () => ({
  default: mockCryptoPartial,
  ...mockCryptoPartial,
}))

// Mock console to avoid noise
beforeEach(() => {
  console.log = mock()
  console.error = mock()
})

describe('KV-backed Store', () => {
  beforeEach(() => {
    // Clear module cache for fresh imports
    delete require.cache[require.resolve('./store')]
  })

  test('should expose default values on first load', async () => {
    const { default: store } = await import('./store')
    const settings = store.get('settings')
    expect(settings.shareAnalytics).toBe(true)
    expect(settings.launchAtLogin).toBe(true)
    expect(settings.isShortcutGloballyEnabled).toBe(false)
    const main = store.get('main')
    expect(main.navExpanded).toBe(true)
  })

  test('dot-path set should update nested value', async () => {
    const { default: store } = await import('./store')
    store.set('settings.launchAtLogin', false)
    const settings = store.get('settings')
    expect(settings.launchAtLogin).toBe(false)
  })

  test('delete should clear top-level key', async () => {
    const { default: store } = await import('./store')
    store.set('main', { navExpanded: false })
    expect(store.get('main').navExpanded).toBe(false)
    store.delete('main')
    expect(store.get('main')).toBeUndefined()
  })
})

describe('Auth helpers', () => {
  test('getCurrentUserId always returns the fixed self-hosted id', async () => {
    const { getCurrentUserId } = await import('./store')
    expect(getCurrentUserId()).toBe('self-hosted')
  })
})
