use std::{
    io::{self, Read},
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
const YTDLP_BINARY: &str = "yt-dlp";
const YTDLP_FORMAT: &str =
    "best[ext=mp4][vcodec!=none][acodec!=none]/best[vcodec!=none][acodec!=none]/best";

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
    Metadata(StreamMetadata),
}

#[derive(Debug, Clone, Default)]
pub struct StreamMetadata {
    pub title: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
    pub codec: Option<String>,
    pub width: Option<u16>,
    pub height: Option<u16>,
    pub fps: Option<f32>,
    pub format: Option<String>,
}

impl StreamMetadata {
    pub fn compact(&self) -> String {
        let mut parts = Vec::new();
        if let Some(codec) = &self.codec {
            parts.push(codec.to_uppercase());
        }
        if let (Some(width), Some(height)) = (self.width, self.height) {
            parts.push(format!("{width}×{height}"));
        }
        parts.join(" · ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceStatus {
    Connecting,
    Live,
    Reconnecting,
    Failed(String),
    Stopped,
}

#[derive(Debug, Clone)]
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
        let metadata_source = source.clone();
        let metadata_sender = sender.clone();
        let _ = thread::Builder::new()
            .name(format!("metadata-{}", slug(&name)))
            .spawn(move || {
                if let Some(metadata) = probe_metadata(&metadata_source) {
                    let _ = metadata_sender.send(MediaEvent::Metadata(metadata));
                }
            });
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

fn probe_metadata(source: &SourceConfig) -> Option<StreamMetadata> {
    if source.kind == SourceKind::Camera || source.input.starts_with("camera://") {
        return None;
    }
    let resolved = resolve_input(&source.input, source.kind).ok()?;
    let mut command = Command::new("ffprobe");
    command.args([
        "-v",
        "error",
        "-rw_timeout",
        "8000000",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=codec_name,width,height,avg_frame_rate:stream_tags=title:format=format_name:format_tags=title,location,comment,description",
        "-of",
        "default=noprint_wrappers=1",
    ]);
    command.args(&resolved.input_args);
    if !source.headers.is_empty() {
        let headers = source
            .headers
            .iter()
            .map(|(key, value)| format!("{key}: {value}\r\n"))
            .collect::<String>();
        command.args(["-headers", &headers]);
    }
    let output = command.arg(&resolved.input).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let mut metadata = StreamMetadata::default();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "codec_name" => metadata.codec = clean_metadata(value),
            "width" => metadata.width = value.parse().ok(),
            "height" => metadata.height = value.parse().ok(),
            "avg_frame_rate" => metadata.fps = parse_rate(value),
            "format_name" => metadata.format = clean_metadata(value),
            "TAG:title" if metadata.title.is_none() => metadata.title = clean_metadata(value),
            "TAG:location" => metadata.location = clean_metadata(value),
            "TAG:comment" | "TAG:description" => metadata.description = clean_metadata(value),
            _ => {}
        }
    }
    Some(metadata)
}

fn clean_metadata(value: &str) -> Option<String> {
    let value = value
        .chars()
        .filter(|character| !character.is_control())
        .take(160)
        .collect::<String>();
    (!value.trim().is_empty()).then_some(value)
}

fn parse_rate(value: &str) -> Option<f32> {
    if let Some((numerator, denominator)) = value.split_once('/') {
        let numerator: f32 = numerator.parse().ok()?;
        let denominator: f32 = denominator.parse().ok()?;
        (denominator != 0.0).then_some(numerator / denominator)
    } else {
        value.parse().ok()
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

pub fn ytdlp_available() -> bool {
    binary_available(YTDLP_BINARY)
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

fn binary_available(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
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
    let resolved_input = if is_youtube_url(input) {
        resolve_youtube_url(input)?
    } else {
        input.to_owned()
    };
    let is_http = resolved_input.starts_with("http://") || resolved_input.starts_with("https://");
    let input_args = if resolved_input.starts_with("rtsp://") {
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
        input: resolved_input,
        input_args,
        reconnect: is_http,
        loop_file: is_file,
    })
}

fn is_youtube_url(input: &str) -> bool {
    let Some((scheme, rest)) = input.split_once("://") else {
        return false;
    };
    if !matches!(scheme, "http" | "https") {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority
        .rsplit('@')
        .next()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        host.as_str(),
        "youtube.com"
            | "www.youtube.com"
            | "m.youtube.com"
            | "music.youtube.com"
            | "youtu.be"
            | "www.youtu.be"
            | "youtube-nocookie.com"
            | "www.youtube-nocookie.com"
    )
}

fn resolve_youtube_url(input: &str) -> Result<String> {
    let output = Command::new(YTDLP_BINARY)
        .args([
            "--ignore-config",
            "--no-playlist",
            "--no-warnings",
            "--quiet",
            "--no-progress",
            "--socket-timeout",
            "10",
            "--retries",
            "1",
            "--fragment-retries",
            "1",
            "--extractor-retries",
            "1",
            "--format",
            YTDLP_FORMAT,
            "--get-url",
            "--",
            input,
        ])
        .output()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                anyhow::anyhow!(
                    "YouTube page URLs require `yt-dlp` on PATH; install it from https://github.com/yt-dlp/yt-dlp"
                )
            } else {
                anyhow::anyhow!(error).context("could not start yt-dlp")
            }
        })?;
    if !output.status.success() {
        bail!(
            "yt-dlp could not resolve the YouTube page (status {})",
            output.status
        );
    }
    parse_resolved_youtube_url(&String::from_utf8_lossy(&output.stdout))
}

