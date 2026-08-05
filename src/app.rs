use std::time::{Duration, Instant};

use crate::{
    config::{Config, Renderer, SourceConfig, SourceKind, source_name},
    media::{MediaEvent, SourceStatus, SourceWorker, StreamMetadata, VideoFrame},
};

pub struct Feed {
    pub worker: SourceWorker,
    pub frame: Option<VideoFrame>,
    pub status: SourceStatus,
    pub paused: bool,
    pub frames_seen: u64,
    pub measured_fps: f32,
    pub source: SourceConfig,
    pub metadata: StreamMetadata,
    fps_window_started: Instant,
    fps_window_frames: u32,
}

pub struct App {
    pub config: Config,
    pub feeds: Vec<Feed>,
    pub selected: usize,
    pub show_help: bool,
    pub show_details: bool,
    pub running: bool,
    pub adding_source: bool,
    pub source_input: String,
    pub source_error: Option<String>,
}

impl App {
    pub fn new(config: Config) -> Self {
        let now = Instant::now();
        let feeds = config
            .sources
            .iter()
            .cloned()
            .map(|source| Self::new_feed(source, config.ui.fps, now))
            .collect();
        let show_help = config.ui.show_help;
        Self {
            config,
            feeds,
            selected: 0,
            show_help,
            show_details: false,
            running: true,
            adding_source: false,
            source_input: String::new(),
            source_error: None,
        }
    }

    fn new_feed(source: SourceConfig, fps: u8, now: Instant) -> Feed {
        Feed {
            worker: SourceWorker::spawn(source.clone(), fps),
            frame: None,
            status: SourceStatus::Connecting,
            paused: false,
            frames_seen: 0,
            measured_fps: 0.0,
            source,
            metadata: StreamMetadata::default(),
            fps_window_started: now,
            fps_window_frames: 0,
        }
    }

    pub fn update(&mut self) {
        for feed in &mut self.feeds {
            while let Ok(event) = feed.worker.receiver.try_recv() {
                match event {
                    MediaEvent::Frame(frame) => {
                        feed.frames_seen += 1;
                        feed.fps_window_frames += 1;
                        if !feed.paused {
                            feed.frame = Some(frame);
                        }
                    }
                    MediaEvent::Status(status) => feed.status = status,
                    MediaEvent::Metadata(metadata) => feed.metadata = metadata,
                }
            }
            let elapsed = feed.fps_window_started.elapsed();
            if elapsed >= Duration::from_secs(1) {
                feed.measured_fps = feed.fps_window_frames as f32 / elapsed.as_secs_f32();
                feed.fps_window_frames = 0;
                feed.fps_window_started = Instant::now();
            }
        }
    }

    pub fn next(&mut self) {
        if !self.feeds.is_empty() {
            self.selected = (self.selected + 1) % self.feeds.len();
        }
    }

    pub fn previous(&mut self) {
        if !self.feeds.is_empty() {
            self.selected = self.selected.checked_sub(1).unwrap_or(self.feeds.len() - 1);
        }
    }

    pub fn toggle_pause(&mut self) {
        if let Some(feed) = self.feeds.get_mut(self.selected) {
            feed.paused = !feed.paused;
        }
    }

    pub fn toggle_renderer(&mut self) {
        self.config.ui.renderer = match self.config.ui.renderer {
            Renderer::Ascii => Renderer::Blocks,
            Renderer::Blocks => Renderer::Ascii,
        };
    }

    pub fn open_source_entry(&mut self) {
        self.show_help = false;
        self.show_details = false;
        self.adding_source = true;
        self.source_input.clear();
        self.source_error = None;
    }

    pub fn cancel_source_entry(&mut self) {
        self.adding_source = false;
        self.source_input.clear();
        self.source_error = None;
    }

    pub fn submit_source(&mut self) {
        let input = self.source_input.trim().to_owned();
        if input.is_empty() {
            self.source_error = Some("Paste a direct stream URL or local media path.".into());
            return;
        }
        if input.chars().count() > 4096 {
            self.source_error = Some("That source is too long.".into());
            return;
        }
        if input.chars().any(char::is_control) {
            self.source_error = Some("The source cannot contain control characters.".into());
            return;
        }
        let index = self.feeds.len();
        let source = SourceConfig {
            name: source_name(&input, index),
            kind: if input.starts_with("camera://") {
                SourceKind::Camera
            } else {
                SourceKind::Auto
            },
            input,
            headers: Default::default(),
        };
        self.feeds.push(Self::new_feed(
            source.clone(),
            self.config.ui.fps,
            Instant::now(),
        ));
        self.config.sources.push(source);
        self.selected = self.feeds.len() - 1;
        self.cancel_source_entry();
    }

    pub fn remove_selected(&mut self) {
        if self.selected >= self.feeds.len() {
            return;
        }
        let mut feed = self.feeds.remove(self.selected);
        feed.worker.stop();
        self.config.sources.remove(self.selected);
        self.selected = self.selected.min(self.feeds.len().saturating_sub(1));
        self.show_details = false;
    }

    pub fn stop(&mut self) {
        self.running = false;
        for feed in &mut self.feeds {
            feed.worker.stop();
        }
    }
}
