//! Whisper ggml model auto-download from Hugging Face on first run.

use anyhow::{anyhow, Context, Result};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const HF_BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// Ensures the ggml model file exists in `models_dir`, downloading it on
/// first run. `progress` is called with (downloaded_bytes, total_bytes).
pub fn ensure_model(
    models_dir: &Path,
    model_file_name: &str,
    progress: &dyn Fn(u64, u64),
) -> Result<PathBuf> {
    let model_path = models_dir.join(model_file_name);
    if model_path.exists() {
        return Ok(model_path);
    }

    std::fs::create_dir_all(models_dir)?;
    let url = format!("{HF_BASE_URL}/{model_file_name}");
    eprintln!("[ito-tray] Downloading Whisper model from {url} ...");

    let agent = build_agent();
    let response = agent
        .get(&url)
        .call()
        .with_context(|| format!("Failed to download {url}"))?;

    let total: u64 = response
        .header("Content-Length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    // Download to a temp file first so a partial download never looks like a
    // valid model.
    let tmp_path = model_path.with_extension("bin.partial");
    let mut file = std::fs::File::create(&tmp_path)?;
    let mut reader = response.into_reader();
    let mut buffer = [0u8; 1024 * 256];
    let mut downloaded: u64 = 0;

    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        file.write_all(&buffer[..n])?;
        downloaded += n as u64;
        progress(downloaded, total);
    }
    file.flush()?;
    drop(file);

    if total > 0 && downloaded != total {
        std::fs::remove_file(&tmp_path).ok();
        return Err(anyhow!(
            "Model download incomplete: {downloaded} of {total} bytes"
        ));
    }

    std::fs::rename(&tmp_path, &model_path)?;
    eprintln!(
        "[ito-tray] Model saved to {} ({downloaded} bytes)",
        model_path.display()
    );
    Ok(model_path)
}

/// Builds an HTTP agent, honoring HTTPS_PROXY/https_proxy if set.
fn build_agent() -> ureq::Agent {
    let proxy_env = std::env::var("HTTPS_PROXY")
        .or_else(|_| std::env::var("https_proxy"))
        .ok();

    let mut builder = ureq::AgentBuilder::new();
    if let Some(proxy_url) = proxy_env {
        match ureq::Proxy::new(&proxy_url) {
            Ok(proxy) => builder = builder.proxy(proxy),
            Err(e) => eprintln!("[ito-tray] Ignoring invalid HTTPS_PROXY: {e}"),
        }
    }
    builder.build()
}
