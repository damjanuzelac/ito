/**
 * Local no-op timing collector.
 *
 * The original implementation batched timing reports and shipped them to the
 * server's TimingService for analytics. That telemetry has been removed for
 * the local, single-user build; this stub keeps the call sites in the core
 * dictation path unchanged while doing no reporting.
 */
export enum TimingEventName {
  INTERACTION_ACTIVE = 'interaction_active',
  WINDOW_CONTEXT_GATHER = 'window_context_gather',
  CURSOR_CONTEXT_GATHER = 'cursor_context_gather',
  SELCTED_TEXT_GATHER = 'selected_text_gather',
  SERVER_DICTATION = 'server_dictation',
  SERVER_EDITING = 'server_editing',
  GRAMMAR_SERVICE = 'grammar_service',
  TEXT_WRITER = 'text_writer',
}

class TimingCollector {
  startInteraction(_interactionId?: string): void {}

  clearInteraction(_interactionId?: string): void {}

  finalizeInteraction(_interactionId?: string): void {}

  startTiming(_eventName: TimingEventName, _interactionId?: string): void {}

  endTiming(_eventName: TimingEventName, _interactionId?: string): void {}

  async timeAsync<T>(
    _eventName: TimingEventName,
    fn: () => Promise<T> | T,
    _interactionId?: string,
  ): Promise<T> {
    return await fn()
  }

  shutdown(): void {}
}

export const timingCollector = new TimingCollector()
