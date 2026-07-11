---
name: jukebox-tui-visual-qa
description: Render and inspect jukebox TUI states using deterministic fixtures. Test terminal dimensions, long text, Unicode, wide glyphs, narrow layouts, no-color, keyboard focus, overlays, status messages, and mouse hitboxes. Use before claiming a UI change is complete or when auditing visual quality.
---

# Jukebox TUI Visual QA

Render and inspect all important TUI states. Reject layouts that technically render but are confusing, clipped, unstable, or unusable.

## When to use

- After any TUI change (layout, overlay, player bar, columns, footer).
- Before a release candidate (visual regression check).
- When auditing accessibility (NO_COLOR, narrow terminals, CJK).

## Procedure

1. **Snapshot tests:** Run `cargo test --test layout -- --nocapture` to verify insta snapshots. Any new state needs a snapshot.
2. **Terminal dimensions:** Test at 160×50 (wide), 80×24 (standard), 60×20 (narrow), 40×15 (too-small). Verify: wide=full Miller columns; narrow=single pane; too-small=message.
3. **Long text:** Track titles >80 chars, artist names >40 chars, album names >60 chars. Verify: truncation or wrapping, not overlap. Check `pad_between` right-anchored quality tag.
4. **Unicode/CJK:** Katakana/hiragana titles (width=2), emoji (width=2), combining marks (width=0). Verify: alignment, no clipping, `pad_between` correct.
5. **NO_COLOR mode:** `NO_COLOR=1 cargo run -- play`. Verify: all colors collapse to `Reset`; selection still visible (glyph or inverse, not color-only).
6. **Keyboard focus:** Verify focus column has accent border; unfocused have dim border. Verify `focus_col` clamped to `max_focus_col`.
7. **Overlays:** Open each overlay type (Search, Help, PlaylistPicker, Command, YtAuth, Discover). Verify: Esc closes; typing routes correctly; no nested-overlay crash.
8. **Status messages:** `yt_status` / `yt_error` in footer. Verify: auto-clear (TTL), no permanent stale messages, error color (yellow) vs status color (accent).
9. **Mouse hitboxes:** Click transport glyphs, progress gauge, volume meter, browse rows. Verify: hit-test maps to the right action (approximate is OK — see `input.rs:651-653`).
10. **Now-playing:** Play a local track, a YT track. Verify: `▶` glyph on now-playing row, quality label correct, progress gauge updates.

## Rejection criteria

- Layout technically renders but is **confusing** (unclear what's focused, what's playing).
- **Clipped** text that cuts off critical info (track title, error message).
- **Unstable** layout that flickers or jumps on resize.
- **Unusable** at narrow dimensions (can't navigate, can't read).
- **Color-only signal** that disappears under NO_COLOR.

## Key files

- `src/tui/view/layout.rs` — top-level layout + breakpoints.
- `src/tui/view/columns.rs` — Miller columns + track rows.
- `src/tui/view/player_bar.rs` — now-playing + gauge + flags.
- `src/tui/view/overlay.rs` — all overlay rendering.
- `src/tui/view/theme.rs` — colors, `pad_between`, `NO_COLOR`.
- `tests/snapshots/*.snap` — insta snapshots.

## References

- `references/state-inventory.md` — the state checklist (required for every visual QA pass).
- `references/visual-qa-checklist.md` — step-by-step visual QA template.
- `tests/snapshots/*.snap`, `tests/layout.rs` — snapshot test patterns.