fn parse_resolved_youtube_url(output: &str) -> Result<String> {
    let urls = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let Some(url) = urls.first().copied() else {
        bail!("yt-dlp returned no playable media URL");
    };
    if urls.len() != 1 {
        bail!("yt-dlp returned multiple media URLs; expected one combined video URL");
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        bail!("yt-dlp returned an unsupported media URL");
    }
    if url.chars().any(char::is_control) {
        bail!("yt-dlp returned a media URL with control characters");
    }
    Ok(url.to_owned())
}

fn worker_loop(
    source: SourceConfig,
    fps: u8,
    sender: SyncSender<MediaEvent>,
    stop: Arc<AtomicBool>,
) {
    let youtube_source = is_youtube_url(&source.input);
    let mut resolved = match resolve_input(&source.input, source.kind) {
        Ok(value) => value,
        Err(error) => {
            let _ = sender.send(MediaEvent::Status(SourceStatus::Failed(error.to_string())));
            return;
        }
    };
    let mut attempt = 0_u32;
    while !stop.load(Ordering::Relaxed) {
        if attempt > 0 && youtube_source {
            match resolve_input(&source.input, source.kind) {
                Ok(value) => resolved = value,
                Err(error) => {
                    if sender
                        .send(MediaEvent::Status(SourceStatus::Failed(short_error(
                            &error,
                        ))))
                        .is_err()
                    {
                        return;
                    }
                    attempt = attempt.saturating_add(1);
                    let delay = Duration::from_secs(u64::from(attempt.min(5)));
                    let deadline = Instant::now() + delay;
                    while Instant::now() < deadline && !stop.load(Ordering::Relaxed) {
                        thread::sleep(Duration::from_millis(100));
                    }
                    continue;
                }
            }
        }
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
    fn detects_supported_youtube_page_hosts() {
        assert!(is_youtube_url("https://www.youtube.com/watch?v=abc"));
        assert!(is_youtube_url("https://youtu.be/abc"));
        assert!(is_youtube_url("https://music.youtube.com/watch?v=abc"));
        assert!(!is_youtube_url("https://example.com/watch?v=abc"));
        assert!(!is_youtube_url(
            "https://youtube.com.evil.example/watch?v=abc"
        ));
    }

    #[test]
    fn parses_one_resolved_youtube_url() {
        assert_eq!(
            parse_resolved_youtube_url("\nhttps://video.example/stream.mp4?token=secret\n")
                .unwrap(),
            "https://video.example/stream.mp4?token=secret"
        );
        assert!(parse_resolved_youtube_url("").is_err());
        assert!(
            parse_resolved_youtube_url("https://a.example/one\nhttps://b.example/two").is_err()
        );
        assert!(parse_resolved_youtube_url("file:///tmp/video.mp4").is_err());
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

    #[test]
    fn parses_fractional_frame_rate() {
        assert_eq!(parse_rate("30000/1001").unwrap().round(), 30.0);
        assert!(parse_rate("0/0").is_none());
    }

    #[test]
    fn metadata_text_cannot_inject_terminal_controls() {
        assert_eq!(
            clean_metadata("City\u{1b}[2J Camera").unwrap(),
            "City[2J Camera"
        );
    }
}
