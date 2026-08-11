//! Groq speech-to-text client.
//!
//! Two things here matter for perceived latency, and neither is the request
//! itself:
//!
//! - the connection is opened while the user is still speaking ([`prewarm`]),
//!   so the upload does not pay for DNS + TCP + TLS;
//! - the response is asked for as plain text, so there is nothing to parse.

use crate::Config;
use anyhow::{Context, Result};
use std::time::Duration;

const TRANSCRIPTION_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
/// Cheap authenticated endpoint, used only to open a pooled connection.
const WARMUP_URL: &str = "https://api.groq.com/openai/v1/models";

/// Whisper caps the prompt hint at 224 tokens.
const MAX_PROMPT_TOKENS: usize = 224;
/// Rough token estimate used to stay under that cap.
const CHARS_PER_TOKEN: usize = 4;

/// Builds the shared HTTP agent. Cloning it shares the connection pool, which
/// is the entire point — a warmed connection has to be reachable from the
/// thread that later posts the audio.
pub fn build_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout(Duration::from_secs(60))
        .build()
}

/// Opens a connection to the API in the background so it is already in the
/// pool when the clip is ready. Best effort: failures are silent because the
/// real request reports them properly anyway.
pub fn prewarm(agent: &ureq::Agent, api_key: &str) {
    if api_key.is_empty() {
        return;
    }
    let agent = agent.clone();
    let authorization = format!("Bearer {api_key}");
    std::thread::spawn(move || {
        let _ = agent
            .get(WARMUP_URL)
            .set("Authorization", &authorization)
            .call();
    });
}

/// Builds the vocabulary hint sent alongside the audio. Empty when there is
/// no dictionary, in which case the field is omitted entirely.
fn transcription_prompt(dictionary: &[String]) -> String {
    if dictionary.is_empty() {
        return String::new();
    }

    const BASE: &str = "Dictionary entries include: ";
    let budget =
        MAX_PROMPT_TOKENS.saturating_sub(BASE.len().div_ceil(CHARS_PER_TOKEN)) * CHARS_PER_TOKEN;

    let mut vocabulary = dictionary.join(", ");
    if vocabulary.len() > budget {
        // Cut on a char boundary, then back off to the last complete entry.
        let mut cut = budget;
        while cut > 0 && !vocabulary.is_char_boundary(cut) {
            cut -= 1;
        }
        vocabulary.truncate(cut);
        if let Some(idx) = vocabulary.rfind(',') {
            vocabulary.truncate(idx);
        }
    }
    if vocabulary.trim().is_empty() {
        return String::new();
    }
    format!("{BASE}{vocabulary}. ")
}

/// A boundary that cannot collide with the WAV payload. Uniqueness only has
/// to hold within one request, so the clock is enough and saves a uuid dep.
fn multipart_boundary() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("----ito{nanos:x}")
}

/// Assembles the `multipart/form-data` upload: the request fields followed by
/// the WAV itself.
fn build_multipart_body(config: &Config, wav: &[u8], boundary: &str) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::with_capacity(wav.len() + 1024);

    let text_part = |name: &str, value: &str, body: &mut Vec<u8>| {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    };
    text_part("model", &config.model, &mut body);
    // Plain text back: nothing to deserialize, and no JSON escaping to undo.
    text_part("response_format", "text", &mut body);
    text_part("temperature", "0", &mut body);
    if let Some(language) = config.explicit_language() {
        text_part("language", language, &mut body);
    }
    let prompt = transcription_prompt(&config.dictionary);
    if !prompt.is_empty() {
        text_part("prompt", &prompt, &mut body);
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
    body
}

