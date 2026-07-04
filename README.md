# jukebox

A filtered-lossless jukebox: a bash indexer that dedupes a FLAC library into a
symlinked, artist-organized view, plus a Rust TUI with model-free multilingual
search and mpv playback.

The originals in `lossless/` are never mutated — `standardize.sh` only writes
symlinks, `catalog.json`, and a Tantivy index into `filtered_lossless/`.

## Setup

```bash
cargo build --release
./target/release/jukebox          # first run prompts for your lossless source dir
./target/release/jukebox sync      # build symlinks + catalog + search index
./target/release/jukebox play      # launch the TUI
```

`jukebox sync` walks every `.flac` under the configured source dir, reads tags
via `metaflac` and quality via `ffprobe`, keeps the best copy per song (highest
bit depth → sample rate → ISRC → TIDAL id → tag count → shortest path), and
symlinks each unique track into `filtered_lossless/<Artist>/` with a
standardized name:

```
filtered_lossless/<Artist>/<canonicalArtists> - <TITLE> [<bitDepth>bit-<kHz>kHz].flac
```

A collaboration is symlinked into **every** constituent artist's folder with
the same canonical filename; audio bytes are never duplicated.

## Config

Resolved via the `dirs` crate:

- `$XDG_CONFIG_HOME/jukebox/config.yml` if `XDG_CONFIG_HOME` is set, else
- `~/Library/Application Support/jukebox/config.yml` on macOS, or
- `~/.config/jukebox/config.yml` on Linux.

```yaml
version: 1
source_dir: /Users/you/Music/lossless
filtered_dir: /Users/you/Music/filtered_lossless
player: mpv          # mpv | afplay
mpv_socket: /tmp/jukebox-mpv.sock
```

`jukebox config` re-runs the first-run prompt.

## Commands

| command | what it does |
|---|---|
| `jukebox` / `jukebox play` | launch the TUI |
| `jukebox sync` | run `scripts/standardize.sh` then rebuild the search index |
| `jukebox index` | rebuild the Tantivy search index from `catalog.json` |
| `jukebox search <query>` | one-shot CLI search (prints ranked results) |
| `jukebox config` | show / re-run the config prompt |

## Search

Full-text search is model-free and offline: [Tantivy](https://github.com/quickwit-oss/tantivy)
(BM25 ranking + fuzzy edit-distance matching) with [Lindera](https://github.com/lindera/lindera)
Japanese morphological tokenization. Cross-script matches work via algorithmic
kana↔romaji transliteration — typing `burubado` finds `ブルーバード`, and typos
like `Freedon` still match `Freedom`.

## Keybindings (TUI)

`Tab` cycle panes · `↑/↓` move cursor · `/` focus search · `space` enqueue
artist (Artists pane) · `enter` enqueue result (Search) / play-now (Queue) ·
`s`/`S` shuffle (S also jumps) · `r` remove · `c` clear queue · `n`/`p`
next/prev · `←/→` seek ±5s · `q` quit (stops playback).

If `mpv` isn't available, playback falls back to `afplay` (no seek).

## Design

See `specs/2026-07-04-filtered-lossless-jukebox-design.md` (design) and
`specs/2026-07-04-jukebox-implementation-plan.md` (task-by-task plan).

## Development

```bash
cargo test                  # Rust unit + integration tests
bats scripts/test/*.bats    # bash helper + standardize.sh tests
```
