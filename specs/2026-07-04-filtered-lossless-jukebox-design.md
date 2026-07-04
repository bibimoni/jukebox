# Filtered Lossless Library + Jukebox CLI — Design

**Date:** 2026-07-04
**Status:** Revised (pending spec review)
**Scope:** A symlinked, deduplicated, artist-organized view over `~/Music/lossless/` produced by a **bash script**, plus a Rust TUI jukebox that plays it via mpv with full-text semantic search and a first-run config flow.

---

## Goals

1. Organize every unique track under `~/Music/filtered_lossless/<Artist>/` as **relative symlinks** to untouched originals in `lossless/`. Zero duplication of audio bytes.
2. Standardize every symlinked track's filename (artist, title, audio quality) using streaming-service-style naming derived from FLAC tags.
3. Build a searchable catalog with model-free, multilingual full-text search (Tantivy + Lindera + algorithmic transliteration).
4. Ship a Claude-Code-style TUI that selects artists, builds/shuffles a queue, and plays via mpv with live control.

## Non-Goals

- Renaming or moving the original files in `lossless/`. They are immutable inputs.
- Re-encoding, transcoding, or otherwise modifying audio data.
- Album art / metadata rewriting. Tags are read-only inputs.
- A server or network service. Everything runs in-process inside one binary.
- Lyrics or mood-based recommendation. (No lyrics/genre tags exist in the source.)

## Constraints (verified from the environment)

- Source: `~/Music/lossless/` — 2,128 FLAC files across 38 top-level album/discography folders + a `le/` singles folder (~238 GB).
- All 2,128 FLACs have `ARTIST` and `TITLE` tags. Tags are TIDAL-sourced and rich (ARTIST, ALBUMARTIST, TITLE, ALBUM, TRACKNUMBER, DISCNUMBER, ISRC, TIDAL_TRACK_ID, etc.).
- Tag key casing is inconsistent (`ARTIST=` vs `artist=`). All tag reads must be case-insensitive.
- Audio quality (sample rate, bit depth) is reliably available via `ffprobe` (`bits_per_raw_sample` is non-zero for all sampled files).
- `ARTIST` frequently differs from `ALBUMARTIST` (collabs, remixes, `Various Artists` albums). Multi-artist strings use separators `;` `,` `×` `/` `+` `&`.
- Available tooling: `ffprobe`, `metaflac`, `mpv`, `afplay`, `yq` v4, `jq`, `python3`, `cargo 1.95`, `node`, `deno`. No Ollama, no OpenAI/Voyage/Cohere keys; only Anthropic env keys (Anthropic has no embeddings API).
- Project location: `~/Dev/jukebox/`. Config lives under XDG (`$XDG_CONFIG_HOME`, fallback `~/.config`).
- `$XDG_CONFIG_HOME` is unset on this machine → config path resolves to `~/.config/jukebox/config.yml`.

---

## Architecture

Two languages, one project at `~/Dev/jukebox/`:

- **`scripts/standardize.sh`** (bash) — the indexer. Walks `lossless/`, reads tags + quality, dedupes (keep best copy per song), writes relative symlinks + `catalog.json` + `duplicates.log`. Reads its paths from `config.yml` (via `yq`) or `--source`/`--out` flags / env vars.
- **`jukebox`** (Rust binary) — config management (first-run prompt), search index build (Tantivy) from `catalog.json`, and the TUI + mpv playback.

```
lossless/*.flac  ──►  scripts/standardize.sh  ──►  filtered_lossless/<Artist>/<standardized>.flac  (relative symlinks)
                          │                            filtered_lossless/catalog.json             (track metadata)
                          │                            filtered_lossless/duplicates.log
                          ▼
   config.yml (XDG)  ──►  jukebox (Rust)
                            │  builds  ─► filtered_lossless/search-index/   (Tantivy)
                            │  reads   ─► catalog.json
                            ▼
                       jukebox TUI  ──►  mpv (child process, JSON IPC over Unix socket)
```

1. **`standardize.sh`** (bash): walks `lossless/`, reads tags + quality, dedupes, writes symlinks + `catalog.json` + `duplicates.log`. Deterministic and idempotent.
2. **`catalog.json`**: one record per unique track (see schema below). Authoritative metadata source, produced by the bash script.
3. **`jukebox index`** (Rust subcommand): reads `catalog.json`, computes transliteration variants, builds the Tantivy search index. Run after `standardize.sh` (the CLI can invoke the script then build the index in one `jukebox sync` command).
4. **`jukebox` TUI** (`jukebox` / `jukebox play`): reads catalog + index, drives mpv. Owns config + first-run flow.

