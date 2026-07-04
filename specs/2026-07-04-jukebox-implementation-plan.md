# Jukebox Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a working `jukebox` CLI (Rust) plus a `standardize.sh` bash indexer that, together, deduplicate a FLAC library into a symlinked, artist-organized, searchable, mpv-driven jukebox.

**Architecture:** A bash script (`scripts/standardize.sh`) walks the immutable `lossless/` tree, reads FLAC tags + audio quality, dedupes keeping the best copy per song, and emits relative symlinks + `catalog.json` + `duplicates.log` under `filtered_lossless/`. A Rust binary (`jukebox`) owns config/first-run, builds a Tantivy full-text index (with Lindera Japanese tokenization + kana↔romaji transliteration variants) from `catalog.json`, and runs a ratatui TUI that drives mpv over a Unix socket for playback.

**Tech Stack:** bash + `metaflac`/`ffprobe`/`jq`/`yq` (indexer); Rust 1.95, `tantivy 0.26`, `lindera 4.0` + `lindera-tantivy 4.0` (`embed-ipadic`), `ratatui 0.30`, `crossterm 0.29`, `clap 4.6`, `dirs 6.0`, `wana_kana 5.0`, `serde`/`serde_json`, `anyhow`, `bats` (shell tests).

## Global Constraints

- `lossless/` originals are **immutable inputs** — never renamed, moved, re-encoded, or re-tagged. Only `filtered_lossless/` is written.
- FLAC tags are read case-insensitively (`ARTIST=` vs `artist=` both occur).
- Audio quality from `ffprobe` `bits_per_raw_sample` + `sample_rate`.
- Dedup key = `canonicalArtistsSorted | normalizedTitle` (quality NOT in key). Winner tiebreaker: bit_depth → sample_rate → has ISRC → has TIDAL_TRACK_ID → more tags → shortest path.
- Standardized filename: `{canonicalArtists} - {TITLE} [{bitDepth}bit-{sampleRateKHz}kHz].flac`.
- Artist separators to normalize: `;` `,` `×` `/` `+` `&` → `, ` (also full-width `×` U+00D7 and `×` U+00D7 variants).
- Config path: `$XDG_CONFIG_HOME/jukebox/config.yml`, fallback `~/.config/jukebox/config.yml` (`$XDG_CONFIG_HOME` is unset on this machine → fallback wins).
- `standardize.sh` resolves paths by priority: `--source`/`--out` flags → `LOSSLESS_SOURCE`/`LOSSLESS_FILTERED` env → `config.yml` (via `yq`) → defaults `~/Music/lossless` / `~/Music/filtered_lossless`.
- Symlinks are **relative** (`../../lossless/...`) so the tree survives `~/Music` being moved.
- "Quality defaults to highest" is enforced by the indexer (only the best copy per song enters the catalog); the TUI never chooses between qualities.
- Project root: `~/Dev/jukebox/`. Tests run with `cargo test` and `bats`.

---

## File Structure

```
~/Dev/jukebox/
  Cargo.toml                      # workspace + deps (Task 1)
  README.md                        # usage (Task 15)
  .gitignore                       # target/, filtered_lossless/, *.sock
  scripts/
    standardize.sh                 # bash indexer (Tasks 2-5)
    lib/normalize.bash              # pure helper functions, sourced (Task 2)
    test/
      helpers.bash                 # bats helpers + fixture builder
      normalize.bats               # Task 2
      dedup.bats                   # Task 3
      integration.bats             # Task 4
      inputs.bats                  # Task 5
  src/
    main.rs                        # clap dispatch (Task 6)
    config.rs                      # Config + first-run (Task 1)
    catalog.rs                     # parse catalog.json (Task 7)
    translit.rs                    # kana variants (Task 8)
    search.rs                      # Tantivy build+query (Tasks 9-10)
    player.rs                      # Player trait + mpv + afplay + stub (Task 11)
    tui/
      mod.rs                       # App + event loop (Task 12)
      view.rs                      # draw panes (Task 12)
      queue.rs                     # queue ops + deterministic shuffle (Task 11/12)
  tests/
    config.rs                      # Task 1
    catalog.rs                     # Task 7
    translit.rs                    # Task 8
    search.rs                      # Tasks 9-10
    player.rs                      # Task 11
    tui.rs                         # Tasks 12-13
```

Files are split by responsibility. `translit.rs` is separated from `search.rs` so the pure-string logic is unit-testable without a Tantivy index. `tui/` is a module directory so view/state/queue each stay focused.

---

## Task 1: Rust scaffold + `config.rs` (load/save/path resolution)

**Files:**
- Create: `Cargo.toml`, `src/main.rs`, `src/config.rs`, `.gitignore`, `tests/config.rs`
- Produces: `Config` struct, `config_path()`, `Config::load()`, `Config::save()`, `Config::default_for(source_dir)`, `validate_source_dir()`.

**Interfaces:**
- Produces (used by Tasks 6, 7, 9, 11):
  ```rust
  // src/config.rs
  pub struct Config {
      pub version: u32,
      pub source_dir: PathBuf,
      pub filtered_dir: PathBuf,
      pub player: PlayerKind,
      pub mpv_socket: PathBuf,
  }
  pub enum PlayerKind { Mpv, Afplay }
  pub fn config_path() -> PathBuf;                 // $XDG_CONFIG_HOME/jukebox/config.yml or ~/.config/jukebox/config.yml
  impl Config {
      pub fn default_for(source_dir: PathBuf) -> Self;
      pub fn load() -> Result<Option<Config>>;     // Ok(None) if file missing
      pub fn save(&self) -> Result<()>;            // writes dir+file mode 0700
  }
  pub fn validate_source_dir(p: &Path) -> Result<()>; // exists + >=1 .flac
  ```

- [ ] **Step 1: Write `Cargo.toml`**

```toml
[package]
name = "jukebox"
version = "0.1.0"
edition = "2021"

[dependencies]
anyhow = "1.0"
clap = { version = "4.6", features = ["derive"] }
crossterm = "0.29"
dirs = "6.0"
ratatui = "0.30"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tantivy = "0.26"
lindera = { version = "4.0", features = ["embed-ipadic"] }
lindera-tantivy = { version = "4.0", features = ["embed-ipadic"] }
wana_kana = "5.0"
unicode-normalization = "0.1"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: Write `.gitignore`**

```
/target
filtered_lossless/
*.sock
```

- [ ] **Step 3: Write minimal `src/main.rs` placeholder**

```rust
mod config;
fn main() -> anyhow::Result<()> { Ok(()) }
```

- [ ] **Step 4: Write the failing test `tests/config.rs`**

```rust
use jukebox::config::{config_path, Config, validate_source_dir};
use std::fs;
use tempfile::tempdir;

// We can't fully control $XDG_CONFIG_HOME here, so test save/load roundtrip
// via a direct path by using the public save/load that target config_path().
// Instead, test the bits we can control deterministically.

#[test]
fn default_for_sets_filtered_sibling() {
    let cfg = Config::default_for("/Users/distiled/Music/lossless".into());
    assert_eq!(cfg.source_dir, std::path::Path::new("/Users/distiled/Music/lossless"));
    assert_eq!(cfg.filtered_dir, std::path::Path::new("/Users/distiled/Music/filtered_lossless"));
    assert_eq!(cfg.version, 1);
}

#[test]
fn validate_rejects_missing_dir() {
    let r = validate_source_dir(std::path::Path::new("/nonexistent/xyz"));
    assert!(r.is_err());
}

#[test]
fn validate_rejects_dir_without_flac() {
    let d = tempdir().unwrap();
    fs::write(d.path().join("not-audio.txt"), b"x").unwrap();
    assert!(validate_source_dir(d.path()).is_err());
}

#[test]
fn save_then_load_roundtrip() {
    // Use a temp HOME so config_path() lands in our tempdir.
    let tmp = tempdir().unwrap();
    // HOME-based fallback is what dirs::config_dir() uses on macOS when XDG unset.
    std::env::set_var("HOME", tmp.path());
    std::env::remove_var("XDG_CONFIG_HOME");
    let cfg = Config::default_for(tmp.path().join("lossless"));
    cfg.save().unwrap();
    let loaded = Config::load().unwrap().expect("config should exist");
    assert_eq!(loaded.source_dir, cfg.source_dir);
    assert_eq!(loaded.filtered_dir, cfg.filtered_dir);
    let p = config_path();
    assert!(p.starts_with(tmp.path()), "config_path {p:?} should be under temp HOME");
    let meta = fs::metadata(p).unwrap();
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(meta.permissions().mode() & 0o777, 0o700);
}

#[test]
fn load_returns_none_when_missing() {
    let tmp = tempdir().unwrap();
    std::env::set_var("HOME", tmp.path());
    std::env::remove_var("XDG_CONFIG_HOME");
    assert!(Config::load().unwrap().is_none());
}
```

- [ ] **Step 5: Run the test to verify it fails**

Run: `cargo test --test config`
Expected: FAIL with `error[E0432]` (module `jukebox::config` symbols not found / unresolved).

- [ ] **Step 6: Implement `src/config.rs`**

```rust
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlayerKind {
    Mpv,
    Afplay,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    pub source_dir: PathBuf,
    pub filtered_dir: PathBuf,
    pub player: PlayerKind,
    pub mpv_socket: PathBuf,
}

/// Resolve the config file path.
/// Honors `$XDG_CONFIG_HOME`, else falls back to `~/.config` (via `dirs`).
pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::config_dir())
        .unwrap_or_else(|| PathBuf::from("/tmp/.config"));
    base.join("jukebox").join("config.yml")
}

impl Config {
    /// Derive a Config from a source dir: filtered_dir is the `filtered_lossless`
    /// sibling of `source_dir`.
    pub fn default_for(source_dir: PathBuf) -> Self {
        let filtered_dir = source_dir
            .parent()
            .map(|p| p.join("filtered_lossless"))
            .unwrap_or_else(|| PathBuf::from("filtered_lossless"));
        Config {
            version: 1,
            source_dir,
            filtered_dir,
            player: PlayerKind::Mpv,
            mpv_socket: PathBuf::from("/tmp/jukebox-mpv.sock"),
        }
    }

    pub fn load() -> Result<Option<Self>> {
        let p = config_path();
        if !p.exists() {
            return Ok(None);
        }
        let text = fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
        let cfg: Config = serde_yaml_compat(&text)
            .with_context(|| format!("parsing {}", p.display()))?;
        Ok(Some(cfg))
    }

    pub fn save(&self) -> Result<()> {
        let p = config_path();
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
            // 0700 on the dir
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).ok();
        }
        let text = format_yaml(self)?;
        fs::write(&p, text).with_context(|| format!("writing {}", p.display()))?;
        fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).ok();
        Ok(())
    }
}

/// Validate that a source dir exists and contains at least one .flac file.
pub fn validate_source_dir(p: &Path) -> Result<()> {
    if !p.is_dir() {
        return Err(anyhow!("source dir does not exist: {}", p.display()));
    }
    let has_flac = walkdir_has_flac(p);
    if !has_flac {
        return Err(anyhow!("source dir contains no .flac files: {}", p.display()));
    }
    Ok(())
}

fn walkdir_has_flac(root: &Path) -> bool {
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = match fs::read_dir(&d) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|s| s.to_str()).map(|s| s.eq_ignore_ascii_case("flac")).unwrap_or(false) {
                return true;
            }
        }
    }
    false
}

// ---- minimal YAML (de)serialization without pulling serde_yaml ----
// We hand-roll a tiny reader/writer for our 5-key flat config to avoid an extra
// dependency that is in maintenance mode. Format is fixed and simple.

fn serde_yaml_compat(text: &str) -> Result<Config> {
    let mut version = 1u32;
    let mut source_dir = PathBuf::new();
    let mut filtered_dir = PathBuf::new();
    let mut player = PlayerKind::Mpv;
    let mut mpv_socket = PathBuf::from("/tmp/jukebox-mpv.sock");
    for line in text.lines() {
        let line = line.split('#').next().unwrap().trim();
        if line.is_empty() { continue; }
        let (k, v) = match line.split_once(':') { Some(kv) => kv, None => continue };
        let k = k.trim();
        let v = v.trim().trim_matches('"');
        match k {
            "version" => version = v.parse().unwrap_or(1),
            "source_dir" => source_dir = PathBuf::from(v),
            "filtered_dir" => filtered_dir = PathBuf::from(v),
            "player" => player = if v == "afplay" { PlayerKind::Afplay } else { PlayerKind::Mpv },
            "mpv_socket" => mpv_socket = PathBuf::from(v),
            _ => {}
        }
    }
    Ok(Config { version, source_dir, filtered_dir, player, mpv_socket })
}

