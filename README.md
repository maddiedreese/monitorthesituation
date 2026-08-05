# monitorthesituation

Monitor any situation you want, directly from your terminal, through video feeds rendered in ASCII.

`monitorthesituation` runs entirely inside your existing terminal. It opens an
alternate screen, arranges your feeds into an adaptive grid, and restores the
terminal exactly as it found it when you quit. There is no desktop window,
account, server, telemetry, recording, or bundled camera directory.

## What it does

- Renders several sources simultaneously in one terminal window
- Lets you paste and name feeds without leaving the viewer
- Accepts HLS, RTSP, HTTP video, MJPEG, local video files, and USB webcams
- Offers detailed Unicode half-blocks and classic ASCII rendering
- Inherits the terminal's foreground and background palette; video art can use true color
- Supports color video and monochrome output
- Reconnects interrupted sources with bounded backoff
- Resizes dynamically with the terminal
- Fills each pane with a centered crop instead of letterboxing
- Shows six feeds per page by default, with configurable paging for larger walls
- Keeps credentials out of configuration through environment variables
- Decodes everything locally through FFmpeg
- Shows sanitized source, protocol, codec, format, resolution, and frame-rate details

## Requirements

- A reasonably modern terminal emulator
- [FFmpeg](https://ffmpeg.org/) (`ffmpeg` and `ffprobe` on `PATH`)
- Rust 1.85 or newer when installing from source

Install FFmpeg on common platforms:

```sh
# macOS
brew install ffmpeg

# Ubuntu / Debian
sudo apt install ffmpeg

# Windows
winget install Gyan.FFmpeg
```

## Install

Download a binary archive for macOS, Linux, or Windows from the GitHub Releases
page, or install from source:

```sh
git clone https://github.com/maddiedreese/monitorthesituation.git
cd monitorthesituation
cargo install --path .
```

During development, use `cargo run --` in place of `monitorthesituation`.

## Quick start

Start with an empty viewer:

```sh
monitorthesituation
```

Press `a`, enter `Location name | URL`, and press Enter. The location becomes
the pane title. Entering a URL by itself also works: the viewer uses an embedded
location or title when the stream provides one, then falls back to its hostname.
Press `a` again to add another feed, and the viewer rearranges the panes. URLs may be
HLS (`.m3u8`), RTSP, direct HTTP video, or MJPEG streams. A webpage containing a
video player is not itself a stream URL.

Open one or more authorized streams directly:

```sh
monitorthesituation run \
  'https://camera.example/live/index.m3u8' \
  'rtsp://camera.local/live'
```

Command-line options can temporarily override the configuration:

```sh
monitorthesituation run --ascii --mono --fps 6 --columns 2 --max-panes 8 <URL> <URL>
```

Open a local webcam:

```sh
monitorthesituation run camera://0
```

List camera devices first if you are not sure which one to use:

```sh
monitorthesituation devices
```

Or create a persistent configuration:

```sh
monitorthesituation init
```

This creates:

- macOS: `~/Library/Application Support/monitorthesituation/config.yaml`
- Linux: `~/.config/monitorthesituation/config.yaml`
- Windows: `%APPDATA%\monitorthesituation\config.yaml`

Then run `monitorthesituation` with no arguments.

If something is not behaving as expected, run the built-in installation check:

```sh
monitorthesituation doctor
```

## Configuration

```yaml
version: 1

ui:
  renderer: blocks       # blocks | ascii
  color: true
  fps: 10                # 1–30
  columns: auto          # auto, or a fixed number
  max_panes: 6           # visible at once; additional feeds use pages
  show_help: true
  ascii_ramp: " .:-=+*#%@"

sources:
  - name: Harbor
    kind: stream
    input: https://camera.example/live/index.m3u8

  - name: Front gate
    kind: stream
    input: ${FRONT_GATE_URL}

  - name: USB camera
    kind: camera
    input: camera://0
```

Supported `kind` values are `auto`, `stream`, `camera`, and `file`. `auto`
detects local files and camera URLs automatically. Local files loop so they are
useful for testing layouts.

Authenticated HTTP streams can use headers without committing secrets:

```yaml
sources:
  - name: Private stream
    input: ${PRIVATE_STREAM_URL}
    headers:
      Authorization: "Bearer ${PRIVATE_STREAM_TOKEN}"
```

Environment variables are expanded only inside source inputs and header values.
They are never displayed in the interface or diagnostic messages.

## Controls

| Key | Action |
|---|---|
| `Tab`, arrows, `h/j/k/l` | Select a pane |
| `1`–`9` | Select a pane by number |
| `a` | Paste and add another feed |
| `x` | Remove the selected feed from this session |
| `[` / `]`, `Page Up` / `Page Down` | Move between feed pages |
| `s` | Change how many panes are visible (1–36) |
| `Space` | Pause the selected image |
| `r` | Toggle block/ASCII rendering |
| `c` | Toggle color/monochrome |
| `i` | Show source and video-stream information |
| `?` | Show help |
| `q`, `Ctrl-C` | Quit |

## Terminal compatibility

True-color terminals produce the best result. Limited terminals still receive
ordinary Unicode or ASCII output; `color: false` avoids relying on hue entirely.
See [docs/TERMINALS.md](docs/TERMINALS.md) for compatibility details.

## Source responsibility

`monitorthesituation` does not discover, scrape, bypass, bundle, archive, or
redistribute third-party feeds. Only connect to sources you own or are allowed
to view through a direct media URL. In particular, a public webpage containing
a video player does not necessarily authorize extracting its underlying stream.

Demo footage and screenshots published by this project require documented
permission from the camera operator. See [docs/CAMERA_PERMISSION.md](docs/CAMERA_PERMISSION.md).

## Privacy

Frames move directly from FFmpeg into terminal memory. FFprobe reads technical
stream metadata from the same source for the information panel. The application
does not record frames, write them to disk, inspect their contents, or send them
elsewhere. Pausing a pane freezes its latest in-memory frame while decoding
continues. Source locations shown in the interface omit URL credentials, paths,
query strings, and fragments.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build --release
```

The project is MIT-licensed. Contributions are welcome; see
[CONTRIBUTING.md](CONTRIBUTING.md).
