# jukebox

A filtered-lossless jukebox: a bash indexer that dedupes a FLAC library into a
symlinked, artist-organized view, plus a Rust TUI with model-free multilingual
search and mpv playback.

The originals in `lossless/` are never mutated — `standardize.sh` only writes
symlinks, `catalog.json`, and a Tantivy index into `filtered_lossless/`.

## Install

```bash
cargo binstall jukebox          # downloads the prebuilt binary (no compile)
jukebox                         # first run prompts for your lossless source dir
jukebox sync                    # build symlinks + catalog + search index
jukebox play                    # launch the TUI
```

`cargo binstall` downloads the prebuilt binary from the
[releases page](https://github.com/bibimoni/jukebox/releases) — recommended,
since building from source embeds the ~40MB IPADIC dictionary (slow, RAM-heavy).

<details><summary>Alternative install methods</summary>

```bash
# From source (slower; embeds the IPADIC dictionary at build time):
cargo install jukebox

# Or grab a binary archive directly from a release:
tar xzf jukebox-<target>.tar.gz      # e.g. jukebox-aarch64-apple-darwin.tar.gz
./jukebox
```

Or build from a checkout:

```bash
cargo build --release
./target/release/jukebox
```

</details>

### Runtime prerequisites

- `metaflac`, `ffprobe`, `jq`, `yq` — for `jukebox sync`
- `mpv` — for playback (falls back to `afplay` on macOS if unavailable)
- **YouTube (optional):** `python3` + `ytmusicapi` + `yt-dlp` — install with
  `pip install -r scripts/yt/requirements.txt` (or run `:yt setup` from inside
  the TUI). Without these, the YouTube view shows a setup hint; local
  playback is fully functional regardless.

## YouTube integration

jukebox can also stream from YouTube alongside your local hi-res library. Three
source modes are cycled with `M`: **Local** (your lossless catalog only),
**YouTube** (account playlists, suggested playlists, search, and YouTube's own
autoplay radio), and **Mixed** (play the local copy when a track matches your
library by ISRC or normalized artist+title, else stream from YouTube).

- `4` switches to the **Y** view (account playlists `♫` + suggested/mood
  `✦`, with an Up-Next pane for short lists).
- `M` cycles the source mode; the active mode is shown as a `MODE` flag in the
  player bar. Switching mode never stops playback.
- `/` searches (scoped to the view — YouTube in the Y view, local BM25
  otherwise); `f` filters the focused column; `s` plays a random track; `S`
  opens a discover overlay.
- **Auth:** run `:yt auth` inside the TUI and paste your YouTube cookies
  (Netscape cookies.txt — export with a "Get cookies.txt" browser extension
  from a logged-in `youtube.com` tab; **a Premium account is recommended** — it
  unlocks ad-free 256k AAC streams and account rate limits). One paste feeds
  both the metadata sidecar (`ytmusicapi`) and the stream resolver (`yt-dlp`).
  `:yt logout` clears the cookies.

> **YouTube Terms of Service.** Automated access to YouTube may violate
> YouTube's Terms of Service. This integration is intended for personal use
> with content you have the right to access (e.g. your own Premium account).
> YouTube audio is lossy (~256k AAC / ~160k Opus, ≤ 48 kHz) — local tracks stay
> bit-perfect and CoreAudio re-clocks the device to the stream's rate when a
> YouTube session begins (held across consecutive YT tracks, restored on
> return to local). You are responsible for your use; the authors provide no
> warranty. "YouTube" is a trademark of Google LLC; this project is not
> affiliated with or endorsed by YouTube.


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