fn format_yaml(c: &Config) -> Result<String> {
    let player = match c.player { PlayerKind::Mpv => "mpv", PlayerKind::Afplay => "afplay" };
    Ok(format!(
        "# jukebox config — written by `jukebox`\n\
         version: {v}\n\
         source_dir: \"{s}\"\n\
         filtered_dir: \"{f}\"\n\
         player: {p}\n\
         mpv_socket: \"{m}\"\n",
        v = c.version,
        s = c.source_dir.display(),
        f = c.filtered_dir.display(),
        p = player,
        m = c.mpv_socket.display(),
    ))
}
```

- [ ] **Step 7: Make `config` module public in `src/main.rs`**

```rust
pub mod config;

fn main() -> anyhow::Result<()> {
    Ok(())
}
```

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test --test config`
Expected: PASS — all 5 tests green.

- [ ] **Step 9: Commit**

```bash
cd ~/Dev/jukebox
git init 2>/dev/null; git add -A
git commit -m "feat(config): config struct, path resolution, save/load with 0700"
```

---

## Task 2: `standardize.sh` helper library — tag reading, artist splitting, filename sanitization

**Files:**
- Create: `scripts/lib/normalize.bash`, `scripts/test/helpers.bash`, `scripts/test/normalize.bats`
- Produces (sourced shell functions): `normalize_artist_string`, `split_artists`, `sanitize_filename`, `khz_label`, `read_tag` (case-insensitive), `normalized_title`, `canonical_artists_sorted`.

**Interfaces:**
- Consumes: `metaflac`, `ffprobe`, `jq`.
- Produces (used by Tasks 3-5): the functions above, all pure (no side effects), sourced via `source scripts/lib/normalize.bash`.

- [ ] **Step 1: Write `scripts/test/helpers.bash`**

```bash
# Common bats helpers. Source from each .bats file via:
#   load "$(dirname "$BATS_TEST_FILENAME")/helpers.bash"
PROJECT_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
source "$PROJECT_ROOT/scripts/lib/normalize.bash"

# mkflac <dir> <filename> <tagK=tagV ...>  — create a tiny placeholder file with
# FLAC-ish Vorbis-comment tags written via metaflac. Used by integration tests.
mkflac() {
  local dir="$1" file="$2"; shift 2
  mkdir -p "$dir"
  local path="$dir/$file"
  # 1-byte placeholder; metaflac can't tag a non-flac, so we synthesize via ffmpeg if available
  if command -v ffmpeg >/dev/null; then
    ffmpeg -loglevel quiet -y -f lavfi -i anullsrc=r=44100:cl=mono -t 0.1 "$path" >/dev/null 2>&1
  else
    : > "$path"   # fallback; tag reads will be empty
  fi
  if command -v metaflac >/dev/null; then
    for kv in "$@"; do metaflac --remove-tag "${kv%%=*}" "$path" 2>/dev/null; metaflac --set-tag "$kv" "$path" 2>/dev/null; done
  fi
}
```

- [ ] **Step 2: Write the failing test `scripts/test/normalize.bats`**

```bats
#!/usr/bin/env bats
load helpers.bash

@test "normalize_artist_string joins separators with ', '" {
  [ "$(normalize_artist_string 'PinocchioP; Hatsune Miku; Kasane Teto')" = "PinocchioP, Hatsune Miku, Kasane Teto" ]
}

@test "normalize_artist_string handles full-width multiplication sign" {
  [ "$(normalize_artist_string 'DAOKO×米津玄師')" = "DAOKO, 米津玄師" ]
}

@test "normalize_artist_string handles ampersand and plus" {
  [ "$(normalize_artist_string 'A & B + C')" = "A, B, C" ]
}

@test "split_artists emits one line per artist trimmed" {
  out="$(split_artists 'A; B , C')"
  [ "$out" = $'A\nB\nC' ]
}

@test "split_artists drops empty parts and Various Artists" {
  out="$(split_artists 'Various Artists; Real Artist')"
  [ "$out" = "Real Artist" ]
}

@test "sanitize_filename replaces slashes colons and leading dots" {
  [ "$(sanitize_filename 'A/B: C')" = "A-B- C" ]
}

@test "sanitize_filename collapses spaces" {
  [ "$(sanitize_filename 'A   B')" = "A B" ]
}

@test "sanitize_filename preserves unicode" {
  [ "$(sanitize_filename '米津玄師')" = "米津玄師" ]
}

@test "khz_label converts 44100 -> 44.1kHz" {
  [ "$(khz_label 44100)" = "44.1kHz" ]
  [ "$(khz_label 48000)" = "48kHz" ]
  [ "$(khz_label 96000)" = "96kHz" ]
  [ "$(khz_label 192000)" = "192kHz" ]
}

@test "normalized_title lowercases strips punctuation collapses whitespace" {
  [ "$(normalized_title "  Blue-Bird!! " )" = "bluebird" ]
}

@test "canonical_artists_sorted sorts lowercased joined with pipe" {
  [ "$(canonical_artists_sorted 'B; A')" = "a|b" ]
}

@test "read_tag is case-insensitive" {
  skip_if_no_metaflac
  d="$(mktemp -d)"
  mkflac "$d" "t.flac" "artist=Ado" "TITLE=Freedom"
  [ "$(read_tag "$d/t.flac" ARTIST)" = "Ado" ]
  [ "$(read_tag "$d/t.flac" title)" = "Freedom" ]
  rm -rf "$d"
}

skip_if_no_metaflac() {
  command -v metaflac >/dev/null || skip "metaflac not installed"
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cd ~/Dev/jukebox && bats scripts/test/normalize.bats`
Expected: FAIL — `command not found` for the helper functions.

- [ ] **Step 4: Implement `scripts/lib/normalize.bash`**

