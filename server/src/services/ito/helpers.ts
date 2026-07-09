import { ItoContext } from './types.js'
import { ITO_MODE_PROMPT } from './constants.js'
import { ItoMode } from '../../generated/ito_pb.js'
import {
  END_APP_NAME_MARKER,
  END_CONTEXT_MARKER,
  END_USER_COMMAND_MARKER,
  END_WINDOW_TITLE_MARKER,
  START_APP_NAME_MARKER,
  START_CONTEXT_MARKER,
  START_USER_COMMAND_MARKER,
  START_WINDOW_TITLE_MARKER,
} from '../../constants/markers.js'

export function createUserPromptWithContext(
  transcript: string,
  context?: ItoContext,
): string {
  let contextPrompt = ''
  if (context) {
    if (context.windowTitle) {
      contextPrompt += `\n${START_WINDOW_TITLE_MARKER}\n${context.windowTitle}\n${END_WINDOW_TITLE_MARKER}`
    }
    if (context.appName) {
      contextPrompt += `\n${START_APP_NAME_MARKER}\n${context.appName}\n${END_APP_NAME_MARKER}`
    }
  }
  const userPrompt = `
    ${contextPrompt}${context?.contextText ? '\n' : ''}
    ${START_CONTEXT_MARKER}
    ${context?.contextText || ''}
    ${END_CONTEXT_MARKER}
    ${START_USER_COMMAND_MARKER}
    ${transcript}
    ${END_USER_COMMAND_MARKER}
  `
  return userPrompt
}

export function detectItoMode(transcript: string): ItoMode {
  const words = transcript.trim().split(/\s+/)
  const firstFiveWords = words.slice(0, 5).join(' ').toLowerCase()

  return firstFiveWords.includes('hey ito') ? ItoMode.EDIT : ItoMode.TRANSCRIBE
}

export interface ModePromptSettings {
  transcriptionPrompt: string
  editingPrompt: string
}

export function getPromptForMode(
  mode: ItoMode,
  advancedSettings: ModePromptSettings,
): string {
  switch (mode) {
    case ItoMode.EDIT:
      return (
        // TODO: Figure out how to version advanced settings such that we can overwrite user settings when a significant change is made
        // advancedSettingsHeaders.editingPrompt || ITO_MODE_PROMPT[ItoMode.EDIT]
        ITO_MODE_PROMPT[ItoMode.EDIT]
      )
    case ItoMode.TRANSCRIBE:
      return (
        advancedSettings.transcriptionPrompt ||
        ITO_MODE_PROMPT[ItoMode.TRANSCRIBE]
      )
    default:
      return ITO_MODE_PROMPT[ItoMode.TRANSCRIBE]
  }
}
