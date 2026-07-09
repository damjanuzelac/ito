import * as dotenv from 'dotenv'
import { createTranscriptionPrompt } from '../prompts/transcription.js'
import {
  ClientNoSpeechError,
  ClientAudioTooShortError,
  ClientApiError,
  ClientError,
} from './errors.js'
import { ClientProvider } from './providers.js'
import { LlmProvider } from './llmProvider.js'
import { TranscriptionOptions } from './asrConfig.js'
import { IntentTranscriptionOptions } from './intentTranscriptionConfig.js'
import { DEFAULT_ADVANCED_SETTINGS } from '../constants/generated-defaults.js'
import { itoVocabulary } from './groqClient.js'

dotenv.config()

const DEFAULT_BASE_URL = 'http://localhost:8000'
const DEFAULT_MODEL = 'Systran/faster-whisper-small'

/**
 * ASR client for a local, OpenAI-compatible Whisper server
 * (e.g. speaches / faster-whisper-server) exposing POST /v1/audio/transcriptions.
 *
 * The model is configured server-side via LOCAL_WHISPER_MODEL because local
 * servers use Hugging Face model ids (e.g. Systran/faster-whisper-small)
 * rather than the cloud model names clients send.
 */
class LocalWhisperClient implements LlmProvider {
  private readonly _baseUrl: string
  private readonly _model: string

  constructor(baseUrl: string, model: string) {
    this._baseUrl = baseUrl.replace(/\/+$/, '')
    this._model = model
  }

  public get isAvailable(): boolean {
    return true
  }

  public async adjustTranscript(
    _userPrompt: string,
    _options?: IntentTranscriptionOptions,
  ): Promise<string> {
    throw new Error(
      'Transcript adjustment is not supported by the local Whisper provider.',
    )
  }

  public async transcribeAudio(
    audioBuffer: Buffer,
    options?: TranscriptionOptions,
  ): Promise<string> {
    const fileType = options?.fileType || 'wav'
    const vocabulary = options?.vocabulary
    const noSpeechThreshold =
      options?.noSpeechThreshold ?? DEFAULT_ADVANCED_SETTINGS.noSpeechThreshold

    const fullVocabulary = [...itoVocabulary, ...(vocabulary || [])]
    const transcriptionPrompt = createTranscriptionPrompt(fullVocabulary)

    const formData = new FormData()
    formData.append(
      'file',
      new Blob([new Uint8Array(audioBuffer)]),
      `audio.${fileType}`,
    )
    formData.append('model', this._model)
    formData.append('prompt', transcriptionPrompt)
    formData.append('response_format', 'verbose_json')

    try {
      console.log(
        `Transcribing ${audioBuffer.length} bytes of audio using local model ${this._model} at ${this._baseUrl}...`,
      )

      const response = await fetch(`${this._baseUrl}/v1/audio/transcriptions`, {
        method: 'POST',
        body: formData,
      })

      if (!response.ok) {
        const errorBody = await response.text().catch(() => '')
        if (errorBody.includes('Audio file is too short')) {
          throw new ClientAudioTooShortError(ClientProvider.LOCAL)
        }
        throw new ClientApiError(
          `Local Whisper server responded with ${response.status}: ${errorBody}`,
          ClientProvider.LOCAL,
          undefined,
          response.status,
        )
      }

      const transcription = (await response.json()) as {
        text?: string
        segments?: Array<{ no_speech_prob?: number }>
      }

      // Not every local server reports no_speech_prob; only gate on it when present.
      const first = transcription.segments?.[0]
      if (
        typeof first?.no_speech_prob === 'number' &&
        first.no_speech_prob > noSpeechThreshold
      ) {
        console.log('No speech probability:', first.no_speech_prob)
        throw new ClientNoSpeechError(
          ClientProvider.LOCAL,
          first.no_speech_prob,
        )
      }

      return (transcription.text ?? '').trim()
    } catch (error: any) {
      if (error instanceof ClientError) {
        throw error
      }

      console.error('An error occurred during local transcription:', error)
      throw new ClientApiError(
        error.message || 'An unknown error occurred',
        ClientProvider.LOCAL,
        error,
        error.status || error.statusCode,
      )
    }
  }
}

export const localWhisperClient = new LocalWhisperClient(
  process.env.LOCAL_WHISPER_BASE_URL || DEFAULT_BASE_URL,
  process.env.LOCAL_WHISPER_MODEL || DEFAULT_MODEL,
)