The `lossless/` tree is never mutated. Only `filtered_lossless/` is written (symlinks, JSON, index, logs).

---

## Part 0 — Config & first-run (Rust CLI)

### 0.1 Project layout

```
~/Dev/jukebox/
  Cargo.toml
  README.md
  src/
    main.rs            # arg dispatch: index | sync | play | config
    config.rs          # resolve/create config.yml, first-run prompt
    catalog.rs         # parse catalog.json
    search.rs          # Tantivy index build + query + transliteration variants
    player.rs          # mpv IPC child process + fallback to afplay
    tui/               # ratatui app, panes, input
  scripts/
    standardize.sh     # the bash indexer (Part 1)
  specs/
    2026-07-04-filtered-lossless-jukebox-design.md
  tests/
```

### 0.2 `config.yml` location & schema

Resolved via the `dirs` crate: `$XDG_CONFIG_HOME/jukebox/config.yml`, falling back to `~/.config/jukebox/config.yml` when `XDG_CONFIG_HOME` is unset (the case on this machine).

```yaml
# ~/.config/jukebox/config.yml
version: 1
source_dir: /Users/distiled/Music/lossless          # asked on first run
filtered_dir: /Users/distiled/Music/filtered_lossless  # default = sibling of source_dir, editable
player: mpv                                          # mpv | afplay
mpv_socket: /tmp/jukebox-mpv.sock                   # optional override
```

### 0.3 First-run flow

`jukebox` (any subcommand) on startup:
1. Resolves the config path; if `config.yml` does not exist or `source_dir` is empty/unset → **first-run**.
2. Prompts the user for the lossless source directory (interactive text input with default `~/Music/lossless` prefilled; tab/`readline` path completion). Validates: directory exists and contains ≥1 `.flac` file.
3. Derives `filtered_dir` as the `filtered_lossless/` sibling of `source_dir` unless the user overrides it.
4. Writes `config.yml` (creates `~/.config/jukebox/` if needed, mode 0700), then proceeds with the requested subcommand.
5. The user can re-run the prompt any time via `jukebox config` (or `jukebox config set source_dir <path>`).

`standardize.sh` reads the same `config.yml` (via `yq`) so both sides agree on paths after the first-run prompt — no separate config for the script.

---

## Part 1 — Bash standardization script (`scripts/standardize.sh`)

The standardization step is a portable bash script (no Rust required to run it), so it can be run standalone or invoked by the CLI. It uses `metaflac`, `ffprobe`, `jq`, and `yq` (all present).

### 1.0 Inputs & config

`standardize.sh` resolves its working directories in this priority order:
1. Flags: `--source <dir>` and `--out <dir>`.
2. Env vars: `LOSSLESS_SOURCE`, `LOSSLESS_FILTERED`.
3. `config.yml` (read via `yq`): `.source_dir` and `.filtered_dir`.
4. Defaults: `~/Music/lossless` and `~/Music/filtered_lossless`.

It refuses to run if the source directory does not exist or contains no `.flac` files. It refuses to wipe `--out` if the directory does not look like a `filtered_lossless/` layout (must be absent, empty, or contain a `_build.log`/`catalog.json` marker) — safety against clobbering an unrelated directory.

### 1.1 Tag reading

Per FLAC, read (case-insensitive key match) via `metaflac --export-tags-to=-`:
`ARTIST`, `ALBUMARTIST`, `TITLE`, `ALBUM`, `TRACKNUMBER`, `DISCNUMBER`, `ISRC`, `TIDAL_TRACK_ID`, `DATE`.

Audio quality via `ffprobe -v error -show_entries stream=sample_rate,bits_per_raw_sample`:
- `sample_rate` (Hz) → kHz label: 44100→`44.1kHz`, 48000→`48kHz`, 96000→`96kHz`, 192000→`192kHz`.
- `bits_per_raw_sample` → bit depth (`16`, `24`).

### 1.2 Standardized filename