/// Posts one WAV clip and returns the transcript.
pub fn transcribe_wav(agent: &ureq::Agent, config: &Config, wav: &[u8]) -> Result<String> {
    anyhow::ensure!(
        !config.groq_api_key.is_empty(),
        "No groq_api_key configured"
    );

    let boundary = multipart_boundary();
    let body = build_multipart_body(config, wav, &boundary);

    let response = agent
        .post(TRANSCRIPTION_URL)
        .set("Authorization", &format!("Bearer {}", config.groq_api_key))
        .set(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .send_bytes(&body)
        .map_err(describe_error)?;

    let text = response
        .into_string()
        .context("Groq response was not readable text")?;
    Ok(text.trim().to_string())
}

/// Turns a ureq failure into something worth showing in a tooltip. The useful
/// part of an HTTP error is Groq's own message in the body, not the status.
fn describe_error(error: ureq::Error) -> anyhow::Error {
    match error {
        ureq::Error::Status(code, response) => {
            let body = response.into_string().unwrap_or_default();
            let detail = body.trim();
            if detail.is_empty() {
                anyhow::anyhow!("Groq returned HTTP {code}")
            } else {
                anyhow::anyhow!("Groq returned HTTP {code}: {detail}")
            }
        }
        ureq::Error::Transport(transport) => {
            anyhow::anyhow!("Could not reach Groq: {transport}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_dictionary_means_no_prompt() {
        assert!(transcription_prompt(&[]).is_empty());
    }

    #[test]
    fn dictionary_becomes_a_prompt() {
        let dictionary = vec!["Damjan".to_string(), "Zagreb".to_string()];
        assert_eq!(
            transcription_prompt(&dictionary),
            "Dictionary entries include: Damjan, Zagreb. "
        );
    }

    #[test]
    fn long_dictionary_is_truncated_on_an_entry_boundary() {
        let dictionary: Vec<String> = (0..500).map(|i| format!("entry{i}")).collect();
        let prompt = transcription_prompt(&dictionary);
        assert!(prompt.len() < MAX_PROMPT_TOKENS * CHARS_PER_TOKEN);
        // Truncation must not leave a half-written entry behind.
        assert!(prompt.ends_with(". "));
        let entries = prompt.trim_end_matches(". ");
        let last = entries.rsplit(", ").next().unwrap();
        assert!(
            dictionary.iter().any(|e| e == last),
            "last entry {last:?} was cut mid-word"
        );
    }

    #[test]
    fn multipart_boundaries_differ_between_requests() {
        assert_ne!(multipart_boundary(), multipart_boundary());
    }

    /// Splits a multipart body into its parts, so the structure can be checked
    /// without depending on the order fields happen to be written in.
    fn parts_of(body: &[u8], boundary: &str) -> Vec<String> {
        String::from_utf8_lossy(body)
            .split(&format!("--{boundary}"))
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty() && part != "--")
            .collect()
    }

    #[test]
    fn multipart_body_carries_the_fields_and_the_audio() {
        let config = Config::default();
        let wav = crate::audio::to_wav(&[1i16, -1, 1, -1]);
        let body = build_multipart_body(&config, &wav, "BOUND");

        // The WAV must survive byte for byte, headers and all.
        let start = body
            .windows(4)
            .position(|w| w == b"RIFF")
            .expect("no WAV in the body");
        assert_eq!(&body[start..start + wav.len()], &wav[..]);

        let parts = parts_of(&body, "BOUND");
        assert!(parts
            .iter()
            .any(|p| p.contains("name=\"model\"") && p.contains("whisper-large-v3-turbo")));
        assert!(parts
            .iter()
            .any(|p| p.contains("name=\"response_format\"") && p.ends_with("text")));
        assert!(parts
            .iter()
            .any(|p| p.contains("name=\"file\"") && p.contains("filename=\"audio.wav\"")));

        // Every multipart body has to finish with the closing delimiter.
        assert!(body.ends_with(b"--BOUND--\r\n"));
    }

    #[test]
    fn optional_fields_are_omitted_rather_than_sent_empty() {
        let mut config = Config::default();
        let body = build_multipart_body(&config, b"wav", "BOUND");
        let rendered = String::from_utf8_lossy(&body).to_string();
        assert!(
            !rendered.contains("name=\"language\""),
            "auto must not pin a language"
        );
        assert!(
            !rendered.contains("name=\"prompt\""),
            "an empty dictionary must not send a prompt"
        );

        config.language = "hr".to_string();
        config.dictionary = vec!["Zagreb".to_string()];
        let parts = parts_of(&build_multipart_body(&config, b"wav", "BOUND"), "BOUND");
        assert!(parts
            .iter()
            .any(|p| p.contains("name=\"language\"") && p.ends_with("hr")));
        assert!(parts
            .iter()
            .any(|p| p.contains("name=\"prompt\"") && p.contains("Zagreb")));
    }
}
