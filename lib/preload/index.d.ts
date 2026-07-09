import { ElectronAPI } from '@electron-toolkit/preload'
import type api from './api'

interface KeyEvent {
  type: 'keydown' | 'keyup'
  key: string
  timestamp: string
  raw_code: number
}

interface StoreAPI {
  get(key: string): any
  set(property: string, val: any): void
}

interface SelectedTextOptions {
  format?: 'json' | 'text'
  maxLength?: number
}

interface SelectedTextResult {
  success: boolean
  text: string | null
  error: string | null
  length: number
}

interface SelectedTextAPI {
  get: (options?: SelectedTextOptions) => Promise<SelectedTextResult>
  getString: (maxLength?: number) => Promise<string | null>
  hasSelected: () => Promise<boolean>
}

declare global {
  interface Window {
    electron: ElectronAPI & {
      store: StoreAPI
    }
    api: typeof api
  }
}