```
{canonicalArtists} - {TITLE} [{bitDepth}bit-{sampleRateKHz}kHz].flac
```

- `canonicalArtists`: the multi-artist `ARTIST` string normalized to `, `-joined form, preserving original primary-artist order. Separators normalized: `;` `,` `×` `×` `/` `+` `&` → `, `.
  - `PinocchioP; Hatsune Miku; Kasane Teto` → `PinocchioP, Hatsune Miku, Kasane Teto`
  - `DAOKO×米津玄師` → `DAOKO, 米津玄師`
- **Filename sanitization:** replace `\0` and `/` → `_`; `:` → `-`; strip leading dots; collapse runs of spaces. Unicode (Japanese) preserved verbatim.
- Examples:
  - `Ado - Freedom [24bit-48kHz].flac`
  - `DAOKO, 米津玄師 - 打上花火 [24bit-96kHz].flac`
  - `Aimer - Thinking Bout' You [16bit-44.1kHz].flac`

The filename's artist string is the **full canonical multi-artist string** (not the per-folder artist), so a collab shows its complete credit inside every artist folder it appears in, and the symlink filename is identical across folders.

### 1.3 Artist splitting → folders

Split `ARTIST` on the same separators into individual artist names (trim each). Each resulting artist name gets a folder under `filtered_lossless/`. A collab track is symlinked into **every** constituent artist's folder.

- `Various Artists` albums: resolve to the per-track `ARTIST` (split). A literal `Various Artists` folder is never created.
- Single-artist tracks: one folder, one symlink.
- An artist folder name is the sanitized single artist name.

### 1.4 Dedup — keep only the best copy per song

**Dedup key (per song):** `canonicalArtistsSorted | normalizedTitle`
- `canonicalArtistsSorted`: the split artist names, lowercased, sorted as a set, joined with `|`. Reordering or separator differences do not create dupes.
- `normalizedTitle`: lowercase, trim, collapse internal whitespace, strip punctuation, NFKC-normalize.

**All copies of a song collapse to one symlink per artist folder.** Quality is NOT part of the key.

**Winner selection (tiebreaker) — highest quality wins, then provenance:**
1. Higher `bit_depth`.
2. Higher `sample_rate`.
3. Has `ISRC` tag.
4. Has `TIDAL_TRACK_ID` tag.
5. More total tags (proxy for richer metadata).
6. Shortest source path (proxy for "canonical" location).

The winner becomes the symlink target. Losers are recorded in `duplicates.log` (source path, quality, reason). Originals in `lossless/` are never touched.

### 1.5 Symlinks

- **Relative** symlinks: `filtered_lossless/<Artist>/<file>.flac` → `../../lossless/<original path>` (computed so the tree stays valid if `~/Music` is moved).
- If a sanitized filename collides within an artist folder (two different songs sanitize to the same name), append ` (2)`, ` (3)`, etc. Logged.

### 1.6 `catalog.json` schema

```json
{
  "version": 1,
  "built_at": "<ISO8601, from bash `date` at index time>",
  "source_root": "/Users/distiled/Music/lossless",
  "tracks": [
    {
      "id": "<stable hash of dedup key>",
      "artists": ["Ado"],
      "primary_artist": "Ado",
      "title": "Freedom",
      "album": "Ado's Best Adobum",
      "track_number": 15,
      "disc_number": 1,
      "duration_sec": 186.1,
      "bit_depth": 24,
      "sample_rate_hz": 48000,
      "isrc": "JPPO02105116",
      "source_path": "lossless/Ado discography/Ado's Best Adobum - Ado/Disc 1/15 - Ado - Freedom.flac",
      "symlink_path": "filtered_lossless/Ado/Ado - Freedom [24bit-48kHz].flac",
      "symlinked_into_artists": ["Ado"]
    }
  ]
}
```

Collab example: a track by `DAOKO, 米津玄師` has `symlinked_into_artists: ["DAOKO","米津玄師"]` and two symlinks on disk, both with the same canonical filename.

### 1.7 Idempotency

`standardize.sh` wipes `filtered_lossless/` (symlinks + logs) and rebuilds fully from `lossless/` + tag data; `catalog.json` is regenerated. A `--incremental` flag may compare file mtimes/inodes to skip unchanged probes in future iterations, but the first shipped version rebuilds fully (the whole catalog probes in well under a minute given 2,128 files). The Tantivy index is rebuilt separately by `jukebox index` from `catalog.json` (also idempotent: it recreates `search-index/`).

