use std::time::{Duration, Instant};

use crate::{
    config::{Config, Renderer},
    media::{MediaEvent, SourceStatus, SourceWorker, VideoFrame},
};

pub struct Feed {
    pub worker: SourceWorker,
    pub frame: Option<VideoFrame>,
    pub status: SourceStatus,
    pub paused: bool,
    pub frames_seen: u64,
    pub measured_fps: f32,
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
}

impl App {
    pub fn new(config: Config) -> Self {
        let now = Instant::now();
        let feeds = config
            .sources
            .iter()
            .cloned()
            .map(|source| Feed {
                worker: SourceWorker::spawn(source, config.ui.fps),
                frame: None,
                status: SourceStatus::Connecting,
                paused: false,
                frames_seen: 0,
                measured_fps: 0.0,
                fps_window_started: now,
                fps_window_frames: 0,
            })
            .collect();
        let show_help = config.ui.show_help;
        Self {
            config,
            feeds,
            selected: 0,
            show_help,
            show_details: false,
            running: true,
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

    pub fn stop(&mut self) {
        self.running = false;
        for feed in &mut self.feeds {
            feed.worker.stop();
        }
    }
}
