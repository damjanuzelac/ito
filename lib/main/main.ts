import './env'
import { app, protocol, BrowserWindow } from 'electron'
import { electronApp, optimizer } from '@electron-toolkit/utils'
import {
  createAppWindow,
  createPillWindow,
  mainWindow,
  registerResourcesProtocol,
  startPillPositioner,
} from './app'
import { initializeLogging } from './logger'
import { registerIPC } from '../window/ipcEvents'
import { registerDevIPC } from '../window/ipcDev'
import { initializeDatabase } from './sqlite/db'
import { startKeyListener } from '../media/keyboard'
import { preventAppNap } from './appNap'
import { checkAccessibilityPermission } from '../utils/crossPlatform'
import { initializeStore } from './store'
import { selectedTextReaderService } from '../media/selected-text-reader'
import { macOSAccessibilityContextProvider } from '../media/macOSAccessibilityContextProvider'
import { voiceInputService } from './voiceInputService'
import { initializeMicrophoneSelection } from '../media/microphoneSetUp'
import { createAppTray } from './tray'
import { teardown } from './teardown'
import { ITO_ENV } from './env'

protocol.registerSchemesAsPrivileged([
  {
    scheme: 'res',
    privileges: { standard: true, secure: true, supportFetchAPI: true },
  },
])

// Ensure only a single instance of the app runs at a time
const gotTheLock = app.requestSingleInstanceLock()
if (!gotTheLock) {
  app.quit()
} else {
  app.on('second-instance', () => {
    // Someone tried to run a second instance, focus our window instead
    const existingWindow = BrowserWindow.getAllWindows().find(
      win => !win.isDestroyed(),
    )
    if (existingWindow) {
      if (existingWindow.isMinimized()) existingWindow.restore()
      existingWindow.focus()
    }
  })
}

// This method will be called when Electron has finished
// initialization and is ready to create browser windows.
// Some APIs can only be used after this event occurs.
app.whenReady().then(async () => {
  // Initialize the database BEFORE logging so KV writes have a schema
  try {
    await initializeDatabase()
  } catch (error) {
    console.error('Failed to initialize database, quitting app.', error)
    return
  }

  // Initialize KV-backed store and run migrations before anything reads/writes
  try {
    await initializeStore()
  } catch (err) {
    console.error('Failed to initialize main store, quitting app.', err)
    return
  }

  // Initialize logging after DB + store so batched log persistence can write
  initializeLogging()

  // Prevent app nap
  preventAppNap()

  // Register the handler for the 'res' protocol now that the app is ready.
  const appId = ITO_ENV === 'prod' ? 'ai.ito.ito' : `ai.ito.ito-${ITO_ENV}`
  registerResourcesProtocol()
  electronApp.setAppUserModelId(appId)

  // IMPORTANT: Register IPC handlers BEFORE creating windows
  // This prevents the renderer from making IPC calls before handlers are ready
  registerIPC()

  if (!app.isPackaged) {
    registerDevIPC()
  }

  // Create windows
  createAppWindow()
  createPillWindow()
  startPillPositioner()

  if (checkAccessibilityPermission(false)) {
    console.log('Accessibility permissions found, starting key listener.')
    startKeyListener()
  }

  console.log('Microphone access granted, starting audio recorder.')
  voiceInputService.setUpAudioRecorderListeners()

  console.log('Starting selected text reader service.')
  selectedTextReaderService.initialize()

  // Initialize cursor context provider (macOS only for now)
  if (process.platform === 'darwin') {
    console.log('Starting cursor context provider.')
    macOSAccessibilityContextProvider.initialize()
  }

  // Initialize microphone selection to prefer built-in microphone
  await initializeMicrophoneSelection()

  // Create system tray after audio recorder is initialized and devices are available
  await createAppTray()

  app.on('activate', function () {
    if (mainWindow === null) {
      createAppWindow()
    }
  })

  app.on('before-quit', () => {
    console.log('App is quitting, cleaning up resources...')
    teardown()
  })

  app.on('browser-window-created', (_, window) => {
    optimizer.watchWindowShortcuts(window)
  })
})

app.on('window-all-closed', () => {
  // We want the app to stay alive
})
