use audio_recorder::{
    device_input_rate, list_input_devices, start_capture, AudioConfigInfo, AudioSink,
    CaptureSession, TARGET_SAMPLE_RATE,
};
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "command")]
enum Command {
    #[serde(rename = "start")]
    Start { device_name: Option<String> },
    #[serde(rename = "stop")]
    Stop,
    #[serde(rename = "list-devices")]
    ListDevices,
    #[serde(rename = "get-device-config")]
    GetDeviceConfig { device_name: Option<String> },
}

#[derive(Serialize)]
struct DeviceList {
    #[serde(rename = "type")]
    response_type: String,
    devices: Vec<String>,
}

#[derive(Serialize)]
struct AudioConfig {
    #[serde(rename = "type")]
    response_type: String,
    input_sample_rate: u32,
    output_sample_rate: u32,
    channels: u8,
}

const MSG_TYPE_JSON: u8 = 1;
const MSG_TYPE_AUDIO: u8 = 2;

fn write_framed_message(writer: &mut impl Write, msg_type: u8, data: &[u8]) -> io::Result<()> {
    let len = data.len() as u32;
    writer.write_all(&[msg_type])?;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(data)?;
    writer.flush()
}

/// Sink that speaks the framed stdout protocol consumed by the Electron app.
struct StdoutSink {
    stdout: Arc<Mutex<io::Stdout>>,
}

impl StdoutSink {
    fn write_json(&self, value: &serde_json::Value) {
        if let Ok(json_string) = serde_json::to_string(value) {
            let mut writer = self.stdout.lock().unwrap();
            let _ = write_framed_message(&mut *writer, MSG_TYPE_JSON, json_string.as_bytes());
        }
    }
}

impl AudioSink for StdoutSink {
    fn on_config(&self, config: AudioConfigInfo) {
        let cfg = AudioConfig {
            response_type: "audio-config".to_string(),
            input_sample_rate: config.input_sample_rate,
            output_sample_rate: config.output_sample_rate,
            channels: config.channels,
        };
        if let Ok(json_string) = serde_json::to_string(&cfg) {
            let mut writer = self.stdout.lock().unwrap();
            let _ = write_framed_message(&mut *writer, MSG_TYPE_JSON, json_string.as_bytes());
        }
    }

    fn on_chunk(&self, pcm: &[i16]) {
        let mut buffer = Vec::with_capacity(pcm.len() * 2);
        for s in pcm {
            buffer.extend_from_slice(&s.to_le_bytes());
        }
        let mut writer = self.stdout.lock().unwrap();
        if let Err(e) = write_framed_message(&mut *writer, MSG_TYPE_AUDIO, &buffer) {
            eprintln!(
                "[audio-recorder] CRITICAL: Failed to write to stdout: {}",
                e
            );
        }
    }

    fn on_drain_complete(&self) {
        self.write_json(&serde_json::json!({ "type": "drain-complete" }));
    }
}

fn main() {
    let stdout = Arc::new(Mutex::new(io::stdout()));
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<Command>();

    thread::spawn(move || {
        let stdin = io::stdin();
        for l in stdin.lock().lines().map_while(Result::ok) {
            if l.trim().is_empty() {
                continue;
            }
            if let Ok(command) = serde_json::from_str::<Command>(&l) {
                cmd_tx
                    .send(command)
                    .expect("Failed to send command to processor");
            }
        }
    });

    let host = audio_recorder::create_host();
    let sink = Arc::new(StdoutSink {
        stdout: Arc::clone(&stdout),
    });
    let mut active_session: Option<CaptureSession> = None;

    while let Ok(command) = cmd_rx.recv() {
        match command {
            Command::ListDevices => {
                let response = DeviceList {
                    response_type: "device-list".to_string(),
                    devices: list_input_devices(&host),
                };
                if let Ok(json_string) = serde_json::to_string(&response) {
                    let mut writer = stdout.lock().unwrap();
                    let _ =
                        write_framed_message(&mut *writer, MSG_TYPE_JSON, json_string.as_bytes());
                }
            }
            Command::Start { device_name } => {
                if let Some(session) = active_session.take() {
                    session.stop();
                }
                match start_capture(
                    &host,
                    device_name.as_deref(),
                    Arc::clone(&sink) as Arc<dyn AudioSink>,
                ) {
                    Ok(session) => active_session = Some(session),
                    Err(e) => {
                        eprintln!(
                            "[audio-recorder] CRITICAL: Failed to create audio stream: {}",
                            e
                        );
                    }
                }
            }
            Command::Stop => {
                if let Some(session) = active_session.take() {
                    session.stop();
                }
            }
            Command::GetDeviceConfig { device_name } => {
                let cfg = AudioConfig {
                    response_type: "audio-config".to_string(),
                    input_sample_rate: device_input_rate(&host, device_name.as_deref()),
                    output_sample_rate: TARGET_SAMPLE_RATE,
                    channels: 1,
                };
                if let Ok(json_string) = serde_json::to_string(&cfg) {
                    let mut writer = stdout.lock().unwrap();
                    let _ =
                        write_framed_message(&mut *writer, MSG_TYPE_JSON, json_string.as_bytes());
                }
            }
        }
    }
}
