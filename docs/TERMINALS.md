# Terminal behavior and compatibility

`monitorthesituation` is a terminal user interface, not a desktop overlay. It
uses the standard alternate-screen capability also used by programs such as
`vim`, `less`, and `htop`.

On startup it:

1. Enables raw keyboard input.
2. Enters the alternate screen.
3. Hides the cursor.
4. Draws only inside the terminal's current dimensions.

On normal exit or error it reverses those changes and restores the original
screen. `Ctrl-C` is handled as an application command while the interface is
open.

## Expected support

- Apple Terminal
- iTerm2
- Ghostty
- Kitty
- Alacritty
- WezTerm
- GNOME Terminal
- Konsole
- Windows Terminal

Other terminals implementing standard ANSI control sequences should work too.

The block renderer uses `▀` with independent foreground and background colors,
effectively displaying two image pixels per character cell. The ASCII renderer
uses the configured brightness ramp and is the most conservative fallback.

Some terminals report only 16 or 256 colors even though they accept 24-bit RGB.
The application currently sends true-color sequences and lets the terminal map
them to its available palette. Use `color: false` for predictable monochrome.

## Multiplexers and remote shells

The interface works inside `tmux` and over SSH, but terminal capability and
bandwidth affect the result. For remote sessions, lower `fps` to 4–8 and prefer
the ASCII renderer. True color in older `tmux` configurations may require
enabling RGB terminal overrides.