```bash
#!/usr/bin/env bash
# Pure helper functions for standardize.sh. Sourcing has no side effects.
# All functions print to stdout; no global mutation.

# Replace every artist separator with ', ' and trim/collapse whitespace.
# Separators (spec §1.2): ; , × / + &  (and full-width × U+00D7, × U+00D7).
normalize_artist_string() {
  local s="$1"
  # First split into newline-joined parts, then re-join with ', '.
  split_artists "$s" | paste -sd ', ' -
}

# Print one artist per line, trimmed. Drops empty parts and 'Various Artists'.
split_artists() {
  local s="$1"
  # Normalize all separators to newline.
  printf '%s' "$s" \
    | sed -E $'s/[;,×\\/+]([^&]|$)/\\n\\1/g; s/&/\\n/g' \
    | sed -E 's/×/\n/g' \
    | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//' \
    | grep -v '^$' \
    | grep -vix 'Various Artists'
}

# Sanitize a string for use as a filename. Replace \0 and / with _, : with -,
# strip leading dots, collapse whitespace runs. Preserve unicode.
sanitize_filename() {
  printf '%s' "$1" \
    | tr '\0' '_' \
    | tr '/' '_' \
    | tr ':' '-' \
    | sed -E 's/^[.]+//' \
    | sed -E 's/[[:space:]]+/ /g' \
    | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//'
}

# Convert a sample rate in Hz to a kHz label.
khz_label() {
  local hz="$1"
  awk -v hz="$hz" 'BEGIN{
    if (hz % 1000 == 0) printf "%gkHz", hz/1000;
    else printf "%.1fkHz", hz/1000;
  }'
}

# Read a Vorbis comment tag case-insensitively via metaflac.
# Usage: read_tag <flac> <TAGNAME>
read_tag() {
  local flac="$1" tag="$2"
  metaflac --export-tags-to=- "$flac" 2>/dev/null \
    | awk -F'=' -v want="$(tolower "$tag")" '
        { gsub(/\r$/,""); k=tolower($1); v=substr($0,index($0,"=")+1);
          if (k==want) print v }' \
    | head -1
}

# Normalize a title for the dedup key: lowercase, strip punctuation,
# collapse whitespace. NFKC is applied at the caller via python if needed.
normalized_title() {
  printf '%s' "$1" | tr '[:upper:]' '[:lower:]' \
    | sed -E 's/[[:punct:][:space:]]+/ /g' \
    | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//' \
    | tr -d ' '
}

# Build canonicalArtistsSorted: split, lowercase, sort -u, join with '|'.
canonical_artists_sorted() {
  split_artists "$1" | tr '[:upper:]' '[:lower:]' | sort -u | paste -sd '|' -
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd ~/Dev/jukebox && bats scripts/test/normalize.bats`
Expected: PASS — all tests green (the metaflac test may skip if ffmpeg absent; that's acceptable, but verify the rest pass).

- [ ] **Step 6: Commit**

```bash
cd ~/Dev/jukebox
git add scripts/lib/normalize.bash scripts/test/
git commit -m "feat(standardize): pure normalize/tag helpers + bats tests"
```

---

## Task 3: `standardize.sh` dedup logic — key + winner selection

**Files:**
- Create: `scripts/test/dedup.bats`
- Modify: `scripts/lib/normalize.bash` (append `dedup_key`, `pick_winner`)
- Produces: `dedup_key`, `pick_winner` (reads candidate records on stdin, prints the winning record's JSON-ish line).

**Interfaces:**
- Consumes: `normalized_title`, `canonical_artists_sorted` from Task 2.
- Produces (used by Task 4): `dedup_key`, `pick_winner`.

- [ ] **Step 1: Write the failing test `scripts/test/dedup.bats`**

```bats
#!/usr/bin/env bats
load helpers.bash

@test "dedup_key is stable across separator differences" {
  a="$(dedup_key 'B; A' 'Blue Bird')"
  b="$(dedup_key 'A & B' 'Blue-Bird')"
  [ "$a" = "$b" ]
}

@test "dedup_key differs for different titles" {
  [ "$(dedup_key 'Ado' 'Freedom')" != "$(dedup_key 'Ado' 'Usse')" ]
}

# Candidate format (TSV): bit_depth \t sample_rate \t isrc \t tidal \t ntags \t path
@test "pick_winner prefers higher bit_depth" {
  c1=$'16\t48000\t\t\t5\ta/low.flac'
  c2=$'24\t48000\t\t\t5\tb/high.flac'
  win="$(printf '%s\n%s\n' "$c1" "$c2" | pick_winner)"
  [ "$win" = "$c2" ]
}

@test "pick_winner tiebreaks on sample_rate when bit_depth equal" {
  c1=$'24\t48000\t\t\t5\ta.flac'
  c2=$'24\t96000\t\t\t5\tb.flac'
  win="$(printf '%s\n%s\n' "$c1" "$c2" | pick_winner)"
  [ "$win" = "$c2" ]
}

@test "pick_winner prefers has_isrc when quality equal" {
  c1=$'24\t96000\t\t\t5\ta.flac'
  c2=$'24\t96000\tJPPO02105116\t\t5\tb.flac'
  win="$(printf '%s\n%s\n' "$c1" "$c2" | pick_winner)"
  [ "$win" = "$c2" ]
}

@test "pick_winner prefers has_tidal when isrc equal" {
  c1=$'24\t96000\tJPPO02105116\t\t5\ta.flac'
  c2=$'24\t96000\tJPPO02105116\tT123\t5\tb.flac'
  win="$(printf '%s\n%s\n' "$c1" "$c2" | pick_winner)"
  [ "$win" = "$c2" ]
}

@test "pick_winner prefers more tags when tidal equal" {
  c1=$'24\t96000\tJPPO02105116\tT1\t5\ta.flac'
  c2=$'24\t96000\tJPPO02105116\tT1\t9\tb.flac'
  win="$(printf '%s\n%s\n' "$c1" "$c2" | pick_winner)"
  [ "$win" = "$c2" ]
}

@test "pick_winner prefers shortest path as final tiebreak" {
  c1=$'24\t96000\tJPPO02105116\tT1\t9\tdeeply/nested/path/a.flac'
  c2=$'24\t96000\tJPPO02105116\tT1\t9\tshort/b.flac'
  win="$(printf '%s\n%s\n' "$c1" "$c2" | pick_winner)"
  [ "$win" = "$c2" ]
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd ~/Dev/jukebox && bats scripts/test/dedup.bats`
Expected: FAIL — `dedup_key`/`pick_winner` not found.

- [ ] **Step 3: Append dedup functions to `scripts/lib/normalize.bash`**

```bash
# Build the dedup key: canonicalArtistsSorted | normalizedTitle.
dedup_key() {
  local artists="$1" title="$2"
  printf '%s|%s' "$(canonical_artists_sorted "$artists")" "$(normalized_title "$title")"
}

# Read candidate lines on stdin (TSV):
#   bit_depth \t sample_rate \t isrc \t tidal_id \t n_tags \t path
# Print the winning line per spec §1.4 tiebreaker:
#   bit_depth desc, sample_rate desc, has_isrc desc, has_tidal desc,
#   n_tags desc, path-length asc.
pick_winner() {
  awk -F'\t' '
    function score(c,    isrc, tidal, plen) {
      isrc  = (c[3] != "" ? 1 : 0);
      tidal = (c[4] != "" ? 1 : 0);
      plen  = length(c[6]);
      return sprintf("%05d|%010d|%d|%d|%05d|%09d",
                     c[1]+0, c[2]+0, isrc, tidal, c[5]+0, 1000000000 - plen);
    }
    {
      n = split($0, c, "\t");
      s = score(c);
      if (s > best) { best = s; bestline = $0; }
    }
    END { print bestline }
  '
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd ~/Dev/jukebox && bats scripts/test/dedup.bats`
Expected: PASS — all 7 tests green.

- [ ] **Step 5: Commit**

```bash
cd ~/Dev/jukebox
git add scripts/lib/normalize.bash scripts/test/dedup.bats
git commit -m "feat(standardize): dedup key + winner-selection tiebreaker"
```

---

## Task 4: `standardize.sh` main script — symlink creation, `catalog.json`, `duplicates.log`, idempotency

**Files:**
- Create: `scripts/standardize.sh`, `scripts/test/integration.bats`
- Produces: a runnable `standardize.sh` that emits `filtered_lossless/<Artist>/` symlinks + `catalog.json` + `duplicates.log`.

**Interfaces:**
- Consumes: every function from `scripts/lib/normalize.bash` (Tasks 2-3), plus `metaflac`/`ffprobe`/`jq`.
- Produces: `catalog.json` (schema below, consumed by Rust `catalog.rs` Task 7) and the on-disk symlink tree (consumed by `player.rs` Task 11).

- [ ] **Step 1: Write the failing integration test `scripts/test/integration.bats`**

```bats
#!/usr/bin/env bats
load helpers.bash

setup() {
  skip_if_no_metaflac   # from normalize.bats helpers
  ROOT="$(mktemp -d)"
  SRC="$ROOT/lossless"
  OUT="$ROOT/filtered_lossless"
}

teardown() { rm -rf "$ROOT"; }

@test "indexes single-artist track into one folder with standardized name" {
  mkflac "$SRC/Album" "01 - Freedom.flac" \
    "ARTIST=Ado" "TITLE=Freedom" "ALBUM=Ado's Best" \
    "TRACKNUMBER=1" "ISRC=JPPO02105116"
  run "$PROJECT_ROOT/scripts/standardize.sh" --source "$SRC" --out "$OUT"
  [ "$status" -eq 0 ]
  [ -L "$OUT/Ado/Ado - Freedom [16bit-44.1kHz].flac" ]
}

@test "collab track is symlinked into every artist folder with the same filename" {
  mkflac "$SRC" "t.flac" "ARTIST=DAOKO×米津玄師" "TITLE=打上花火" "ALBUM=Fireworks"
  run "$PROJECT_ROOT/scripts/standardize.sh" --source "$SRC" --out "$OUT"
  [ "$status" -eq 0 ]
  [ -L "$OUT/DAOKO/DAOKO, 米津玄師 - 打上花火 [16bit-44.1kHz].flac" ]
  [ -L "$OUT/米津玄師/DAOKO, 米津玄師 - 打上花火 [16bit-44.1kHz].flac" ]
}

@test "duplicate at lower quality is dropped and logged" {
  mkflac "$SRC/A" "hifi.flac" "ARTIST=Ado" "TITLE=Freedom" "ISRC=X1"
  mkflac "$SRC/B" "lofi.flac"  "ARTIST=Ado" "TITLE=Freedom"
  run "$PROJECT_ROOT/scripts/standardize.sh" --source "$SRC" --out "$OUT"
  [ "$status" -eq 0 ]
  # winner symlink exists; only one symlink in the artist folder
  [ "$(find "$OUT/Ado" -type L | wc -l | tr -d ' ')" = "1" ]
  grep -q "lofi.flac" "$OUT/duplicates.log"
}

@test "catalog.json is valid JSON with expected fields" {
  mkflac "$SRC" "t.flac" "ARTIST=Ado" "TITLE=Freedom" "ALBUM=Ado's Best"
  run "$PROJECT_ROOT/scripts/standardize.sh" --source "$SRC" --out "$OUT"
  [ "$status" -eq 0 ]
  jq -e '.version == 1 and (.tracks | length == 1)' "$OUT/catalog.json" >/dev/null
  jq -e '.tracks[0].artists[0] == "Ado" and .tracks[0].title == "Freedom"' "$OUT/catalog.json" >/dev/null
  jq -e '.tracks[0].bit_depth == 16 and .tracks[0].sample_rate_hz == 44100' "$OUT/catalog.json" >/dev/null
}

@test "idempotent: second run rebuilds cleanly" {
  mkflac "$SRC" "t.flac" "ARTIST=Ado" "TITLE=Freedom"
  "$PROJECT_ROOT/scripts/standardize.sh" --source "$SRC" --out "$OUT" >/dev/null 2>&1
  run "$PROJECT_ROOT/scripts/standardize.sh" --source "$SRC" --out "$OUT"
  [ "$status" -eq 0 ]
  [ -L "$OUT/Ado/Ado - Freedom [16bit-44.1kHz].flac" ]
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd ~/Dev/jukebox && bats scripts/test/integration.bats`
Expected: FAIL — `standardize.sh` does not exist.

- [ ] **Step 3: Implement `scripts/standardize.sh`**

```bash
#!/usr/bin/env bash
# standardize.sh — dedupe + symlink + catalog builder over lossless/.
# Spec: ~/Dev/jukebox/specs/2026-07-04-filtered-lossless-jukebox-design.md §1.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/lib/normalize.bash"

# --- arg/env/config/defaults resolution (spec §1.0) ---
SOURCE=""
OUT=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --source) SOURCE="$2"; shift 2;;
    --out)    OUT="$2"; shift 2;;
    *) echo "unknown arg: $1" >&2; exit 2;;
  esac
done

CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/jukebox/config.yml"
read_cfg() { # <key> <default>
  if [[ -f "$CONFIG" ]] && command -v yq >/dev/null; then
    yq ".$1" "$CONFIG" 2>/dev/null
  else echo "$2"; fi
}

[[ -z "$SOURCE" ]] && SOURCE="${LOSSLESS_SOURCE:-$(read_cfg source_dir "$HOME/Music/lossless")}"
[[ -z "$OUT" ]]    && OUT="${LOSSLESS_FILTERED:-$(read_cfg filtered_dir "$HOME/Music/filtered_lossless")}"

# --- guards ---
[[ -d "$SOURCE" ]] || { echo "source dir not found: $SOURCE" >&2; exit 1; }
find "$SOURCE" -type f -iname '*.flac' -print -quit | grep -q . || { echo "no .flac in $SOURCE" >&2; exit 1; }

# safety: refuse to wipe a dir that doesn't look like a filtered_lossless layout
if [[ -d "$OUT" && -n "$(find "$OUT" -mindepth 1 -maxdepth 1 2>/dev/null | head -1)" ]]; then
  [[ -e "$OUT/catalog.json" || -e "$OUT/_build.log" ]] \
    || { echo "refusing to wipe non-filtered_lossless dir: $OUT" >&2; exit 1; }
fi

# --- prepare output ---
rm -rf "$OUT"
mkdir -p "$OUT"
: > "$OUT/_build.log"
: > "$OUT/duplicates.log"
exec 3>>"$OUT/_build.log"

# Absolute roots for relative-symlink computation.
SRC_ABS="$(cd "$SOURCE" && pwd)"
OUT_ABS="$(cd "$OUT" && pwd)"
# Find common ancestor to compute a relative symlink target.
rel_target() { # <abs_target> <abs_symlink_dir> -> relative path from dir to target
  python3 - "$1" "$2" <<'PY'
import os,sys
print(os.path.relpath(sys.argv[1], start=sys.argv[2]))
PY
}

# --- probe every flac ---
shopt -s nullglob globstar
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# records.tsv: dedup_key \t bit_depth \t sample_rate \t isrc \t tidal \t ntags \t path
: > "$TMP/records.tsv"

probe() { # <flac-path>
  local flac="$1"
  local artists title album track disc isrc tidal date
  artists="$(read_tag "$flac" ARTIST)";     [[ -z "$artists" ]] && artists="$(read_tag "$flac" ALBUMARTIST)"
  title="$(read_tag "$flac" TITLE)"
  album="$(read_tag "$flac" ALBUM)"
  track="$(read_tag "$flac" TRACKNUMBER)"
  disc="$(read_tag "$flac" DISCNUMBER)"
  isrc="$(read_tag "$flac" ISRC)"
  tidal="$(read_tag "$flac" TIDAL_TRACK_ID)"
  date="$(read_tag "$flac" DATE)"
  [[ -z "$artists" ]] && artists="[unknown artist]"
  [[ -z "$title" ]]   && title="$(basename "$flac" .flac)"

  # quality
  local sr bd
  read sr bd < <(ffprobe -v error -show_entries stream=sample_rate,bits_per_raw_sample \
    -of csv=p=0 "$flac" 2>/dev/null | head -1 | awk -F',' '{print $1, ($2==""?"16":$2)}')
  sr="${sr:-44100}"; bd="${bd:-16}"

  local ntags
  ntags="$(metaflac --export-tags-to=- "$flac" 2>/dev/null | grep -c '=' || true)"

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$(dedup_key "$artists" "$title")" "$bd" "$sr" "$isrc" "$tidal" "$ntags" "$flac"
}

while IFS= read -r -d '' flac; do
  probe "$flac" >> "$TMP/records.tsv"
done < <(find "$SOURCE" -type f -iname '*.flac' -print0)

# --- group by dedup_key, pick winner ---
declare -A WINNERS   # dedup_key -> candidate line
declare -A LOSERS     # dedup_key -> newline-joined loser lines
prev_key=""
declare -a bucket
flush_bucket() {
  [[ ${#bucket[@]} -eq 0 ]] && return
  local win
  win="$(printf '%s\n' "${bucket[@]}" | pick_winner)"
  WINNERS["$1"]="$win"
  local l
  for l in "${bucket[@]}"; do [[ "$l" != "$win" ]] && LOSERS["$1"]+="$l"$'\n'; done
  bucket=()
}
sort -t$'\t' -k1,1 "$TMP/records.tsv" | while IFS=$'\t' read -r key rest; do
  if [[ "$key" != "$prev_key" && -n "$prev_key" ]]; then flush_bucket "$prev_key"; fi
  bucket+=("$key"$'\t'"$rest"); prev_key="$key"
done
flush_bucket "$prev_key"

# Note: the `while | ...` subshell above loses WINNERS/LOSERS. Re-do without pipe.
# (Restart grouping using a process-substitution approach that preserves vars.)
declare -A WINNERS; declare -A LOSERS; bucket=(); prev_key=""
last_key() { :; }
while IFS=$'\t' read -r key rest; do
  if [[ "$key" != "$prev_key" && -n "$prev_key" ]]; then
    win="$(printf '%s\n' "${bucket[@]}" | pick_winner)"; WINNERS["$prev_key"]="$win"
    for l in "${bucket[@]}"; do [[ "$l" != "$win" ]] && LOSERS["$prev_key"]+="$l"$'\n'; done
    bucket=()
  fi
  bucket+=("$key"$'\t'"$rest"); prev_key="$key"
done < <(sort -t$'\t' -k1,1 "$TMP/records.tsv")
# final flush
if [[ ${#bucket[@]} -gt 0 ]]; then
  win="$(printf '%s\n' "${bucket[@]}" | pick_winner)"; WINNERS["$prev_key"]="$win"
  for l in "${bucket[@]}"; do [[ "$l" != "$win" ]] && LOSERS["$prev_key"]+="$l"$'\n'; done
fi

# --- emit symlinks + catalog.json + duplicates.log ---
tracks_json='[]'
emit_winner() { # <dedup_key> <winner-line>
  local key="$1" line="$2"
  IFS=$'\t' read -r _ bd sr isrc tidal ntags path <<<"$line"
  # re-read tags from the winner for the catalog record
  local artists title album track disc
  artists="$(read_tag "$path" ARTIST)"; [[ -z "$artists" ]] && artists="$(read_tag "$path" ALBUMARTIST)"
  title="$(read_tag "$path" TITLE)"; [[ -z "$title" ]] && title="$(basename "$path" .flac)"
  album="$(read_tag "$path" ALBUM)"; track="$(read_tag "$path" TRACKNUMBER)"; disc="$(read_tag "$path" DISCNUMBER)"

  local canon split_names
  canon="$(normalize_artist_string "$artists")"
  # split into per-artist folders
  local -a arts; mapfile -t arts < <(split_artists "$artists")
  [[ ${#arts[@]} -eq 0 ]] && arts=("[unknown artist]")

  local khz; khz="$(khz_label "$sr")"
  local fname; fname="$(sanitize_filename "$canon - $title [${bd}bit-${khz}].flac")"

  local symlinked='[]'
  local artist dir relpath linkpath
  for artist in "${arts[@]}"; do
    dir="$OUT_ABS/$(sanitize_filename "$artist")"
    mkdir -p "$dir"
    # collision handling
    linkpath="$dir/$fname"; local n=2
    while [[ -e "$linkpath" || -L "$linkpath" ]]; do
      linkpath="$dir/${fname%.flac} ($n).flac"; n=$((n+1))
    done
    relpath="$(rel_target "$path" "$dir")"
    ln -s "$relpath" "$linkpath"
    symlinked="$(echo "$symlinked" | jq -c --arg a "$artist" '. += [$a]')"
  done

  local id; id="$(printf '%s' "$key" | shasum -a 256 | cut -c1-16)"

  local src_rel; src_rel="$(rel_target "$path" "$OUT_ABS/../..")"
  # store source_path relative to the parent of lossless (matches spec example)
  src_rel="lossless/$(rel_target "$path" "$SRC_ABS" )"

  tracks_json="$(echo "$tracks_json" | jq -c \
    --arg id "$id" --arg artists "$canon" \
    --argjson arts "$(printf '%s\n' "${arts[@]}" | jq -R . | jq -s .)" \
    --arg primary "${arts[0]}" --arg title "$title" --arg album "$album" \
    --arg track "$track" --arg disc "$disc" --arg isrc "$isrc" \
    --arg source "$src_rel" \
    --argjson symlinked "$symlinked" \
    --arg bd "$bd" --arg sr "$sr" \
    '. += [{
       id:$id, artists:$arts, primary_artist:$primary, title:$title,
       album:($album|select(.>"")), track_number:($track|tonumber?),
       disc_number:($disc|tonumber?), bit_depth:($bd|tonumber),
       sample_rate_hz:($sr|tonumber), isrc:($isrc|select(.>"")),
       source_path:$source, symlinked_into_artists:$symlinked
     }]')"
}

for key in "${!WINNERS[@]}"; do emit_winner "$key" "${WINNERS[$key]}"; done

# losers log
for key in "${!LOSERS[@]}"; do
  while IFS= read -r l; do
    [[ -z "$l" ]] && continue
    IFS=$'\t' read -r _ bd sr isrc tidal ntags path <<<"$l"
    echo -e "DUP\t$key\t$bd/$sr\t$isrc\t$path" >> "$OUT/duplicates.log"
  done <<<"${LOSERS[$key]}"
}

jq -n --argjson tracks "$tracks_json" --arg src "$SRC_ABS" --arg at "$(date -u +%FT%TZ)" \
  '{version:1, built_at:$at, source_root:$src, tracks:$tracks}' > "$OUT/catalog.json"

echo "indexed $(echo "$tracks_json" | jq 'length') unique tracks" >&3
```

> Note: the double-grouping block (lines starting `declare -A WINNERS` the second time) replaces the subshell-losing first attempt. The first `sort | while` block is dead code to remove. See Step 4 cleanup.

- [ ] **Step 4: Remove the dead first grouping block**

Delete from the first occurrence of `declare -A WINNERS   # dedup_key` through the line `flush_bucket "$prev_key"` (the subshell version), keeping only the process-substitution version. Re-read the file after writing to confirm only one grouping loop remains.

- [ ] **Step 5: Make the script executable**

```bash
chmod +x ~/Dev/jukebox/scripts/standardize.sh
```

- [ ] **Step 6: Run the integration tests to verify they pass**

Run: `cd ~/Dev/jukebox && bats scripts/test/integration.bats`
Expected: PASS — all 5 tests green. If `mkflac` synthesizes a real flac via ffmpeg, bit_depth/sample_rate will be 16/44100 as asserted.

- [ ] **Step 7: Commit**

```bash
cd ~/Dev/jukebox
git add scripts/standardize.sh scripts/test/integration.bats
git commit -m "feat(standardize): full indexer — symlinks, catalog.json, duplicates.log"
```

---

## Task 5: `standardize.sh` input resolution + safety guards (unit tests)

**Files:**
- Create: `scripts/test/inputs.bats`
- Modify: `scripts/standardize.sh` only if a guard is missing (the guards already exist from Task 4; this task hardens them with explicit tests).

**Interfaces:**
- Produces: documented exit codes — `0` success, `1` bad source/empty/safety, `2` unknown arg.

- [ ] **Step 1: Write the failing test `scripts/test/inputs.bats`**

```bats
#!/usr/bin/env bats
load helpers.bash

@test "unknown flag exits 2" {
  run "$PROJECT_ROOT/scripts/standardize.sh" --bogus
  [ "$status" -eq 2 ]
}

@test "missing source dir exits 1" {
  run "$PROJECT_ROOT/scripts/standardize.sh" --source /no/such/dir --out "$(mktemp -d)"
  [ "$status" -eq 1 ]
}

@test "source dir without flac exits 1" {
  d="$(mktemp -d)"; : > "$d/notes.txt"
  run "$PROJECT_ROOT/scripts/standardize.sh" --source "$d" --out "$(mktemp -d)"
  [ "$status" -eq 1 ]
  rm -rf "$d"
}

@test "refuses to wipe a non-filtered layout" {
  out="$(mktemp -d)"; echo "important" > "$out/secret.txt"
  d="$(mktemp -d)"; mkflac "$d" "t.flac" "ARTIST=A" "TITLE=T"
  run "$PROJECT_ROOT/scripts/standardize.sh" --source "$d" --out "$out"
  [ "$status" -eq 1 ]
  [ -f "$out/secret.txt" ]   # not wiped
  rm -rf "$out" "$d"
}
```

- [ ] **Step 2: Run the tests to verify they fail/pass**

Run: `cd ~/Dev/jukebox && bats scripts/test/inputs.bats`
Expected: If the safety guard check from Task 4 already covers it, these may pass immediately. Any failure means a guard is missing — patch `scripts/standardize.sh` to add it, then re-run until PASS.

- [ ] **Step 3: Commit**

```bash
cd ~/Dev/jukebox
git add scripts/test/inputs.bats scripts/standardize.sh
git commit -m "test(standardize): input resolution + safety guard tests"
```

---

## Task 6: `main.rs` clap dispatch + first-run prompt + `jukebox config`

**Files:**
- Create: `src/cli.rs`, `src/prompt.rs`, `tests/cli.rs`
- Modify: `src/main.rs`
- Produces: `Cli` enum, `run()`, `prompt_source_dir()`, `ensure_config()`.

**Interfaces:**
- Consumes: `Config`, `config_path`, `validate_source_dir` from Task 1.
- Produces (used by Tasks 9, 11, 13, 15): `ensure_config() -> Result<Config>` (loads or runs first-run then loads), and the subcommand dispatch.

- [ ] **Step 1: Write the failing test `tests/cli.rs`**

```rust
use jukebox::cli::Cli;
use jukebox::config::Config;
use jukebox::prompt::prompt_source_dir;
use std::io::Write;
use tempfile::tempdir;

#[test]
fn ensure_config_runs_first_run_from_stdin() {
    let tmp = tempdir().unwrap();
    std::env::set_var("HOME", tmp.path());
    std::env::remove_var("XDG_CONFIG_HOME");
    // create a valid source dir with a flac
    let src = tmp.path().join("lossless");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.flac"), b"x").unwrap();
    // feed the path on stdin
    let input = format!("{}\n", src.display());
    // We can't easily redirect stdin in a unit test; instead call prompt_source_dir
    // with a Cursor-like reader by constructing it via a helper.
    let mut buf = std::io::Cursor::new(input.into_bytes());
    let chosen = prompt_source_dir_with(&mut buf, &src).unwrap();
    assert_eq!(chosen, src.canonicalize().unwrap());
}
```

> `prompt_source_dir_with` is a testable variant taking an explicit reader + default; `prompt_source_dir` (no `_with`) wraps it using `std::io::stdin()`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test cli`
Expected: FAIL — `jukebox::cli`, `jukebox::prompt` not found.

- [ ] **Step 3: Implement `src/prompt.rs`**

```rust
use anyhow::{anyhow, Result};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use crate::config::validate_source_dir;

/// Prompt on stdin for a source directory, with `default` prefilled.
/// Reads one line, expands `~`, validates, repeats on bad input up to 3 times.
pub fn prompt_source_dir(default: &Path) -> Result<PathBuf> {
    let stdin = std::io::stdin();
    let mut lock = stdin.lock();
    prompt_source_dir_with(&mut lock, default)
}

/// Testable variant: reads from `r`, writes the prompt to stderr.
pub fn prompt_source_dir_with<R: BufRead>(r: &mut R, default: &Path) -> Result<PathBuf> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    for _ in 0..3 {
        eprint!("Lossless source dir [{}]: ", default.display());
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        let n = r.read_line(&mut line)?;
        if n == 0 { return Err(anyhow!("no input on stdin")); }
        let raw = line.trim();
        let expanded = if raw.starts_with('~') {
            let rest = &raw[1..];
            let rest = rest.strip_prefix('/').unwrap_or(rest);
            home.join(rest)
        } else if raw.is_empty() {
            default.to_path_buf()
        } else {
            PathBuf::from(raw)
        };
        match validate_source_dir(&expanded) {
            Ok(()) => return Ok(expanded.canonicalize().unwrap_or(expanded)),
            Err(e) => eprintln!("  invalid: {e}"),
        }
    }
    Err(anyhow!("gave up after 3 invalid attempts"))
}
```

- [ ] **Step 4: Implement `src/cli.rs`**

```rust
use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::config::{Config, config_path};

#[derive(Parser, Debug)]
#[command(name = "jukebox", version, about = "Filtered-lossless jukebox")]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Launch the TUI (default).
    Play,
    /// Run standardize.sh then rebuild the search index.
    Sync,
    /// Build/rebuild the Tantivy search index from catalog.json.
    Index,
    /// Re-run the directory prompt, or set a field.
    Config {
        /// e.g. `set source_dir <path>`
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// One-shot CLI search.
    Search {
        /// query string
        query: Vec<String>,
    },
}

/// Load config, or run the first-run prompt then load.
pub fn ensure_config() -> Result<Config> {
    if let Some(cfg) = Config::load()? {
        if cfg.source_dir.as_os_str().is_empty() {
            first_run()?;
        } else {
            return Ok(cfg);
        }
    } else {
        first_run()?;
    }
    Config::load()?.ok_or_else(|| anyhow::anyhow!("config still missing after first-run"))
}

fn first_run() -> Result<()> {
    eprintln!("Welcome to jukebox. Let's configure your library.");
    let default = dirs::home_dir().map(|h| h.join("Music/lossless")).unwrap_or_default();
    let source = crate::prompt::prompt_source_dir(&default)?;
    let cfg = Config::default_for(source);
    cfg.save()?;
    eprintln!("Saved config to {}", config_path().display());
    Ok(())
}
```

- [ ] **Step 5: Update `src/main.rs` with dispatch**

```rust
pub mod cli;
pub mod config;
pub mod prompt;

fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    match args.cmd.unwrap_or(cli::Cmd::Play) {
        cli::Cmd::Config { args } => {
            let _cfg = cli::ensure_config()?;
            if !args.is_empty() {
                eprintln!("config edits are not yet supported; edit {}", config::config_path().display());
            } else {
                println!("config: {}", config::config_path().display());
            }
        }
        cli::Cmd::Play => { eprintln!("(TUI not implemented yet)"); }
        cli::Cmd::Sync => { eprintln!("(sync not implemented yet)"); }
        cli::Cmd::Index => { eprintln!("(index not implemented yet)"); }
        cli::Cmd::Search { query } => { eprintln!("search: {}", query.join(" ")); }
    }
    Ok(())
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --test cli && cargo build`
Expected: PASS; binary builds.

- [ ] **Step 7: Commit**

```bash
cd ~/Dev/jukebox
git add src/cli.rs src/prompt.rs src/main.rs tests/cli.rs
git commit -m "feat(cli): clap dispatch + first-run prompt + jukebox config"
```

---

## Task 7: `catalog.rs` — parse `catalog.json`

**Files:**
- Create: `src/catalog.rs`, `tests/catalog.rs`
- Produces: `Catalog`, `Track`, `Catalog::load()`, `Track::resolve_source()`.

**Interfaces:**
- Consumes: the `catalog.json` schema from Task 4.
- Produces (used by Tasks 9, 11, 12):
  ```rust
  pub struct Catalog { pub version: u32, pub built_at: String, pub source_root: PathBuf, pub tracks: Vec<Track> }
  pub struct Track { /* all fields per spec §1.6 */ }
  impl Catalog { pub fn load(path: &Path) -> Result<Catalog>; }
  impl Track { pub fn resolve_source(&self, source_root: &Path) -> PathBuf; }
  ```

- [ ] **Step 1: Write the failing test `tests/catalog.rs`**

```rust
use jukebox::catalog::Catalog;
use std::fs;
use tempfile::tempdir;

fn sample() -> &'static str {
    r#"{
      "version": 1,
      "built_at": "2026-07-04T00:00:00Z",
      "source_root": "/Users/distiled/Music/lossless",
      "tracks": [
        {
          "id": "abc123",
          "artists": ["Ado"],
          "primary_artist": "Ado",
          "title": "Freedom",
          "album": "Ado's Best",
          "track_number": 15,
          "disc_number": 1,
          "bit_depth": 24,
          "sample_rate_hz": 48000,
          "isrc": "JPPO02105116",
          "source_path": "lossless/Ado discography/Ado - Freedom.flac",
          "symlinked_into_artists": ["Ado"]
        }
      ]
    }"#
}