---

## Part 2 — Search (Tantivy, model-free, multilingual)

### 2.1 Engine

[Tantivy](https://github.com/quickwit-oss/tantivy) — an in-process Rust full-text search library (Lucene equivalent: BM25 ranking, tokenizers, fuzzy/phrase queries). No server, no model, no API key, fully offline.

### 2.2 Japanese tokenization

[Lindera](https://github.com/lindera/lindera) via `lindera-tantivy` (v4.0.0, actively maintained) provides morphological CJK tokenization so `ブルーバード` is tokenized properly rather than as an opaque byte blob.

### 2.3 Cross-script matching (the "feels semantic" part, no ML)

For every title/artist containing kana or kanji, `jukebox index` (Rust) generates variant strings via a transliteration crate (`wana_kana`-style, pure Rust) and indexes them into variant fields:
- katakana ↔ hiragana (`ブルーバード` ↔ `ぶるーばーど`)
- kana → romaji (`ぶるーばーど` → `burubado`)

These variants are **searchable text only, never displayed.** Typing `blue bird`, `burubado`, or Japanese finds the matched track regardless of script. Tantivy fuzzy matching (edit distance) catches typos.

### 2.4 Tantivy schema

| field | type | tokenizer | purpose |
|---|---|---|---|
| `id` | stored | — | link to catalog record |
| `artists` | text | Lindera | artist match |
| `title` | text | Lindera | primary title |
| `title_variants` | text | lowercase | romaji/kana cross-script |
| `artist_variants` | text | lowercase | romaji/kana for artists |
| `album` | text | Lindera | album match |
| `quality` | string (facet) | — | filter by bit-depth/sample-rate |

**Ranking:** BM25 (Tantivy default) across `title` (boosted ×2), `title_variants` (×1.5), `artists`, `artist_variants`, `album`.

### 2.5 Index location

`filtered_lossless/search-index/` — on-disk Tantivy index (a few MB), built by the indexer alongside `catalog.json`, rebuilt on re-index.

---

## Part 3 — `jukebox` TUI (Rust + ratatui + mpv)

### CLI commands

| command | what it does |
|---|---|
| `jukebox` / `jukebox play` | launch the TUI (runs first-run config flow if config.yml missing) |
| `jukebox sync` | run `standardize.sh` then `jukebox index` (full rebuild of symlinks + catalog + search index) |
| `jukebox index` | build/rebuild the Tantivy search index from `catalog.json` |
| `jukebox config` | re-run the directory prompt; `jukebox config set <key> <val>` to edit a field |
| `jukebox search <query>` | one-shot CLI search (prints ranked results; useful for scripting) |

### 3.1 Aesthetic

Claude-Code-style: dark background, warm amber/orange accent for active selection, box-drawing borders, three panes, bottom input/status bar.

### 3.2 Layout

```
┌─ Artists ──┐┌─ Search: blue bird_______________ ─┐┌─ Queue ────┐
│> Ado       ││ 91% ブルーバード — Ikimono-gakari    ││ 1. ...     │
│  Aimer     ││ 87% Blue Bird — Naruto Shippuden OP ││ 2. ...     │
│  Eve       ││ 79% Blueberry — ...                 ││ ▶3. ...    │
│  ...       ││                                     ││            │
└────────────┘└─────────────────────────────────────┘└────────────┘
│ /:search  Tab:panes  space:multi-select  enter:enqueue  s:shuffle  ␣:play  n:next  ←/→:seek │
```

Three panes: **Artists** (left), **Search/Results** (center), **Queue** (right). `Tab` cycles focus. Bottom bar shows keybindings + now-playing (title, artist, quality, position/duration).

### 3.3 Flows

- **Artist pane:** `↑/↓` move cursor; `space`/`enter` enqueue all tracks of the selected artist; `a` enqueue all artists; `/` live-filters the artist list (substring + normalized).
- **Search pane** (`/` or `Tab` to focus): type a query; results stream ranked by BM25 across the whole catalog. `space` multi-select; `enter` enqueues selected tracks (or, with a toggle, their artists). Each result line: `score% title — artist (album) [quality]`.
- **Queue pane:** `↑/↓` move; `enter` play-now (jump); `r` remove; `s` shuffle (Fisher–Yates, seeded RNG — deterministic per session, re-shufflable); `S` shuffle + jump to random start; `c` clear.

### 3.4 Playback via mpv

- mpv launched as a child: `mpv --no-video --input-ipc-server=<socket> --gapless audio <first track>`.
- TUI communicates over the Unix socket using JSON IPC commands: `loadfile`, `set pause true/false`, `seek <sec>`, `playlist-next`, `playlist-play-index`.
- The TUI owns queue ordering; on mpv's `end-file` event it feeds the next track (`loadfile append-play` / explicit `playlist-next`).
- Status bar polls `time-pos` + `duration` from the socket.
- Quality "defaults to highest" is already guaranteed by the indexer (only the best copy per song is in the catalog), so the TUI never has to choose between qualities.

### 3.5 Keybindings (summary)

| key | action |
|---|---|
| `Tab` | cycle pane focus |
| `↑` `↓` | move cursor in focused pane |
| `/` | focus search / start inline filter |
| `space` | multi-select (search) / enqueue artist (artist pane) |
| `enter` | play-now (queue) / enqueue selected (search) |
| `s` / `S` | shuffle queue / shuffle + jump |
| `r` | remove from queue |
| `c` | clear queue |
| `␣` (space, in queue) | play/pause |
| `n` / `p` | next / previous track |
| `←` `→` | seek ±5s |
| `q` | quit (kills mpv) |
| `Q` | quit, leave mpv playing (detached) |

---

## Error Handling

- **Missing/unreadable tags:** track still indexed using filename-derived fallbacks (`[unknown artist]` / `[unknown title]`); flagged in `_build.log`. Symlink name uses placeholders rather than skipping.
- **`ffprobe` failure:** skip track, log, continue.
- **Symlink target missing at play time:** TUI marks the track dead, skips to next, logs.
- **mpv socket unavailable / mpv missing:** fall back to launching `afplay` per-track (no seek), with a warning in the status bar.
- **Tantivy index missing on TUI launch:** prompt to run `jukebox index`.
- **`filtered_lossless/` already populated:** indexer wipes and rebuilds; refuses to run if it detects a non-`filtered_lossless` layout (safety against wiping an unrelated dir).

## Testing

- **`standardize.sh` unit checks (shell tests, e.g. `bats`):** tag normalization (case-insensitive), artist splitting + separator normalization, filename sanitization (every forbidden char), dedup key construction, winner-selection tiebreaker ordering, relative-symlink path computation. Helpers tested as pure functions via small `awk`/`sed` pipelines or sourced functions.
- **`standardize.sh` integration test:** a small fixture `lossless/` copy with (a) a collab, (b) a `Various Artists` album, (c) a song duplicated at two qualities — assert the exact `filtered_lossless/` tree, `catalog.json`, and `duplicates.log`.
- **Rust `search.rs` / `catalog.rs` unit tests:** catalog parsing; cross-script variant generation; cross-script matches (`blue bird` → `ブルーバード`; `burubado` → Japanese title); fuzzy typo tolerance; ranking (exact title beats album match).
- **Rust `config.rs` tests:** first-run creates config.yml with correct path resolution (XDG unset → `~/.config`); rejects non-existent/empty source dir.
- **Search tests:** cross-script matches (`blue bird` → `ブルーバード`; `burubado` → Japanese title); fuzzy typo tolerance; ranking (exact title beats album match).
- **TUI tests (no mpv):** a `Player` trait with a stub implementation; queue operations (enqueue, deterministic shuffle, skip, seek-command construction) tested via the stub.
- **mpv integration:** a manual/smoke test path that boots mpv and asserts `loadfile` + `end-file` flow; not part of the unit suite.

## Open Questions / Future

- `--incremental` indexing (mtime/inode-based skip) — deferred; first version is full rebuild.
- Album pane / album-oriented browsing — deferred; v1 is artist + search + queue.
- ReplayGain normalization flag for mpv — deferred; tags have `REPLAYGAIN_*` but application is optional.

## Out of Scope (explicit)

- Embedding/ML search. Replaced by Tantivy + transliteration per user decision.
- Hosted search APIs (Elasticsearch server, OpenAI/Voyage embeddings).
- Mutating `lossless/` originals in any way.
