//! Prompt building and mode detection ported from the Ito server
//! (server/src/prompts/transcription.ts, services/ito/helpers.ts and
//! shared-constants.js).

/// Vocabulary that is always fed to the ASR prompt.
pub const ITO_VOCABULARY: [&str; 2] = ["Ito", "Hey Ito"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItoMode {
    Transcribe,
    Edit,
}

/// Rough token estimate (1 token ≈ 4 characters).
fn estimate_token_count(text: &str) -> usize {
    text.len().div_ceil(4)
}

/// Creates a transcription prompt that stays within the 224 token limit.
/// Mirrors createTranscriptionPrompt from the server.
pub fn create_transcription_prompt(vocabulary: &[String]) -> String {
    const MAX_TOKENS: usize = 224;

    if vocabulary.is_empty() {
        return String::new();
    }

    let base_prompt = "Dictionary entries include: ";
    let base_tokens = estimate_token_count(&format!("{base_prompt}. "));
    let available_tokens_for_vocab = MAX_TOKENS.saturating_sub(base_tokens);

    let mut vocab_string = vocabulary.join(", ");

    if estimate_token_count(&vocab_string) > available_tokens_for_vocab {
        let max_vocab_length = (available_tokens_for_vocab * 4).saturating_sub(10); // Leave buffer
        vocab_string.truncate(
            vocab_string
                .char_indices()
                .map(|(i, _)| i)
                .take_while(|&i| i <= max_vocab_length)
                .last()
                .unwrap_or(0),
        );
        // Remove incomplete last term
        if let Some(idx) = vocab_string.rfind(',') {
            vocab_string.truncate(idx);
        }
    }

    if vocab_string.trim().is_empty() {
        return String::new();
    }

    format!("{base_prompt}{vocab_string}. ")
}

/// Builds the full vocabulary (fixed Ito terms + user dictionary).
pub fn full_vocabulary(dictionary: &[String]) -> Vec<String> {
    let mut vocab: Vec<String> = ITO_VOCABULARY.iter().map(|s| s.to_string()).collect();
    vocab.extend(dictionary.iter().cloned());
    vocab
}

/// Detects edit mode: the first five words (lowercased) contain "hey ito".
pub fn detect_ito_mode(transcript: &str) -> ItoMode {
    let first_five_words = transcript
        .split_whitespace()
        .take(5)
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    if first_five_words.contains("hey ito") {
        ItoMode::Edit
    } else {
        ItoMode::Transcribe
    }
}

// Markers from server/src/constants/markers.ts
const START_WINDOW_TITLE_MARKER: &str = "{START_WINDOW_TITLE_MARKER}";
const END_WINDOW_TITLE_MARKER: &str = "{END_WINDOW_TITLE_MARKER}";
const START_APP_NAME_MARKER: &str = "{START_APP_NAME_MARKER}";
const END_APP_NAME_MARKER: &str = "{END_APP_NAME_MARKER}";
const START_USER_COMMAND_MARKER: &str = "{START_USER_COMMAND_MARKER}";
const END_USER_COMMAND_MARKER: &str = "{END_USER_COMMAND_MARKER}";
const START_CONTEXT_MARKER: &str = "{START_CONTEXT_MARKER}";
const END_CONTEXT_MARKER: &str = "{END_CONTEXT_MARKER}";

/// Window/application context captured at dictation time.
#[derive(Debug, Clone, Default)]
pub struct WindowContext {
    pub window_title: String,
    pub app_name: String,
    pub context_text: String,
}

/// Wraps the transcript with context markers, mirroring
/// createUserPromptWithContext from the server.
pub fn create_user_prompt_with_context(transcript: &str, context: &WindowContext) -> String {
    let mut context_prompt = String::new();
    if !context.window_title.is_empty() {
        context_prompt.push_str(&format!(
            "\n{START_WINDOW_TITLE_MARKER}\n{}\n{END_WINDOW_TITLE_MARKER}",
            context.window_title
        ));
    }
    if !context.app_name.is_empty() {
        context_prompt.push_str(&format!(
            "\n{START_APP_NAME_MARKER}\n{}\n{END_APP_NAME_MARKER}",
            context.app_name
        ));
    }

    let context_newline = if context.context_text.is_empty() {
        ""
    } else {
        "\n"
    };
    format!(
        "\n    {context_prompt}{context_newline}\n    {START_CONTEXT_MARKER}\n    {}\n    {END_CONTEXT_MARKER}\n    {START_USER_COMMAND_MARKER}\n    {transcript}\n    {END_USER_COMMAND_MARKER}\n  ",
        context.context_text
    )
}

/// System prompt for edit mode (ITO_MODE_SYSTEM_PROMPT[EDIT]).
pub const EDIT_SYSTEM_PROMPT: &str = "You are an AI assistant helping to edit documents.";

/// The default editing prompt (editingPrompt from shared-constants.js).
pub const EDITING_PROMPT: &str = r#" You are a Command-Interpreter assistant. Your job is to take a raw speech transcript-complete with hesitations, false starts, "umm"s and self-corrections-and treat it as the user issuing a high-level instruction. Instead of merely polishing their words, you must:
    1.	Extract the intent: identify the action the user is asking for (e.g. "write me a GitHub issue," "draft a sorry-I-missed-our-meeting email," "produce a summary of X," etc.).
    2.	Ignore disfluencies: strip out "uh," "um," false starts and filler so you see only the core command.
    3.	Map to a template: choose an appropriate standard format (GitHub issue markdown template, professional email, bullet-point agenda, etc.) that matches the intent.
    4.	Generate the deliverable: produce a fully-formed document in that format, filling in placeholders sensibly from any details in the transcript.
    5.	Do not add new intent: if the transcript doesn't specify something (e.g. title, recipients, date), use reasonable defaults (e.g. "Untitled Issue," "To: [Recipient]") or prompt the user for the missing piece.
    6.	Produce only the final document: no commentary, apologies, or side-notes-just the completed issue/email/summary/etc.
    7. Your response MUST contain ONLY the resultant text. DO NOT include:
      - Any markers like [START/END CURRENT NOTES CONTENT]
      - Any explanations, apologies, or additional text
      - Any formatting markers like --- or ```
  "#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_vocabulary_gives_empty_prompt() {
        assert_eq!(create_transcription_prompt(&[]), "");
    }

    #[test]
    fn small_vocabulary_is_joined() {
        let vocab = vec![
            "Ito".to_string(),
            "Hey Ito".to_string(),
            "Zagreb".to_string(),
        ];
        assert_eq!(
            create_transcription_prompt(&vocab),
            "Dictionary entries include: Ito, Hey Ito, Zagreb. "
        );
    }

    #[test]
    fn long_vocabulary_is_truncated_at_term_boundary() {
        let vocab: Vec<String> = (0..300).map(|i| format!("word{i:04}")).collect();
        let prompt = create_transcription_prompt(&vocab);
        assert!(estimate_token_count(&prompt) <= 224);
        // Must not end mid-term: last term before ". " suffix is complete
        let body = prompt
            .trim_end()
            .trim_end_matches('.')
            .strip_prefix("Dictionary entries include: ")
            .unwrap();
        assert!(body
            .split(", ")
            .all(|t| t.starts_with("word") && t.len() == 8));
    }

    #[test]
    fn detects_edit_mode_only_in_first_five_words() {
        assert_eq!(detect_ito_mode("Hey Ito, write an email"), ItoMode::Edit);
        assert_eq!(detect_ito_mode("hey   ito please"), ItoMode::Edit);
        assert_eq!(
            detect_ito_mode("This is a normal dictation"),
            ItoMode::Transcribe
        );
        assert_eq!(
            detect_ito_mode("one two three four five hey ito"),
            ItoMode::Transcribe
        );
    }

    #[test]
    fn user_prompt_wraps_transcript_with_markers() {
        let ctx = WindowContext {
            window_title: "Notepad".into(),
            app_name: "notepad.exe".into(),
            context_text: String::new(),
        };
        let prompt = create_user_prompt_with_context("hey ito write a poem", &ctx);
        assert!(prompt.contains("{START_WINDOW_TITLE_MARKER}\nNotepad\n{END_WINDOW_TITLE_MARKER}"));
        assert!(prompt.contains("{START_APP_NAME_MARKER}\nnotepad.exe\n{END_APP_NAME_MARKER}"));
        assert!(prompt.contains(
            "{START_USER_COMMAND_MARKER}\n    hey ito write a poem\n    {END_USER_COMMAND_MARKER}"
        ));
    }
}