#[test]
fn parses_catalog() {
    let d = tempdir().unwrap();
    let p = d.path().join("catalog.json");
    fs::write(&p, sample()).unwrap();
    let c = Catalog::load(&p).unwrap();
    assert_eq!(c.version, 1);
    assert_eq!(c.tracks.len(), 1);
    let t = &c.tracks[0];
    assert_eq!(t.title, "Freedom");
    assert_eq!(t.artists, vec!["Ado".to_string()]);
    assert_eq!(t.bit_depth, 24);
    assert_eq!(t.sample_rate_hz, 48000);
    assert_eq!(t.isrc.as_deref(), Some("JPPO02105116"));
}

#[test]
fn resolve_source_joins_parent_of_source_root() {
    let d = tempdir().unwrap();
    let p = d.path().join("catalog.json");
    fs::write(&p, sample()).unwrap();
    let c = Catalog::load(&p).unwrap();
    let t = &c.tracks[0];
    let abs = t.resolve_source(&c.source_root);
    assert!(abs.ends_with("Ado discography/Ado - Freedom.flac"));
    assert!(abs.is_absolute() || abs.starts_with("lossless/"));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test catalog`
Expected: FAIL — `jukebox::catalog` not found.

- [ ] **Step 3: Implement `src/catalog.rs`**

```rust
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub version: u32,
    pub built_at: String,
    pub source_root: PathBuf,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub artists: Vec<String>,
    pub primary_artist: String,
    pub title: String,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub track_number: Option<u32>,
    #[serde(default)]
    pub disc_number: Option<u32>,
    #[serde(default)]
    pub bit_depth: u32,
    #[serde(default)]
    pub sample_rate_hz: u32,
    #[serde(default)]
    pub isrc: Option<String>,
    pub source_path: PathBuf,
    pub symlinked_into_artists: Vec<String>,
}

