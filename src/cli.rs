use std::{fs, io::IsTerminal, path::PathBuf, process::Command};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use crate::{
    config::{Config, DEFAULT_CONFIG, SourceConfig, SourceKind, source_name},
    media, tui,
};

#[derive(Debug, Parser)]
#[command(name = "monitorthesituation", version, about, propagate_version = true)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Open the video wall (the default command)
    Run {
        /// Configuration file to load
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Use classic brightness-mapped ASCII characters
        #[arg(long, conflicts_with = "blocks")]
        ascii: bool,
        /// Use two-pixel Unicode half blocks (the default)
        #[arg(long, conflicts_with = "ascii")]
        blocks: bool,
        /// Render luminance only
        #[arg(long)]
        mono: bool,
        /// Override the configured decode rate (1–30)
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=30))]
        fps: Option<u8>,
        /// Override the automatic grid width
        #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
        columns: Option<u16>,
        /// Number of panes shown on one page (1–36)
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=36))]
        max_panes: Option<u8>,
        /// Stream URLs, YouTube page URLs, or local media paths
        inputs: Vec<String>,
    },
    /// Create a documented starter configuration
    Init {
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    /// Ask FFprobe to inspect a source without opening the TUI
    Probe { input: String },
    /// List local capture devices reported by FFmpeg
    Devices,
    /// Check the local installation and configuration
    Doctor,
}

pub fn run(cli: Cli) -> Result<()> {
    match cli.command.unwrap_or(Commands::Run {
        config: None,
        ascii: false,
        blocks: false,
        mono: false,
        fps: None,
        columns: None,
        max_panes: None,
        inputs: Vec::new(),
    }) {
        Commands::Run {
            config,
            ascii,
            blocks,
            mono,
            fps,
            columns,
            max_panes,
            inputs,
        } => run_wall(
            config,
            inputs,
            RunOverrides {
                ascii,
                blocks,
                mono,
                fps,
                columns,
                max_panes,
            },
        ),
        Commands::Init { output, force } => init(output, force),
        Commands::Probe { input } => probe(&input),
        Commands::Devices => devices(),
        Commands::Doctor => doctor(),
    }
}

struct RunOverrides {
    ascii: bool,
    blocks: bool,
    mono: bool,
    fps: Option<u8>,
    columns: Option<u16>,
    max_panes: Option<u8>,
}

fn run_wall(path: Option<PathBuf>, inputs: Vec<String>, overrides: RunOverrides) -> Result<()> {
    media::ensure_ffmpeg()?;
    let mut config = match path {
        Some(path) => Config::load(&path)?,
        None => match default_config_path().filter(|path| path.exists()) {
            Some(path) => Config::load(&path)?,
            None => Config::default(),
        },
    };
    for (index, input) in inputs.into_iter().enumerate() {
        config.sources.push(SourceConfig {
            name: source_name(&input, index),
            kind: if input.starts_with("camera://") {
                SourceKind::Camera
            } else {
                SourceKind::Auto
            },
            input,
            headers: Default::default(),
        });
    }
    if overrides.ascii {
        config.ui.renderer = crate::config::Renderer::Ascii;
    }
    if overrides.blocks {
        config.ui.renderer = crate::config::Renderer::Blocks;
    }
    if overrides.mono {
        config.ui.color = false;
    }
    if let Some(fps) = overrides.fps {
        config.ui.fps = fps;
    }
    if let Some(columns) = overrides.columns {
        config.ui.columns = crate::config::Columns::Fixed(columns);
    }
    if let Some(max_panes) = overrides.max_panes {
        config.ui.max_panes = max_panes;
    }
    tui::run(config)
}

fn init(output: Option<PathBuf>, force: bool) -> Result<()> {
    let path = output
        .or_else(default_config_path)
        .context("could not determine a configuration directory; pass --output")?;
    if path.exists() && !force {
        bail!(
            "{} already exists; pass --force to replace it",
            path.display()
        );
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, DEFAULT_CONFIG)
        .with_context(|| format!("could not write {}", path.display()))?;
    println!("Created {}", path.display());
    Ok(())
}

fn probe(input: &str) -> Result<()> {
    media::ensure_ffprobe()?;
    let resolved = media::resolve_input(input, SourceKind::Auto)?;
    let status = Command::new("ffprobe")
        .args(["-hide_banner", "-show_format", "-show_streams"])
        .arg(&resolved.input)
        .status()
        .context("could not start ffprobe")?;
    if !status.success() {
        bail!("FFprobe could not read the source");
    }
    Ok(())
}

fn devices() -> Result<()> {
    media::ensure_ffmpeg()?;
    let mut command = Command::new("ffmpeg");
    command.args(["-hide_banner"]);
    if cfg!(target_os = "macos") {
        command.args(["-f", "avfoundation", "-list_devices", "true", "-i", ""]);
    } else if cfg!(target_os = "windows") {
        command.args(["-f", "dshow", "-list_devices", "true", "-i", "dummy"]);
    } else {
        println!("Video devices commonly appear as /dev/video0, /dev/video1, …");
        command.args(["-f", "v4l2", "-list_formats", "all", "-i", "/dev/video0"]);
    }
    let _ = command.status().context("could not start ffmpeg")?;
    Ok(())
}

fn doctor() -> Result<()> {
    println!("monitorthesituation {}", env!("CARGO_PKG_VERSION"));
    media::ensure_ffmpeg()?;
    println!("✓ ffmpeg available");
    media::ensure_ffprobe()?;
    println!("✓ ffprobe available");
    if media::ytdlp_available() {
        println!("✓ yt-dlp available (YouTube page URLs enabled)");
    } else {
        println!("· yt-dlp not found (YouTube page URLs disabled)");
    }

    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        println!("✓ interactive terminal detected");
    } else {
        println!("! no interactive terminal detected in this invocation");
    }

    match default_config_path() {
        Some(path) if path.exists() => {
            let config = Config::load(&path)?;
            println!(
                "✓ configuration valid: {} ({} sources)",
                path.display(),
                config.sources.len()
            );
        }
        Some(path) => println!("· no default configuration yet ({})", path.display()),
        None => println!("! could not determine the default configuration directory"),
    }
    println!("Ready to monitor the situation.");
    Ok(())
}

fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|path| path.join("monitorthesituation/config.yaml"))
}
