use std::{
    io::Read,
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};

use crate::config::{SourceConfig, SourceKind};

pub const FRAME_WIDTH: u16 = 192;
pub const FRAME_HEIGHT: u16 = 108;

#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub width: u16,
    pub height: u16,
    pub pixels: Arc<[u8]>,
    pub sequence: u64,
    pub received_at: Instant,
}

#[derive(Debug, Clone)]
pub enum MediaEvent {
    Frame(VideoFrame),
    Status(SourceStatus),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceStatus {
    Connecting,
    Live,
    Reconnecting,
    Failed(String),
    Stopped,
}

#[derive(Debug)]
pub struct ResolvedInput {
    pub input: String,
    pub input_args: Vec<String>,
    pub reconnect: bool,
    pub loop_file: bool,
}

pub struct SourceWorker {
    pub name: String,
    pub receiver: Receiver<MediaEvent>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl SourceWorker {
    pub fn spawn(source: SourceConfig, fps: u8) -> Self {
        let (sender, receiver) = mpsc::sync_channel(2);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let name = source.name.clone();
        let thread = thread::Builder::new()
            .name(format!("source-{}", slug(&name)))
            .spawn(move || worker_loop(source, fps, sender, thread_stop))
            .expect("source worker thread should start");
        Self {
            name,
            receiver,
            stop,
            thread: Some(thread),
        }
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Closing the UI breaks FFmpeg's stdout pipe. We deliberately avoid a
        // blocking join here because a remote socket can take time to unwind.
        self.thread.take();
    }
}

impl Drop for SourceWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn ensure_ffmpeg() -> Result<()> {
    ensure_binary("ffmpeg")
}
pub fn ensure_ffprobe() -> Result<()> {
    ensure_binary("ffprobe")
}

fn ensure_binary(name: &str) -> Result<()> {
    let status = Command::new(name)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match status {
        Ok(status) if status.success() => Ok(()),
        _ => bail!("{name} was not found; install FFmpeg and ensure `{name}` is on PATH"),
    }
}

pub fn resolve_input(input: &str, kind: SourceKind) -> Result<ResolvedInput> {
    let camera = kind == SourceKind::Camera || input.starts_with("camera://");
    if camera {
        let device = input.strip_prefix("camera://").unwrap_or(input);
        if device.is_empty() {
            bail!("camera input needs a device, for example camera://0");
        }
        let (input, input_args) = if cfg!(target_os = "macos") {
            (
                format!("{device}:none"),
                vec![
                    "-f".into(),
                    "avfoundation".into(),
                    "-framerate".into(),
                    "15".into(),
                ],
            )
        } else if cfg!(target_os = "windows") {
            (
                format!("video={device}"),
                vec![
                    "-f".into(),
                    "dshow".into(),
                    "-framerate".into(),
                    "15".into(),
                ],
            )
        } else {
            let device = if device.starts_with('/') {
                device.to_owned()
            } else {
                format!("/dev/video{device}")
            };
            (
                device,
                vec!["-f".into(), "v4l2".into(), "-framerate".into(), "15".into()],
            )
        };
        return Ok(ResolvedInput {
            input,
            input_args,
            reconnect: false,
            loop_file: false,
        });
    }

    let is_file =
        kind == SourceKind::File || (kind == SourceKind::Auto && Path::new(input).exists());
    let is_http = input.starts_with("http://") || input.starts_with("https://");
    let input_args = if input.starts_with("rtsp://") {
        vec![
            "-rtsp_transport".into(),
            "tcp".into(),
            "-rw_timeout".into(),
            "15000000".into(),
        ]
    } else {
        Vec::new()
    };
    Ok(ResolvedInput {
        input: input.to_owned(),
        input_args,
        reconnect: is_http,
        loop_file: is_file,
    })
}

fn worker_loop(
    source: SourceConfig,
    fps: u8,
    sender: SyncSender<MediaEvent>,
    stop: Arc<AtomicBool>,
) {
    let resolved = match resolve_input(&source.input, source.kind) {
        Ok(value) => value,
        Err(error) => {
            let _ = sender.send(MediaEvent::Status(SourceStatus::Failed(error.to_string())));
            return;
        }
    };
    let mut attempt = 0_u32;
    while !stop.load(Ordering::Relaxed) {
        let status = if attempt == 0 {
            SourceStatus::Connecting
        } else {
            SourceStatus::Reconnecting
        };
        if sender.send(MediaEvent::Status(status)).is_err() {
            return;
        }
        match stream_once(&resolved, &source, fps, &sender, &stop) {
            Ok(()) if stop.load(Ordering::Relaxed) => break,
            Ok(()) => {}
            Err(error) => {
                if sender
                    .send(MediaEvent::Status(SourceStatus::Failed(short_error(
                        &error,
                    ))))
                    .is_err()
                {
                    return;
                }
            }
        }
        attempt = attempt.saturating_add(1);
        let delay = Duration::from_secs(u64::from(attempt.min(5)));
        let deadline = Instant::now() + delay;
        while Instant::now() < deadline && !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(100));
        }
    }
    let _ = sender.send(MediaEvent::Status(SourceStatus::Stopped));
}