impl Catalog {
    pub fn load(path: &Path) -> Result<Catalog> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading catalog {}", path.display()))?;
        let c: Catalog = serde_json::from_str(&text)
            .with_context(|| format!("parsing catalog {}", path.display()))?;
        Ok(c)
    }
}

impl Track {
    /// Resolve the absolute source path. `source_path` in the catalog is relative
    /// to the parent of `source_root` (e.g. `lossless/...` under `~/Music`).
    pub fn resolve_source(&self, source_root: &Path) -> PathBuf {
        match source_root.parent() {
            Some(parent) => parent.join(&self.source_path),
            None => self.source_path.clone(),
        }
    }

    pub fn quality_label(&self) -> String {
        let khz = if self.sample_rate_hz % 1000 == 0 {
            format!("{}kHz", self.sample_rate_hz / 1000)
        } else {
            format!("{:.1}kHz", self.sample_rate_hz as f64 / 1000.0)
        };
        format!("{}bit-{}", self.bit_depth, khz)
    }
}
```

- [ ] **Step 4: Register the module in `src/main.rs`**

Add `pub mod catalog;` alongside the others.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --test catalog`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cd ~/Dev/jukebox
git add src/catalog.rs src/main.rs tests/catalog.rs
git commit -m "feat(catalog): parse catalog.json with typed records"
```

---

## Task 8: `translit.rs` — kana ↔ romaji variant generation

**Files:**
- Create: `src/translit.rs`, `tests/translit.rs`
- Produces: `pub fn variants(text: &str) -> Vec<String>`.

**Interfaces:**
- Consumes: `wana_kana::ConvertJapanese`, `wana_kana::IsJapaneseStr`.
- Produces (used by Task 9): `variants()` returns alternate-script forms to index alongside the original text.

- [ ] **Step 1: Write the failing test `tests/translit.rs`**

```rust
use jukebox::translit::variants;

#[test]
fn katakana_yields_romaji_and_hiragana() {
    let v = variants("ブルーバード");
    assert!(v.iter().any(|s| s == "burubado"), "got romaji: {:?}", v);
    assert!(v.iter().any(|s| s == "ぶるーばーど"), "got hiragana: {:?}", v);
}

#[test]
fn hiragana_yields_romaji_and_katakana() {
    let v = variants("ぶるーばーど");
    assert!(v.iter().any(|s| s == "burubado"));
    assert!(v.iter().any(|s| s == "ブルーバード"));
}

#[test]
fn ascii_only_yields_no_variants() {
    let v = variants("Blue Bird");
    assert!(v.is_empty(), "got: {:?}", v);
}

#[test]
fn variants_are_deduped() {
    let v = variants("カナカナ");
    assert_eq!(v.len(), 2); // romaji + hiragana
}

