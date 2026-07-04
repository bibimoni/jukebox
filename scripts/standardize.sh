#!/usr/bin/env bash
# standardize.sh — dedupe + symlink + catalog builder over lossless/.
# Spec: ~/Dev/jukebox/specs/2026-07-04-filtered-lossless-jukebox-design.md §1.
#
# Written to run on the macOS system bash (3.2): no associative arrays,
# no mapfile/readarray, no globstar. Grouping state is kept in temp files
# (winners.tsv / losers.tsv) keyed by dedup_key instead.
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

# Resolve the config path the same way the Rust side (config::config_path)
# does: honor $XDG_CONFIG_HOME; otherwise on macOS use
# ~/Library/Application Support/jukebox/config.yml (matching dirs::config_dir),
# else ~/.config/jukebox/config.yml. bash 3.2 compatible.
config_path() {
  if [[ -n "${XDG_CONFIG_HOME:-}" ]]; then
    printf '%s' "$XDG_CONFIG_HOME/jukebox/config.yml"
  elif [[ "$(uname)" == "Darwin" ]]; then
    printf '%s' "$HOME/Library/Application Support/jukebox/config.yml"
  else
    printf '%s' "$HOME/.config/jukebox/config.yml"
  fi
}
CONFIG="$(config_path)"
read_cfg() { # <key> <default>
  if [[ -f "$CONFIG" ]] && command -v yq >/dev/null; then
    yq ".$1" "$CONFIG" 2>/dev/null
  else echo "$2"; fi
}

[[ -z "$SOURCE" ]] && SOURCE="${LOSSLESS_SOURCE:-$(read_cfg source_dir "$HOME/Music/lossless")}"
[[ -z "$OUT" ]]    && OUT="${LOSSLESS_FILTERED:-$(read_cfg filtered_dir "$HOME/Music/filtered_lossless")}"

# --- guards ---
[[ -d "$SOURCE" ]] || { echo "source dir not found: $SOURCE" >&2; exit 1; }
find "$SOURCE" -type f -iname '*.flac' -print -quit | grep -q . \
  || { echo "no .flac in $SOURCE" >&2; exit 1; }

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
# Relative path from a symlink's directory to its target (relative symlink).
rel_target() { # <abs_target> <abs_symlink_dir> -> relative path from dir to target
  python3 - "$1" "$2" <<'PY'
import os,sys
print(os.path.relpath(sys.argv[1], start=sys.argv[2]))
PY
}

# Byte-safe truncation that respects UTF-8 char boundaries.
# macOS filenames are capped at 255 bytes; a track listing dozens of
# artists in its ARTIST tag produces a canon far exceeding that.
truncate_bytes() { # <string> <max_bytes>
  python3 - "$1" "$2" <<'PY'
import sys
s, n = sys.argv[1], int(sys.argv[2])
b = s.encode('utf-8')
if len(b) <= n:
    print(s); sys.exit()
while n > 0:
    try:
        print(b[:n].decode('utf-8')); break
    except UnicodeDecodeError:
        n -= 1
PY
}

# --- probe every flac ---
shopt -s nullglob
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# records.tsv: dedup_key \t bit_depth \t sample_rate \t isrc \t tidal \t ntags \t path
RECORDS="$TMP/records.tsv"
SORTED="$TMP/sorted.tsv"
WINNERS="$TMP/winners.tsv"   # dedup_key \t winner-line
LOSERS="$TMP/losers.tsv"     # dedup_key \t loser-line (one row per loser)
: > "$RECORDS"

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
  # If ffprobe yields nothing usable (corrupt/non-flac file, or ffprobe
  # missing), `read` hits EOF and returns 1 — or, on a malformed file, ffprobe
  # may emit garbage like `0,N/A`. Per spec §Error Handling ("ffprobe failure:
  # skip track, log, continue") we log the file and skip it rather than
  # indexing it with bogus defaults. Validate that sr/bd are positive integers.
  local sr bd
  if ! read sr bd < <(ffprobe -v error -show_entries stream=sample_rate,bits_per_raw_sample \
        -of csv=p=0 "$flac" 2>/dev/null | head -1 | awk -F',' '{print $1, ($2==""?"16":$2)}') \
     || [[ -z "$sr" ]] || ! [[ "$sr" =~ ^[0-9]+$ ]] || [[ "$sr" -le 0 ]] \
     || ! [[ "$bd" =~ ^[0-9]+$ ]] || [[ "$bd" -le 0 ]]; then
    printf 'SKIP\tffprobe-empty\t%s\n' "$flac" >&3
    return 1
  fi
  sr="${sr:-44100}"; bd="${bd:-16}"

  local ntags
  ntags="$(metaflac --export-tags-to=- "$flac" 2>/dev/null | grep -c '=' || true)"

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$(dedup_key "$artists" "$title")" "$bd" "$sr" "$isrc" "$tidal" "$ntags" "$flac"
}

while IFS= read -r -d '' flac; do
  probe "$flac" >> "$RECORDS" || true   # probe returns 1 on skip (already logged)
done < <(find "$SOURCE" -type f -iname '*.flac' -print0)

# Sort records by dedup_key (field 1) so all candidates of a song are adjacent.
sort -t$'\t' -k1,1 "$RECORDS" > "$SORTED"
: > "$WINNERS"; : > "$LOSERS"

# --- group consecutive identical keys, pick winner per group ---
# Process substitution preserves shell variables (a `| while` subshell would
# not), but we write straight to temp files so there is no state to lose.
prev_key=""
bucket_file="$TMP/bucket.tsv"
: > "$bucket_file"
flush_bucket() { # <key>
  [[ ! -s "$bucket_file" ]] && return
  local win
  win="$(cat "$bucket_file" | pick_winner)"
  if [[ -n "$win" ]]; then
    printf '%s\t%s\n' "$1" "$win" >> "$WINNERS"
    local l
    while IFS= read -r l; do
      [[ -z "$l" ]] && continue
      [[ "$l" != "$win" ]] && printf '%s\t%s\n' "$1" "$l" >> "$LOSERS"
    done < "$bucket_file"
  fi
  : > "$bucket_file"
}