fn stream_once(
    resolved: &ResolvedInput,
    source: &SourceConfig,
    fps: u8,
    sender: &SyncSender<MediaEvent>,
    stop: &AtomicBool,
) -> Result<()> {
    let mut child = ffmpeg_command(resolved, source, fps)?
        .spawn()
        .context("could not start FFmpeg")?;
    let mut stdout = child
        .stdout
        .take()
        .context("FFmpeg stdout was unavailable")?;
    let frame_size = usize::from(FRAME_WIDTH) * usize::from(FRAME_HEIGHT) * 3;
    let mut bytes = vec![0_u8; frame_size];
    let mut sequence = 0_u64;
    let mut announced_live = false;

    loop {
        if stop.load(Ordering::Relaxed) {
            terminate(&mut child);
            return Ok(());
        }
        match stdout.read_exact(&mut bytes) {
            Ok(()) => {
                if !announced_live {
                    if sender.send(MediaEvent::Status(SourceStatus::Live)).is_err() {
                        terminate(&mut child);
                        return Ok(());
                    }
                    announced_live = true;
                }
                sequence = sequence.wrapping_add(1);
                let frame = VideoFrame {
                    width: FRAME_WIDTH,
                    height: FRAME_HEIGHT,
                    pixels: Arc::from(bytes.clone()),
                    sequence,
                    received_at: Instant::now(),
                };
                // A bounded channel intentionally applies backpressure. The TUI
                // wants the freshest frame, not an ever-growing queue.
                if sender.send(MediaEvent::Frame(frame)).is_err() {
                    terminate(&mut child);
                    return Ok(());
                }
            }
            Err(error) => {
                let status = child
                    .wait()
                    .ok()
                    .and_then(|s| s.code())
                    .map_or_else(|| "unknown".into(), |c| c.to_string());
                bail!("video stream ended (FFmpeg status {status}): {error}");
            }
        }
    }
}

fn ffmpeg_command(resolved: &ResolvedInput, source: &SourceConfig, fps: u8) -> Result<Command> {
    let mut command = Command::new("ffmpeg");
    command.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
    if resolved.reconnect {
        command.args([
            "-rw_timeout",
            "15000000",
            "-reconnect",
            "1",
            "-reconnect_streamed",
            "1",
            "-reconnect_delay_max",
            "5",
        ]);
    }
    if resolved.loop_file {
        command.args(["-stream_loop", "-1", "-re"]);
    }
    command.args(&resolved.input_args);
    if !source.headers.is_empty() {
        let headers = source
            .headers
            .iter()
            .map(|(key, value)| format!("{key}: {value}\r\n"))
            .collect::<String>();
        command.args(["-headers", &headers]);
    }
    let filter = format!(
        "fps={fps},scale={FRAME_WIDTH}:{FRAME_HEIGHT}:force_original_aspect_ratio=decrease,pad={FRAME_WIDTH}:{FRAME_HEIGHT}:(ow-iw)/2:(oh-ih)/2:black"
    );
    command
        .arg("-i")
        .arg(&resolved.input)
        .args([
            "-map", "0:v:0", "-an", "-sn", "-dn", "-vf", &filter, "-pix_fmt", "rgb24", "-f",
            "rawvideo", "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    Ok(command)
}

fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn short_error(error: &anyhow::Error) -> String {
    let value = error.to_string();
    if value.chars().count() <= 120 {
        value
    } else {
        format!("{}…", value.chars().take(119).collect::<String>())
    }
}

fn slug(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_remote_stream() {
        let result = resolve_input("https://example.com/live.m3u8", SourceKind::Auto).unwrap();
        assert!(result.reconnect);
        assert!(!result.loop_file);
    }

    #[test]
    fn resolves_explicit_file() {
        let result = resolve_input("demo.mp4", SourceKind::File).unwrap();
        assert!(result.loop_file);
    }

    #[test]
    fn resolves_rtsp_transport_without_http_flags() {
        let result = resolve_input("rtsp://camera.local/live", SourceKind::Auto).unwrap();
        assert!(!result.reconnect);
        assert!(
            result
                .input_args
                .iter()
                .any(|value| value == "-rtsp_transport")
        );
    }

    #[test]
    fn rejects_empty_camera() {
        assert!(resolve_input("camera://", SourceKind::Camera).is_err());
    }
}
