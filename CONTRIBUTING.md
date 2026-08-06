# Contributing

Thank you for helping monitor the situation.

## Development setup

Install Rust and FFmpeg, plus `yt-dlp` if you are working on YouTube page
support, then run:

```sh
cargo test
cargo run -- run path/to/a/video.mp4
```

Local video files loop automatically, making them the preferred development
source. Tests and examples must not depend on an external third-party camera.

Before opening a pull request:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## Project boundaries

- Do not add bundled third-party feeds or a camera directory.
- YouTube page support uses a separately installed `yt-dlp` process; do not
  bundle extractor code, browser cookies, or credentials.
- Do not add integrations that bypass a platform's access controls or official
  player rules.
- Never commit camera passwords, tokens, signed URLs, or private hostnames.
- Do not add recording or computer-vision features without prior discussion.
- New source types should normalize frames through the existing media boundary.
- Keep the interface fully usable without a mouse.

Contributions are submitted under the project's MIT license.
