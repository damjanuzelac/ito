//! Optional Groq LLM call for edit mode, mirroring the server's
//! adjustTranscript (server/src/clients/groqClient.ts).

use crate::prompt::{
    create_user_prompt_with_context, WindowContext, EDITING_PROMPT, EDIT_SYSTEM_PROMPT,
};

const GROQ_CHAT_URL: &str = "https://api.groq.com/openai/v1/chat/completions";
const LLM_TEMPERATURE: f32 = 0.1;

/// Rewrites the transcript as an edit-mode command result. Returns the
/// original transcript on any error (same behavior as the server).
pub fn adjust_transcript(
    api_key: &str,
    model: &str,
    transcript: &str,
    context: &WindowContext,
) -> String {
    let user_prompt = format!(
        "{EDITING_PROMPT}\n{}",
        create_user_prompt_with_context(transcript, context)
    );

    let body = serde_json::json!({
        "messages": [
            { "role": "system", "content": EDIT_SYSTEM_PROMPT },
            { "role": "user", "content": user_prompt },
        ],
        "model": model,
        "temperature": LLM_TEMPERATURE,
    });

    let response = ureq::post(GROQ_CHAT_URL)
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Content-Type", "application/json")
        .send_json(body);

    match response {
        Ok(res) => match res.into_json::<serde_json::Value>() {
            Ok(json) => {
                let content = json["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if content.is_empty() {
                    // A space enables emptying the document (server behavior)
                    " ".to_string()
                } else {
                    content
                }
            }
            Err(e) => {
                eprintln!("[ito-tray] Failed to parse Groq response: {e}");
                transcript.to_string()
            }
        },
        Err(e) => {
            eprintln!("[ito-tray] Groq request failed: {e}");
            transcript.to_string()
        }
    }
}