while IFS=$'\t' read -r key rest; do
  if [[ "$key" != "$prev_key" && -n "$prev_key" ]]; then
    flush_bucket "$prev_key"
  fi
  printf '%s\n' "$rest" >> "$bucket_file"
  prev_key="$key"
done < "$SORTED"
flush_bucket "$prev_key"

# --- emit symlinks + catalog.json + duplicates.log ---
tracks_json='[]'
emit_winner() { # <dedup_key> <winner-line>
  local key="$1" line="$2"
  [[ -z "$line" ]] && return
  # winner-line is tab-separated: bd \t sr \t isrc \t tidal \t ntags \t path
  # bash `read` collapses consecutive tabs (empty isrc/tidal fields) because
  # tab is an IFS-whitespace char, so extract with `cut` which preserves empties.
  local bd sr isrc path
  bd="$(cut -f1 <<<"$line")"
  sr="$(cut -f2 <<<"$line")"
  isrc="$(cut -f3 <<<"$line")"
  path="$(cut -f6 <<<"$line")"
  # re-read tags from the winner for the catalog record
  local artists title album track disc
  artists="$(read_tag "$path" ARTIST)"; [[ -z "$artists" ]] && artists="$(read_tag "$path" ALBUMARTIST)"
  title="$(read_tag "$path" TITLE)"; [[ -z "$title" ]] && title="$(basename "$path" .flac)"
  album="$(read_tag "$path" ALBUM)"; track="$(read_tag "$path" TRACKNUMBER)"; disc="$(read_tag "$path" DISCNUMBER)"

  local canon
  canon="$(normalize_artist_string "$artists")"
  # split into per-artist folders (bash 3.2: no mapfile, use a while-read into an array)
  local -a arts=()
  local _a
  while IFS= read -r _a; do arts+=("$_a"); done < <(split_artists "$artists")
  [[ ${#arts[@]} -eq 0 ]] && arts=("[unknown artist]")

  local khz; khz="$(khz_label "$sr")"
  # Cap the canon so the full "canon - title [bd-bit khz].flac" stays under
  # the 255-byte filename limit. 180 bytes leaves headroom for title + bracket + ext.
  local canon_trunc; canon_trunc="$(truncate_bytes "$canon" 180)"
  local fname; fname="$(sanitize_filename "$canon_trunc - $title [${bd}bit-${khz}].flac")"
  # Final safety net: a long title can still push the name over the 255-byte
  # macOS limit. `${#fname}` counts *characters*, not bytes, so for CJK names
  # (byte-heavy, char-light) it never trips — always clamp by byte length here.
  # `truncate_bytes` is a no-op under the limit. 240-byte stem + `.flac` (5) =
  # 245, leaving headroom for the ` (2)` collision suffix added below.
  local stem="${fname%.flac}"
  fname="$(truncate_bytes "$stem" 240).flac"

  local symlinked='[]'
  local artist dir relpath linkpath
  for artist in "${arts[@]}"; do
    dir="$OUT_ABS/$(sanitize_filename "$artist")"
    mkdir -p "$dir"
    # collision handling: append " (2)", " (3)" ...
    linkpath="$dir/$fname"; local n=2
    while [[ -e "$linkpath" || -L "$linkpath" ]]; do
      linkpath="$dir/${fname%.flac} ($n).flac"; n=$((n+1))
    done
    relpath="$(rel_target "$path" "$dir")"
    ln -s "$relpath" "$linkpath"
    symlinked="$(echo "$symlinked" | jq -c --arg a "$artist" '. += [$a]')"
  done

  local id; id="$(printf '%s' "$key" | shasum -a 256 | cut -c1-16)"

  # source_path relative to the PARENT of source_root (matches spec example
  # `lossless/...`); Rust Track::resolve_source joins onto source_root.parent().
  local src_rel; src_rel="lossless/$(rel_target "$path" "$SRC_ABS")"

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
       album:($album|select(.>"") // null),
       track_number:($track|tonumber? // null),
       disc_number:($disc|tonumber? // null),
       bit_depth:($bd|tonumber), sample_rate_hz:($sr|tonumber),
       isrc:($isrc|select(.>"") // null),
       source_path:$source, symlinked_into_artists:$symlinked
     }]')"
}

while IFS=$'\t' read -r wkey wline; do
  [[ -z "$wkey" ]] && continue
  emit_winner "$wkey" "$wline"
done < "$WINNERS"

# losers log — lline is tab-separated: bd \t sr \t isrc \t tidal \t ntags \t path
while IFS=$'\t' read -r lkey lline; do
  [[ -z "$lkey" ]] && continue
  _lbd="$(cut -f1 <<<"$lline")"
  _lsr="$(cut -f2 <<<"$lline")"
  _lisrc="$(cut -f3 <<<"$lline")"
  _lpath="$(cut -f6 <<<"$lline")"
  printf 'DUP\t%s\t%s/%s\t%s\t%s\n' "$lkey" "$_lbd" "$_lsr" "$_lisrc" "$_lpath" >> "$OUT/duplicates.log"
done < "$LOSERS"

jq -n --argjson tracks "$tracks_json" --arg src "$SRC_ABS" --arg at "$(date -u +%FT%TZ)" \
  '{version:1, built_at:$at, source_root:$src, tracks:$tracks}' > "$OUT/catalog.json"

echo "indexed $(echo "$tracks_json" | jq 'length') unique tracks" >&3
