# Research: ratatui NO_COLOR Accessibility Patterns

Date: 2026-07-12
Source: GitHub code search (1376 results for `BorderType::Double`), ratatui 0.30 crate
Confidence: HIGH (verified from real-world ratatui projects + our codebase uses ratatui 0.30)

## Context
Needed for M6.1 (D7 fix): focus/selection is currently color-only — fails under NO_COLOR. Need non-color cues that work in monochrome.

## 1. BorderType for focus indication (D7 fix)

### Available variants in ratatui 0.30:
```rust
pub enum BorderType {
    Plain,
    Rounded,
    Double,
    Thick,
    QuadrantInside,
    QuadrantOutside,
    // Newer variants (may not all be in 0.30):
    LightDoubleDashed,
    HeavyDoubleDashed,
}
```

### Usage pattern for NO_COLOR-safe focus (from LockKnife repo — `lockknife-tui/src/ui/exploit.rs`):
```rust
.border_type(if matches!(app.active_panel, Panel::Exploit) {
    BorderType::Double
} else {
    BorderType::Plain
})
```

### Our current code (`src/tui/view/columns.rs:49-55`):
```rust
fn border<'a>(title: &'a str, focused: bool, theme: &Theme) -> Block<'a> {
    let color = if focused { theme.accent } else { theme.dim };
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))  // COLOR-ONLY — fails under NO_COLOR
        .title(Span::styled(title, Style::default().fg(color)))
}
```

### Fix (color + border-type):
```rust
fn border<'a>(title: &'a str, focused: bool, theme: &Theme) -> Block<'a> {
    let color = if focused { theme.accent } else { theme.dim };
    let border_type = if focused { BorderType::Double } else { BorderType::Plain };
    Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)                    // NON-COLOR cue
        .border_style(Style::default().fg(color))   // color still set (ignored under NO_COLOR)
        .title(Span::styled(title, Style::default().fg(color)))
}
```

Under NO_COLOR, `theme.accent` and `theme.dim` both become `Color::Reset`, but the border TYPE still differs — focused column has `═══` double-line borders, unfocused has `───` single-line. Visually distinct in monochrome.

## 2. Modifier for selection indication (D7 fix)

### Available Modifier flags in ratatui:
```rust
pub enum Modifier {
    BOLD,
    DIM,
    ITALIC,
    UNDERLINED,
    SLOW_BLINK,    // already used in our overlay cursor
    RAPID_BLINK,
    REVERSED,      // reverse video — ideal for selection
    HIDDEN,
    CROSSED_OUT,
}
```

### Our current row selection (`columns.rs:434, 478`):
```rust
let style = if selected || np { accent } else { dim };  // COLOR-ONLY
```

### Fix (add REVERSED or BOLD):
```rust
let style = if selected {
    Style::default().fg(theme.accent).add_modifier(Modifier::REVERSED)
} else if np {
    Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)
} else {
    Style::default().fg(theme.dim)
};
```

Under NO_COLOR, the selected row gets reverse video (fg↔bg swap) — visually distinct without color. The now-playing row gets bold weight.

## 3. Rail active-view indication (D7/D8 fix)

### Current (`columns.rs:72-95`): accent color only.
```rust
let style = if app.view == *v { accent } else { dim };
```

### Fix (add BOLD + show the switch key):
```rust
let style = if app.view == *v {
    accent.add_modifier(Modifier::BOLD)
} else {
    dim
};
// Also show the digit key (D8): "1 A" / "2 P" / "3 Q" / "4 Y"
let label = format!("{} {}", key_digit, glyph);
```

## 4. Footer error/success distinction (D17 fix)

### Current (`footer.rs:22-34`): Yellow vs Cyan (both Reset under NO_COLOR).

### Fix (add glyph prefix):
```rust
let line = if let Some(e) = &app.yt_error {
    Line::from(Span::styled(
        format!("! YT: {e}"),   // "!" prefix for error
        Style::default().fg(if no_color() { Color::Reset } else { Color::Yellow })
            .add_modifier(Modifier::BOLD),
    ))
} else if let Some(s) = &app.yt_status {
    Line::from(Span::styled(
        format!("✓ {s}"),        // "✓" prefix for success
        Style::default().fg(if no_color() { Color::Reset } else { theme.accent }),
    ))
} else {
    hint_line(app, &dim)
};
```

Under NO_COLOR: `!` + bold = error, `✓` = success. No color needed.

## 5. Braille spinner ASCII fallback (D21 fix)

### Current (`player_bar.rs:30`):
```rust
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
```

### Fix (detect NO_COLOR or use ASCII fallback):
```rust
const SPINNER_UNICODE: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SPINNER_ASCII: [&str; 4] = ["|", "/", "-", "\\"];

fn spinner_frame(is_resolving: bool, frame: u8) -> &'static str {
    if !is_resolving { return "▶"; }
    if crate::tui::view::theme::no_color() {
        SPINNER_ASCII[(frame as usize) % SPINNER_ASCII.len()]
    } else {
        SPINNER_UNICODE[(frame as usize) % SPINNER_UNICODE.len()]
    }
}
```

## 6. Playlist/track list icons (D21 fix)

### Current (`columns.rs:229`): `♫` (account), `✦` (suggested).
### Fix: pair with text or ASCII:
- `♫` → `♫` (keep, but also show name)
- `✦` → `*` (ASCII fallback when NO_COLOR)
- Or always: `[A] Liked Songs` / `[S] Focus Flow` (letter prefix: A=Account, S=Suggested)

## Summary of imports needed:

```rust
// columns.rs:
use ratatui::widgets::{Block, BorderType, Borders, ...};
use ratatui::style::Modifier;

// footer.rs:
use ratatui::style::Modifier;

// player_bar.rs:
// (no new imports needed; SPINNER already a const)
```

## Verification:
After changes, test with:
```bash
NO_COLOR=1 cargo run -- play
```
- Focused column should have double-line border (═══) vs single-line (───)
- Selected track should be reverse-video
- Now-playing track should be bold
- Footer errors prefixed with `!`, successes with `✓`
- Spinner should show `|/-\` instead of braille
```
