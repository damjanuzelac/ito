//! Optional Groq LLM call for edit mode, mirroring the server's
//! adjustTranscript (server/src/clients/groqClient.ts).

use crate::prompt::{
    create_user_prompt_with_context, WindowContext, EDITING_PROMPT, EDIT_SYSTEM_PROMPT,
};

const GROQ_CHAT_URL: &str = "https://api.groq.com/openai/v1/chat/completions";
const GROQ_TRANSCRIPTION_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
const LLM_TEMPERATURE: f32 = 0.1;

/// Transcribes a WAV clip through Groq's audio API (OpenAI-compatible
/// multipart upload). `language` of "auto" lets the API detect; `prompt`
/// carries the vocabulary hint. Returns the transcript text.
pub fn transcribe_wav(
    api_key: &str,
    model: &str,
    wav: &[u8],
    language: &str,
    prompt: &str,
) -> Result<String, String> {
    let boundary = format!("ito-tray-{}", uuid::Uuid::new_v4().simple());
    let mut body: Vec<u8> = Vec::with_capacity(wav.len() + 1024);

    let text_part = |name: &str, value: &str, body: &mut Vec<u8>| {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    };
    text_part("model", model, &mut body);
    text_part("response_format", "json", &mut body);
    text_part("temperature", "0", &mut body);
    if language != "auto" && !language.is_empty() {
        text_part("language", language, &mut body);
    }
    if !prompt.is_empty() {
        text_part("prompt", prompt, &mut body);
    }
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; \
             filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let response = ureq::post(GROQ_TRANSCRIPTION_URL)
        .set("Authorization", &format!("Bearer {api_key}"))
        .set(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .send_bytes(&body)
        .map_err(|e| format!("Groq transcription request failed: {e}"))?;

    let json: serde_json::Value = response
        .into_json()
        .map_err(|e| format!("Groq transcription response invalid: {e}"))?;
    json["text"]
        .as_str()
        .map(|t| t.trim().to_string())
        .ok_or_else(|| "Groq transcription response had no text field".to_string())
}

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