#[test]
fn mixed_kana_ascii_still_transliterates_kana() {
    let v = variants("Ado ブルーバード");
    assert!(v.iter().any(|s| s.contains("burubado")));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test translit`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement `src/translit.rs`**

```rust
use wana_kana::{ConvertJapanese, IsJapaneseStr};

/// Return alternate-script variants of `text` for cross-script search.
/// - katakana text → romaji + hiragana
/// - hiragana text → romaji + katakana
/// ASCII-only or kanji-only text yields no variants (kanji→romaji needs a
/// dictionary, which we deliberately do not ship).
/// The original text is NOT included here; the caller indexes it separately.
pub fn variants(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // `is_kana` returns true only if every char is kana; we want "contains kana".
    let has_kana = text.chars().any(|c| {
        let s: &str = std::str::from_chars(&[c]);
        // wana_kana checks operate on whole strings; emulate per-char kana test.
        s.is_kana()
    });
    if !has_kana {
        return out;
    }
    // to_romaji converts kana→romaji and passes through non-kana chars.
    let romaji = text.to_romaji();
    if romaji != text {
        out.push(romaji);
    }
    if text.is_katakana() || has_katakana(text) {
        let h = text.to_hiragana();
        if h != text { out.push(h); }
    }
    if text.is_hiragana() || has_hiragana(text) {
        let k = text.to_katakana();
        if k != text { out.push(k); }
    }
    out.sort();
    out.dedup();
    out
}

fn has_katakana(s: &str) -> bool {
    s.chars().any(|c| {
        let v = c as u32;
        (0x30A0..=0x30FF).contains(&v) || (0xFF66..=0xFF9F).contains(&v)
    })
}
fn has_hiragana(s: &str) -> bool {
    s.chars().any(|c| { (0x3040..=0x309F).contains(&(c as u32)) })
}
```

- [ ] **Step 4: Register the module**

Add `pub mod translit;` to `src/main.rs`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --test translit`
Expected: PASS — all 5 tests green.

- [ ] **Step 6: Commit**

```bash
cd ~/Dev/jukebox
git add src/translit.rs src/main.rs tests/translit.rs
git commit -m "feat(translit): kana<->romaji variant generation"
```

---

## Task 9: `search.rs` — Tantivy schema + index build

**Files:**
- Create: `src/search.rs`, `tests/search.rs`
- Produces: `build_index()`, `SCHEMA`, `INDEX_DIR_NAME`.

**Interfaces:**
- Consumes: `Catalog`, `Track` (Task 7), `translit::variants` (Task 8).
- Produces (used by Tasks 10, 13): `pub fn build_index(catalog: &Catalog, index_dir: &Path) -> Result<()>`, `pub fn open(index_dir: &Path) -> Result<Searcher>`.

- [ ] **Step 1: Write the failing test `tests/search.rs` (build part)**

```rust
use jukebox::catalog::Catalog;
use jukebox::search::build_index;
use std::fs;
use tempfile::tempdir;

fn mini_catalog_json() -> String {
    serde_json::json!({
        "version": 1, "built_at": "2026-07-04T00:00:00Z",
        "source_root": "/tmp/lossless",
        "tracks": [
          { "id":"t1","artists":["Ikimono-gakari"],"primary_artist":"Ikimono-gakari",
            "title":"ブルーバード","album":"My Song","bit_depth":16,"sample_rate_hz":44100,
            "source_path":"lossless/i/01.flac","symlinked_into_artists":["Ikimono-gakari"] },
          { "id":"t2","artists":["Ado"],"primary_artist":"Ado",
            "title":"Freedom","album":"Best","bit_depth":24,"sample_rate_hz":48000,
            "source_path":"lossless/a/01.flac","symlinked_into_artists":["Ado"] },
        ]
    }).to_string()
}

#[test]
fn build_index_writes_segments() {
    let d = tempdir().unwrap();
    let cat_path = d.path().join("catalog.json");
    fs::write(&cat_path, mini_catalog_json()).unwrap();
    let cat = Catalog::load(&cat_path).unwrap();
    let idx = d.path().join("search-index");
    build_index(&cat, &idx).unwrap();
    assert!(idx.is_dir());
    assert!(fs::read_dir(&idx).unwrap().count() > 0);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test search -- build_index_writes_segments`
Expected: FAIL — `jukebox::search` not found.

- [ ] **Step 3: Implement `src/search.rs` (build + open stub)**

```rust
use anyhow::{Context, Result};
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Schema, INDEXED, STORED, TEXT};
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadCondition};

use crate::catalog::Catalog;
use crate::translit::variants;

pub fn schema() -> (Schema, SearchFields) {
    let mut b = Schema::builder();
    let id = b.add_text_field("id", STORED);
    let artists = b.add_text_field("artists", TEXT);
    let title = b.add_text_field("title", TEXT);
    let title_variants = b.add_text_field("title_variants", TEXT);
    let artist_variants = b.add_text_field("artist_variants", TEXT);
    let album = b.add_text_field("album", TEXT);
    let quality = b.add_text_field("quality", INDEXED);
    let s = b.build();
    (s, SearchFields { id, artists, title, title_variants, artist_variants, album, quality })
}

#[derive(Clone)]
pub struct SearchFields {
    pub id: Field,
    pub artists: Field,
    pub title: Field,
    pub title_variants: Field,
    pub artist_variants: Field,
    pub album: Field,
    pub quality: Field,
}

pub fn build_index(catalog: &Catalog, index_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(index_dir)?;
    let (schema, fields) = schema();
    let index = Index::open_or_create_in_dir(index_dir, schema.clone())?;

    // Japanese tokenizer via Lindera on text fields.
    use lindera::dictionary::load_dictionary;
    use lindera::mode::Mode;
    use lindera::segmenter::Segmenter;
    use lindera_tantivy::tokenizer::LinderaTokenizer;
    let dict = load_dictionary("embedded://ipadic").context("loading embedded ipadic")?;
    let segmenter = Segmenter::new(Mode::Normal, dict, None);
    let lindera = LinderaTokenizer::from_segmenter(segmenter);
    for f in [fields.artists, fields.title, fields.album] {
        index.tokenizers().register("lindera", lindera.clone());
        let _ = f; // tokenizer bound to field via schema? Tantivy 0.26: set via field entry.
    }
    // Tantivy 0.26: register tokenizer name on the field's TextOptions.
    // We rebuild schema with named tokenizers.
    let (schema2, fields2) = schema_with_tokenizers();
    let index = Index::open_or_create_in_dir(index_dir, schema2.clone())?;
    index.tokenizers().register("lindera", lindera.clone());
    index.tokenizers().register("lowercase", tantivy::tokenizer::SimpleTokenizer::default()
        .filter(tantivy::tokenizer::LowerCaser));

    let mut writer: IndexWriter = index.writer(50_000_000)?;
    writer.delete_all_documents()?;
    for t in &catalog.tracks {
        let title_variants_str = variants(&t.title).join(" ");
        let artist_variants_str = t.artists.iter().flat_map(|a| variants(a)).collect::<Vec<_>>().join(" ");
        let quality = format!("{}bit-{}Hz", t.bit_depth, t.sample_rate_hz);
        writer.add_document(doc!(
            fields2.id => t.id.as_str(),
            fields2.artists => t.artists.join(" "),
            fields2.title => t.title.as_str(),
            fields2.title_variants => title_variants_str.as_str(),
            fields2.artist_variants => artist_variants_str.as_str(),
            fields2.album => t.album.clone().unwrap_or_default().as_str(),
            fields2.quality => quality.as_str(),
        ))?;
    }
    writer.commit()?;
    Ok(())
}

// Schema variant where text fields use the `lindera` tokenizer explicitly.
fn schema_with_tokenizers() -> (Schema, SearchFields) {
    use tantivy::schema::TextOptions;
    let lindera = TextOptions::default().set_indexing_options(
        tantivy::schema::TextFieldIndexingOptions::default()
            .set_tokenizer("lindera").set_index_option(tantivy::schema::IndexRecordOption::Basic)
    );
    let lower = TextOptions::default().set_indexing_options(
        tantivy::schema::TextFieldIndexingOptions::default()
            .set_tokenizer("lowercase").set_index_option(tantivy::schema::IndexRecordOption::Basic)
    );
    let mut b = Schema::builder();
    let id = b.add_text_field("id", STORED);
    let artists = b.add_text_field("artists", lindera.clone());
    let title = b.add_text_field("title", lindera.clone());
    let title_variants = b.add_text_field("title_variants", lower.clone());
    let artist_variants = b.add_text_field("artist_variants", lower.clone());
    let album = b.add_text_field("album", lindera);
    let quality = b.add_text_field("quality", INDEXED);
    (b.build(), SearchFields { id, artists, title, title_variants, artist_variants, album, quality })
}

pub struct Searcher {
    reader: IndexReader,
    fields: SearchFields,
    index: Index,
}

impl Searcher {
    pub fn open(index_dir: &Path) -> Result<Searcher> {
        let (_, fields) = schema_with_tokenizers();
        let index = Index::open_in_dir(index_dir)?;
        // re-register tokenizers for the opened index too
        use lindera::dictionary::load_dictionary;
        use lindera::mode::Mode;
        use lindera::segmenter::Segmenter;
        use lindera_tantivy::tokenizer::LinderaTokenizer;
        let dict = load_dictionary("embedded://ipadic").context("loading ipadic")?;
        let seg = Segmenter::new(Mode::Normal, dict, None);
        index.tokenizers().register("lindera", LinderaTokenizer::from_segmenter(seg));
        index.tokenizers().register("lowercase", tantivy::tokenizer::SimpleTokenizer::default()
            .filter(tantivy::tokenizer::LowerCaser));
        let reader = index.reader_builder().reload_policy(ReloadCondition::OnCommit).try_into()?;
        Ok(Searcher { reader, fields, index })
    }
}
```

- [ ] **Step 4: Register the module**

Add `pub mod search;` to `src/main.rs`.

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test --test search -- build_index_writes_segments`
Expected: PASS. (First build compiles lindera's embedded IPADIC; expect a longer first build.)

- [ ] **Step 6: Commit**

```bash
cd ~/Dev/jukebox
git add src/search.rs src/main.rs tests/search.rs
git commit -m "feat(search): Tantivy schema + index build with Lindera tokenizer"
```

---

## Task 10: `search.rs` — query + ranking + `jukebox search`/`jukebox index` wiring

**Files:**
- Modify: `src/search.rs` (add `search()`), `src/main.rs` (wire `Index`/`Search`)
- Create: append to `tests/search.rs`
- Produces: `Searcher::search() -> Result<Vec<Hit>>`, `Hit { track_id, score }`.

**Interfaces:**
- Consumes: `Searcher::open` (Task 9).
- Produces: `pub fn search(&self, q: &str, limit: usize) -> Result<Vec<Hit>>`.

- [ ] **Step 1: Write the failing tests (append to `tests/search.rs`)**

```rust
use jukebox::search::Searcher;

fn build_then_open() -> (tempfile::TempDir, Searcher) {
    let d = tempdir().unwrap();
    let cat_path = d.path().join("catalog.json");
    std::fs::write(&cat_path, mini_catalog_json()).unwrap();
    let cat = jukebox::catalog::Catalog::load(&cat_path).unwrap();
    let idx = d.path().join("search-index");
    jukebox::search::build_index(&cat, &idx).unwrap();
    let s = Searcher::open(&idx).unwrap();
    (d, s)
}

#[test]
fn romaji_finds_katakana_title() {
    let (_d, s) = build_then_open();
    let hits = s.search("burubado", 10).unwrap();
    assert!(hits.iter().any(|h| h.track_id == "t1"), "romaji -> ブルーバード");
}

#[test]
fn ascii_title_exact_ranks_high() {
    let (_d, s) = build_then_open();
    let hits = s.search("Freedom", 10).unwrap();
    assert_eq!(hits[0].track_id, "t2");
}

#[test]
fn fuzzy_typo_tolerated() {
    let (_d, s) = build_then_open();
    let hits = s.search("Freedon", 10).unwrap();
    assert!(hits.iter().any(|h| h.track_id == "t2"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test search`
Expected: FAIL — `search()` method not found.

- [ ] **Step 3: Implement `search()` and `Hit` in `src/search.rs`**

Append:

```rust
pub struct Hit {
    pub track_id: String,
    pub score: f32,
}

impl Searcher {
    pub fn search(&self, q: &str, limit: usize) -> Result<Vec<Hit>> {
        let qp = QueryParser::for_index(&self.index, vec![
            self.fields.title, self.fields.title_variants,
            self.fields.artists, self.fields.artist_variants, self.fields.album,
        ]);
        // BM25 boosts: title x2, title_variants x1.5, others x1.
        let qp = qp;
        let mut qp = qp;
        qp.set_field_boost(self.fields.title, 2.0);
        qp.set_field_boost(self.fields.title_variants, 1.5);
        // fuzzy (edit distance 2) on the romaji/ascii variant fields
        qp.set_field_fuzzy(self.fields.title_variants, true, 2, true);
        qp.set_field_fuzzy(self.fields.artist_variants, true, 2, true);
        qp.set_field_fuzzy(self.fields.title, true, 2, true);
        let query = qp.parse_query(q)?;
        let searcher = self.reader.searcher();
        let top = searcher.search(&query, &TopDocs::with_limit(limit))?;
        let mut hits = Vec::new();
        for (score, doc_addr) in top {
            let doc = searcher.doc::<tantivy::TantivyDocument>(doc_addr)?;
            if let Some(id) = doc.get_first(self.fields.id).and_then(|v| v.as_str()) {
                hits.push(Hit { track_id: id.to_string(), score });
            }
        }
        Ok(hits)
    }
}
```

- [ ] **Step 4: Wire `jukebox index` and `jukebox search` in `src/main.rs`**

Replace the `Index` and `Search` stub arms:

```rust
cli::Cmd::Index => {
    let cfg = cli::ensure_config()?;
    let cat = catalog::Catalog::load(&cfg.filtered_dir.join("catalog.json"))?;
    search::build_index(&cat, &cfg.filtered_dir.join("search-index"))?;
    println!("indexed {} tracks", cat.tracks.len());
}
cli::Cmd::Search { query } => {
    let cfg = cli::ensure_config()?;
    let s = search::Searcher::open(&cfg.filtered_dir.join("search-index"))?;
    let q = query.join(" ");
    let cat = catalog::Catalog::load(&cfg.filtered_dir.join("catalog.json"))?;
    for hit in s.search(&q, 25)? {
        if let Some(t) = cat.tracks.iter().find(|t| t.id == hit.track_id) {
            println!("{:>3.0}%  {} — {} [{}]", hit.score * 100.0, t.title, t.primary_artist, t.quality_label());
        }
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --test search`
Expected: PASS — all 4 search tests green.

- [ ] **Step 6: Commit**

```bash
cd ~/Dev/jukebox
git add src/search.rs src/main.rs tests/search.rs
git commit -m "feat(search): BM25 query + fuzzy + jukebox index/search commands"
```

---

## Task 11: `player.rs` — `Player` trait + stub + mpv IPC + afplay fallback

**Files:**
- Create: `src/player.rs`, `src/tui/queue.rs`, `tests/player.rs`
- Produces: `Player` trait, `StubPlayer`, `MpvPlayer`, `AfplayPlayer`, `launch()`, plus `Queue` (deterministic shuffle).

**Interfaces:**
- Consumes: `Track` (Task 7), `Config`/`PlayerKind` (Task 1).
- Produces (used by Tasks 12-13): `pub trait Player`, `pub fn launch(kind, socket) -> Box<dyn Player>`, `Queue` with `enqueue`, `shuffle(seed)`, `remove`, `clear`, `next`, `prev`.

- [ ] **Step 1: Write the failing test `tests/player.rs`**

```rust
use jukebox::player::{StubPlayer, Player};
use jukebox::tui::queue::Queue;

#[test]
fn stub_player_records_loads() {
    let mut p = StubPlayer::default();
    p.load(std::path::Path::new("/x.flac")).unwrap();
    assert_eq!(p.loaded(), Some(std::path::PathBuf::from("/x.flac")));
    p.play_pause().unwrap();
    assert!(p.is_playing());
}

#[test]
fn queue_shuffle_is_deterministic_with_seed() {
    let mut q = Queue::new();
    for i in 0..5 { q.enqueue(format!("id{i}")); }
    q.shuffle(42);
    assert_eq!(q.items().len(), 5);
    let first = q.items().clone();
    q.shuffle(42);
    assert_eq!(q.items(), &first, "same seed -> same order");
}

#[test]
fn queue_next_wraps_and_clear_resets() {
    let mut q = Queue::new();
    q.enqueue("a".into()); q.enqueue("b".into());
    assert_eq!(q.current(), Some(&"a".to_string()));
    q.next();
    assert_eq!(q.current(), Some(&"b".to_string()));
    q.next();
    assert_eq!(q.current(), Some(&"a".to_string())); // wrap
    q.clear();
    assert!(q.items().is_empty());
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test player`
Expected: FAIL — modules not found.

- [ ] **Step 3: Implement `src/tui/queue.rs`**

```rust
/// A play queue with deterministic (seeded) Fisher–Yates shuffle.
#[derive(Default, Clone)]
pub struct Queue {
    items: Vec<String>,
    cursor: usize,
    order: Vec<usize>,   // permutation; identity until shuffled
    order_cursor: usize,
}

impl Queue {
    pub fn new() -> Self { Self::default() }

    pub fn enqueue(&mut self, id: String) {
        let idx = self.items.len();
        self.items.push(id);
        self.order.push(idx);
    }

    pub fn items(&self) -> &Vec<String> { &self.items }
    pub fn len(&self) -> usize { self.items.len() }
    pub fn is_empty(&self) -> bool { self.items.is_empty() }

    /// Fisher–Yates with a linear congruential RNG seeded by `seed`.
    pub fn shuffle(&mut self, seed: u64) {
        let n = self.items.len();
        self.order = (0..n).collect();
        let mut state = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        for i in (1..n).rev() {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let j = (state >> 33) as usize % (i + 1);
            self.order.swap(i, j);
        }
        self.order_cursor = 0;
    }

    /// Index into items for the current position.
    pub fn current(&self) -> Option<&String> {
        let &oidx = self.order.get(self.order_cursor)?;
        self.items.get(oidx)
    }
    pub fn current_index(&self) -> Option<usize> { self.order.get(self.order_cursor).copied() }

    pub fn next(&mut self) {
        if self.order.is_empty() { return; }
        self.order_cursor = (self.order_cursor + 1) % self.order.len();
    }
    pub fn prev(&mut self) {
        if self.order.is_empty() { return; }
        self.order_cursor = (self.order_cursor + self.order.len() - 1) % self.order.len();
    }
    pub fn remove(&mut self, id: &str) {
        if let Some(pos) = self.items.iter().position(|x| x == id) {
            self.items.remove(pos);
            self.order = (0..self.items.len()).collect();
            if self.order_cursor >= self.order.len() && !self.order.is_empty() {
                self.order_cursor = 0;
            }
        }
    }
    pub fn clear(&mut self) {
        self.items.clear();
        self.order.clear();
        self.order_cursor = 0;
    }
}
```

- [ ] **Step 4: Implement `src/player.rs`**

```rust
use anyhow::Result;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use crate::config::PlayerKind;

pub trait Player {
    fn load(&mut self, path: &Path) -> Result<()>;
    fn play_pause(&mut self) -> Result<()>;
    fn seek(&mut self, secs: f64) -> Result<()>;
    fn stop(&mut self) -> Result<()>;
    fn position(&self) -> Option<f64>;
    fn duration(&self) -> Option<f64>;
    fn is_playing(&self) -> bool;
}

// ---------- Stub (tests / dry-run) ----------
#[derive(Default)]
pub struct StubPlayer {
    loaded: Option<std::path::PathBuf>,
    playing: bool,
    pos: f64,
    dur: f64,
}
impl StubPlayer {
    pub fn loaded(&self) -> Option<std::path::PathBuf> { self.loaded.clone() }
}
impl Player for StubPlayer {
    fn load(&mut self, path: &Path) -> Result<()> { self.loaded = Some(path.to_path_buf()); self.playing = true; self.pos = 0.0; self.dur = 180.0; Ok(()) }
    fn play_pause(&mut self) -> Result<()> { self.playing = !self.playing; Ok(()) }
    fn seek(&mut self, secs: f64) -> Result<()> { self.pos = (self.pos + secs).max(0.0).min(self.dur); Ok(()) }
    fn stop(&mut self) -> Result<()> { self.playing = false; Ok(()) }
    fn position(&self) -> Option<f64> { Some(self.pos) }
    fn duration(&self) -> Option<f64> { Some(self.dur) }
    fn is_playing(&self) -> bool { self.playing }
}

// ---------- afplay fallback (per-track, no seek) ----------
pub struct AfplayPlayer { child: Option<Child> }
impl AfplayPlayer {
    pub fn new() -> Self { Self { child: None } }
}
impl Player for AfplayPlayer {
    fn load(&mut self, path: &Path) -> Result<()> {
        self.child = Some(Command::new("afplay").arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?);
        Ok(())
    }
    fn play_pause(&mut self) -> Result<()> { Ok(()) } // afplay has no IPC
    fn seek(&mut self, _secs: f64) -> Result<()> { Ok(()) }
    fn stop(&mut self) -> Result<()> { if let Some(mut c) = self.child.take() { let _ = c.kill(); } Ok(()) }
    fn position(&self) -> Option<f64> { None }
    fn duration(&self) -> Option<f64> { None }
    fn is_playing(&self) -> bool { self.child.as_ref().map(|c| c.try_wait().ok().flatten().is_none()).unwrap_or(false) }
}

// ---------- mpv over Unix socket ----------
pub struct MpvPlayer {
    child: Child,
    sock: std::path::PathBuf,
    conn: Option<std::os::unix::net::UnixStream>,
}
impl MpvPlayer {
    pub fn spawn(socket: &Path) -> Result<Self> {
        let _ = std::fs::remove_file(socket);
        let child = Command::new("mpv")
            .args(["--no-video", "--no-terminal", "--idle", "--gapless-audio=yes"])
            .arg(format!("--input-ipc-server={}", socket.display()))
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn()?;
        // wait for socket to appear (up to 2s)
        for _ in 0..20 {
            if socket.exists() { break; }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let conn = std::os::unix::net::UnixStream::connect(socket).ok();
        Ok(MpvPlayer { child, sock: socket.to_path_buf(), conn })
    }

    fn send(&mut self, cmd: &[serde_json::Value]) -> Result<()> {
        use std::io::Write;
        if let Some(c) = self.conn.as_mut() {
            let msg = serde_json::json!({ "command": cmd });
            writeln!(c, "{}", msg)?;
            c.flush()?;
        }
        Ok(())
    }
}
impl Player for MpvPlayer {
    fn load(&mut self, path: &Path) -> Result<()> {
        self.send(&["loadfile".into(), path.to_string_lossy().into()])?;
        Ok(())
    }
    fn play_pause(&mut self) -> Result<()> {
        // toggle via property read; simplest: set pause based on is_playing probe
        self.send(&["set".into(), "pause".into(), "toggle".into()])?;
        Ok(())
    }
    fn seek(&mut self, secs: f64) -> Result<()> {
        self.send(&["seek".into(), secs.into(), "relative".into()])?;
        Ok(())
    }
    fn stop(&mut self) -> Result<()> {
        let _ = self.send(&["quit".into()]);
        let _ = self.child.kill();
        Ok(())
    }
    fn position(&self) -> Option<f64> { None }   // polled in TUI via get_property (future)
    fn duration(&self) -> Option<f64> { None }
    fn is_playing(&self) -> bool { self.child.try_wait().ok().flatten().is_none() }
}
impl Drop for MpvPlayer {
    fn drop(&mut self) { let _ = self.child.kill(); let _ = std::fs::remove_file(&self.sock); }
}

pub fn launch(kind: PlayerKind, socket: &Path) -> Box<dyn Player> {
    match kind {
        PlayerKind::Mpv => match MpvPlayer::spawn(socket) {
            Ok(p) => Box::new(p),
            Err(_) => Box::new(AfplayPlayer::new()),
        },
        PlayerKind::Afplay => Box::new(AfplayPlayer::new()),
    }
}
```

- [ ] **Step 5: Register modules**

Add `pub mod player;` and `pub mod tui;` (with `tui/mod.rs` declaring `pub mod queue;`) to `src/main.rs`. Create `src/tui/mod.rs`:

```rust
pub mod queue;
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --test player`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
cd ~/Dev/jukebox
git add src/player.rs src/tui/ tests/player.rs src/main.rs
git commit -m "feat(player): Player trait + StubPlayer + mpv IPC + afplay fallback + Queue"
```

---

## Task 12: TUI scaffold — Artists pane + event loop + Artists flow

**Files:**
- Create: `src/tui/mod.rs` (replace stub), `src/tui/view.rs`, `tests/tui.rs`
- Produces: `App` struct, `App::run()`, Artists pane rendering + selection + enqueue.

**Interfaces:**
- Consumes: `Catalog`, `Searcher` (Task 9), `Player`/`launch` (Task 11), `Queue` (Task 11), `Config` (Task 1).
- Produces (used by Task 13): `App` with fields accessible to add Search + Queue panes.

- [ ] **Step 1: Write the failing test `tests/tui.rs`**

```rust
use jukebox::catalog::Catalog;
use jukebox::tui::App;

fn mini_catalog_json() -> String {
    serde_json::json!({
        "version":1,"built_at":"x","source_root":"/tmp/lossless",
        "tracks":[
          {"id":"t1","artists":["Ado"],"primary_artist":"Ado","title":"Freedom",
           "bit_depth":24,"sample_rate_hz":48000,"source_path":"lossless/a/01.flac","symlinked_into_artists":["Ado"]},
          {"id":"t2","artists":["Aimer"],"primary_artist":"Aimer","title":"Brave",
           "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/b/01.flac","symlinked_into_artists":["Aimer"]},
        ]
    }).to_string()
}

#[test]
fn app_builds_artist_index() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, mini_catalog_json()).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()));
    let artists = app.artists();
    assert!(artists.iter().any(|a| a == "Ado"));
    assert!(artists.iter().any(|a| a == "Aimer"));
}

#[test]
fn enqueue_artist_adds_their_tracks() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, mini_catalog_json()).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let mut app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()));
    app.enqueue_artist("Ado");
    assert_eq!(app.queue().len(), 1);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test tui`
Expected: FAIL — `jukebox::tui::App` not found.

- [ ] **Step 3: Implement `src/tui/mod.rs`**

```rust
pub mod queue;
pub mod view;

use crate::catalog::Catalog;
use crate::player::Player;
use std::collections::BTreeMap;

pub enum Pane { Artists, Search, Queue }

pub struct App {
    pub catalog: Catalog,
    pub player: Box<dyn Player>,
    pub queue: queue::Queue,
    pub artists: Vec<String>,                       // sorted unique artist names
    pub artist_index: BTreeMap<String, Vec<usize>>, // artist -> track indices
    pub artist_cursor: usize,
    pub focus: Pane,
    pub search_input: String,
    pub results: Vec<(f32, usize)>,                  // (score, track_index)
    pub result_cursor: usize,
    pub should_quit: bool,
}

impl App {
    pub fn new(catalog: Catalog, player: Box<dyn Player>) -> Self {
        let mut idx: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, t) in catalog.tracks.iter().enumerate() {
            for a in &t.symlinked_into_artists {
                idx.entry(a.clone()).or_default().push(i);
            }
        }
        let artists: Vec<String> = idx.keys().cloned().collect();
        App {
            catalog, player, queue: queue::Queue::new(),
            artists, artist_index: idx,
            artist_cursor: 0, focus: Pane::Artists,
            search_input: String::new(), results: Vec::new(), result_cursor: 0,
            should_quit: false,
        }
    }

    pub fn artists(&self) -> &Vec<String> { &self.artists }
    pub fn queue(&self) -> &queue::Queue { &self.queue }

    pub fn enqueue_artist(&mut self, artist: &str) {
        if let Some(tracks) = self.artist_index.get(artist) {
            for &i in tracks {
                self.queue.enqueue(self.catalog.tracks[i].id.clone());
            }
        }
    }

    /// Run the terminal event loop. Returns when the user quits.
    pub fn run(&mut self) -> anyhow::Result<()> {
        use crossterm::event::{self, Event, KeyCode, KeyEventKind};
        use crossterm::execute;
        use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
        use ratatui::backend::CrosstermBackend;
        use ratatui::Terminal;

        terminal::enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut term = Terminal::new(backend)?;

        while !self.should_quit {
            term.draw(|f| view::draw(f, self))?;
            if let Ok(ev) = event::poll(std::time::Duration::from_millis(200))
                .then(|| event::read())
            {
                if let Event::Key(k) = ev {
                    if k.kind != KeyEventKind::Press { continue; }
                    self.handle_key(k.code);
                }
            }
        }

        terminal::disable_raw_mode()?;
        execute!(std::io::stdout(), LeaveAlternateScreen)?;
        Ok(())
    }

    fn handle_key(&mut self, code: crossterm::event::KeyCode) {
        use crossterm::event::KeyCode::*;
        match code {
            Tab => self.focus = match self.focus {
                Pane::Artists => Pane::Search, Pane::Search => Pane::Queue, Pane::Queue => Pane::Artists,
            },
            Char('q') => { self.should_quit = true; self.player.stop().ok(); }
            Down => self.cursor_down(),
            Up => self.cursor_up(),
            Char(' ') if matches!(self.focus, Pane::Artists) => {
                if let Some(a) = self.artists.get(self.artist_cursor).cloned() {
                    self.enqueue_artist(&a);
                }
            }
            _ => {}
        }
    }

    fn cursor_down(&mut self) {
        match self.focus {
            Pane::Artists => { if self.artist_cursor + 1 < self.artists.len() { self.artist_cursor += 1; } }
            _ => {}
        }
    }
    fn cursor_up(&mut self) {
        match self.focus {
            Pane::Artists => { if self.artist_cursor > 0 { self.artist_cursor -= 1; } }
            _ => {}
        }
    }
}
```

- [ ] **Step 4: Implement `src/tui/view.rs`**

```rust
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::tui::{App, Pane};

pub fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(50), Constraint::Percentage(25)])
        .split(f.area());

    // Artists
    let items: Vec<ListItem> = app.artists.iter().enumerate()
        .map(|(i, a)| {
            let style = if i == app.artist_cursor { Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD) } else { Style::default() };
            ListItem::new(a.as_str()).style(style)
        }).collect();
    let list = List::new(items).block(border("Artists", matches!(app.focus, Pane::Artists)));
    f.render_widget(list, chunks[0]);

    // Search
    let search = Paragraph::new(format!("Search: {}", app.search_input))
        .block(border("Search", matches!(app.focus, Pane::Search)));
    f.render_widget(search, chunks[1]);

    // Queue
    let q: Vec<ListItem> = app.queue().items().iter()
        .map(|id| ListItem::new(id.as_str())).collect();
    let qlist = List::new(q).block(border("Queue", matches!(app.focus, Pane::Queue)));
    f.render_widget(qlist, chunks[2]);
}

fn border<'a>(title: &'a str, focused: bool) -> Block<'a> {
    let style = if focused { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::DarkGray) };
    Block::default().borders(Borders::ALL).title(title).border_style(style)
}
```

- [ ] **Step 5: Register `pub mod tui;` (already done) and run tests**

Run: `cargo test --test tui`
Expected: PASS — both tests green.

- [ ] **Step 6: Commit**

```bash
cd ~/Dev/jukebox
git add src/tui/ tests/tui.rs src/main.rs
git commit -m "feat(tui): App scaffold + Artists pane + event loop"
```

---

## Task 13: Search + Queue panes, keybindings, playback wiring

**Files:**
- Modify: `src/tui/mod.rs`, `src/tui/view.rs`, `src/main.rs` (wire `Play`)
- Create: append to `tests/tui.rs`
- Produces: full keybinding set, live search results, queue play/skip/shuffle.

**Interfaces:**
- Consumes: `Searcher` (Task 10), `Queue` ops (Task 11).

- [ ] **Step 1: Write the failing tests (append to `tests/tui.rs`)**

```rust
use jukebox::search::Searcher;

fn build_catalog_and_index() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, mini_catalog_json()).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let idx = d.path().join("search-index");
    jukebox::search::build_index(&cat, &idx).unwrap();
    (d, cat)
}

#[test]
fn search_populates_results() {
    let (_d, cat) = build_catalog_and_index();
    let s = Searcher::open(&_d.path().join("search-index")).unwrap();
    let hits = s.search("Freedom", 10).unwrap();
    assert!(!hits.is_empty());
}

#[test]
fn enqueue_results_then_next_advances() {
    let (_d, cat) = build_catalog_and_index();
    let mut app = jukebox::tui::App::new(cat, Box::new(jukebox::player::StubPlayer::default()));
    app.queue.enqueue("t1".into());
    app.queue.enqueue("t2".into());
    assert_eq!(app.queue.current().map(|s| s.clone()), Some("t1".to_string()));
    app.queue.next();
    assert_eq!(app.queue.current().map(|s| s.clone()), Some("t2".to_string()));
}
```

- [ ] **Step 2: Run the tests to verify they fail/pass**

Run: `cargo test --test tui`
Expected: the search-populates test passes (Searcher already works); the enqueue test should pass already. Any failing assertion guides the fix.

- [ ] **Step 3: Extend `App` with a `Searcher` field + search wiring**

In `src/tui/mod.rs`, add an optional searcher and result handling:

```rust
pub struct App {
    // ... existing fields ...
    pub searcher: Option<crate::search::Searcher>,
}
```

Update `App::new` to accept an optional searcher:
```rust
pub fn new(catalog: Catalog, player: Box<dyn Player>, searcher: Option<crate::search::Searcher>) -> Self {
    // ... same body, add `searcher,` to the struct literal
}
```

**Update the Task 12 tests:** the `App::new(cat, player)` calls in `tests/tui.rs` now need a third arg — pass `None`:
```rust
let app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()), None);
let mut app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()), None);
```
Re-run `cargo test --test tui` after this change to confirm Task 12's two tests still pass.

Add a method to run a search and a method to enqueue a result:
```rust
pub fn run_search(&mut self) {
    if let Some(s) = self.searcher.as_ref() {
        if let Ok(hits) = s.search(&self.search_input, 50) {
            self.results = hits.into_iter().filter_map(|h| {
                self.catalog.tracks.iter().position(|t| t.id == h.track_id).map(|p| (h.score, p))
            }).collect();
            self.result_cursor = 0;
        }
    }
}
pub fn enqueue_current_result(&mut self) {
    if let Some(&(_, idx)) = self.results.get(self.result_cursor) {
        self.queue.enqueue(self.catalog.tracks[idx].id.clone());
    }
}
pub fn play_current_queue(&mut self) {
    if let Some(id) = self.queue.current().cloned() {
        if let Some(t) = self.catalog.tracks.iter().find(|t| t.id == id) {
            let path = t.resolve_source(&self.catalog.source_root);
            let _ = self.player.load(&path);
        }
    }
}
```

- [ ] **Step 4: Extend key handling in `handle_key`**

```rust
Char('/') => { self.focus = Pane::Search; self.search_input.clear(); }
Char(c) if matches!(self.focus, Pane::Search) => { self.search_input.push(c); self.run_search(); }
Backspace if matches!(self.focus, Pane::Search) => { self.search_input.pop(); self.run_search(); }
Enter if matches!(self.focus, Pane::Search) => self.enqueue_current_result(),
Enter if matches!(self.focus, Pane::Queue) => { self.play_current_queue(); }
Char('s') => self.queue.shuffle(42),
Char('S') => { self.queue.shuffle(42); self.queue.next(); self.play_current_queue(); }
Char('r') if matches!(self.focus, Pane::Queue) => {
    if let Some(id) = self.queue.current().cloned() { self.queue.remove(&id); }
}
Char('c') if matches!(self.focus, Pane::Queue) => self.queue.clear(),
Char('n') => { self.queue.next(); self.play_current_queue(); }
Char('p') => { self.queue.prev(); self.play_current_queue(); }
Left => { let _ = self.player.seek(-5.0); }
Right => { let _ = self.player.seek(5.0); }
Down => self.cursor_down(),
Up => self.cursor_up(),
```

Update `cursor_down`/`cursor_up` to also move in `Search` (result_cursor) and `Queue`.

- [ ] **Step 5: Render results + queue current marker in `view.rs`**

Replace the Search and Queue widget construction with concrete rendering of `app.results` (as `score% title — artist`) and a `▶` on the current queue item:

```rust
// Search pane: input line + ranked results
let mut lines: Vec<ListItem> = vec![ListItem::new(format!("/ {}", app.search_input))];
for (i, (score, tidx)) in app.results.iter().enumerate() {
    let t = &app.catalog.tracks[*tidx];
    let label = format!("{:>3.0}%  {} — {}", score * 100.0, t.title, t.primary_artist);
    let style = if i == app.result_cursor {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else { Style::default() };
    lines.push(ListItem::new(label).style(style));
}
let list = List::new(lines).block(border("Search", matches!(app.focus, Pane::Search)));
f.render_widget(list, chunks[1]);

// Queue pane: items with ▶ on the current
let cur = app.queue.current_index();
let qitems: Vec<ListItem> = app.queue().items().iter().enumerate()
    .map(|(i, id)| {
        let prefix = if Some(i) == cur { "▶ " } else { "  " };
        let track = app.catalog.tracks.iter().find(|t| &t.id == id);
        let label = match track {
            Some(t) => format!("{prefix}{} — {}", t.title, t.primary_artist),
            None => format!("{prefix}{id}"),
        };
        ListItem::new(label)
    }).collect();
let qlist = List::new(qitems).block(border("Queue", matches!(app.focus, Pane::Queue)));
f.render_widget(qlist, chunks[2]);
```

This replaces the `search`/`qlist` construction in the existing `draw` function (keep the Artists pane and the `chunks` layout unchanged).

- [ ] **Step 6: Wire `jukebox play` in `src/main.rs`**

```rust
cli::Cmd::Play => {
    let cfg = cli::ensure_config()?;
    let cat = catalog::Catalog::load(&cfg.filtered_dir.join("catalog.json"))?;
    let searcher = search::Searcher::open(&cfg.filtered_dir.join("search-index")).ok();
    let player = player::launch(cfg.player, &cfg.mpv_socket);
    let mut app = tui::App::new(cat, player, searcher);
    app.run()?;
}
```

- [ ] **Step 7: Run the full test suite**

Run: `cargo test`
Expected: PASS across config, catalog, translit, search, player, tui.

- [ ] **Step 8: Commit**

```bash
cd ~/Dev/jukebox
git add src/tui/ src/main.rs tests/tui.rs
git commit -m "feat(tui): search + queue panes, keybindings, mpv playback wiring"
```

---

## Task 14: `jukebox sync` orchestration + smoke test

**Files:**
- Modify: `src/main.rs` (wire `Sync`), `scripts/standardize.sh` path resolution from config
- Produces: `jukebox sync` runs `standardize.sh` then `jukebox index`.

**Interfaces:**
- Consumes: `standardize.sh` (Task 4), `build_index` (Task 9).

- [ ] **Step 1: Wire the `Sync` arm in `src/main.rs`**

```rust
cli::Cmd::Sync => {
    let cfg = cli::ensure_config()?;
    let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scripts/standardize.sh");
    // Fall back to a sibling-of-binary location if running installed.
    let script = if script.exists() { script } else {
        std::env::current_exe()?.parent().unwrap().join("scripts/standardize.sh")
    };
    let status = std::process::Command::new(&script)
        .args(["--source", &cfg.source_dir.display().to_string(),
               "--out", &cfg.filtered_dir.display().to_string()])
        .status()?;
    if !status.success() { anyhow::bail!("standardize.sh failed"); }
    let cat = catalog::Catalog::load(&cfg.filtered_dir.join("catalog.json"))?;
    search::build_index(&cat, &cfg.filtered_dir.join("search-index"))?;
    println!("synced: {} tracks", cat.tracks.len());
}
```

- [ ] **Step 2: Smoke test against the real library**

Run (manually, against the user's actual library — not a unit test):
```bash
cd ~/Dev/jukebox
cargo run -- config   # first-run prompt; point at ~/Music/lossless
cargo run -- sync
cargo run -- search "Freedom"
cargo run -- play
```
Expected: `sync` prints a track count; `search` prints ranked results; `play` launches the TUI and mpv plays the selected track.

- [ ] **Step 3: Commit**

```bash
cd ~/Dev/jukebox
git add src/main.rs
git commit -m "feat(cli): jukebox sync orchestrates standardize.sh + index"
```

---

## Task 15: README + final integration verification

**Files:**
- Create: `README.md`
- Produces: documented usage + a clean full-suite pass.

- [ ] **Step 1: Write `README.md`**

```markdown
# jukebox

A filtered-lossless jukebox: a bash indexer that dedupes a FLAC library into a
symlinked, artist-organized view, plus a Rust TUI with model-free multilingual
search and mpv playback.

## Setup

```bash
cargo build --release
./target/release/jukebox          # first run prompts for your lossless source dir
./target/release/jukebox sync     # build symlinks + catalog + search index
./target/release/jukebox play     # launch the TUI
```

Config lives at `~/.config/jukebox/config.yml` (or `$XDG_CONFIG_HOME/jukebox/`).

## Commands

| command | what it does |
|---|---|
| `jukebox` / `jukebox play` | launch the TUI |
| `jukebox sync` | run `scripts/standardize.sh` then rebuild the search index |
| `jukebox index` | rebuild the Tantivy search index from `catalog.json` |
| `jukebox search <query>` | one-shot CLI search |
| `jukebox config` | show / re-run the config prompt |

## Keybindings (TUI)

`Tab` panes · `↑/↓` move · `/` search · `space` multi-select / enqueue artist ·
`enter` enqueue / play-now · `s`/`S` shuffle · `r` remove · `c` clear ·
`n`/`p` next/prev · `←/→` seek ±5s · `q` quit (kills mpv) · `Q` quit leaving mpv.

## Design

See `specs/2026-07-04-filtered-lossless-jukebox-design.md` and
`specs/2026-07-04-jukebox-implementation-plan.md`.
```

- [ ] **Step 2: Run the entire suite**

Run: `cd ~/Dev/jukebox && cargo test && bats scripts/test/*.bats`
Expected: all green.

- [ ] **Step 3: Commit**

```bash
cd ~/Dev/jukebox
git add README.md
git commit -m "docs: README with setup, commands, keybindings"
```

---

## Notes for the implementer

- **First build is slow** — `lindera` with `embed-ipadic` compiles+embeds the IPADIC dictionary (~30–60s). This is expected; subsequent builds are cached.
- **`wana_kana` trait method quirk** — `is_kana()` etc. are defined on `&str` via traits; per-char checks in `translit.rs` use a 1-char string slice (`std::str::from_chars`). If that helper isn't stable, replace with explicit Unicode-range checks (`has_katakana`/`has_hiragana` already do this).
- **mpv socket readiness** — `MpvPlayer::spawn` polls up to 2s for the socket. If mpv isn't installed, `launch()` falls back to `AfplayPlayer` automatically.
- **`standardize.sh` grouping** — Task 4 has a deliberate dead-code block to delete (Step 4). Don't skip it, or associative arrays won't survive the `while | sort` subshell.
- **Source path resolution** — `catalog.json`'s `source_path` is relative to the *parent* of `source_root` (matches the spec example `lossless/...`). `Track::resolve_source` joins onto `source_root.parent()`.
