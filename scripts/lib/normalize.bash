#!/usr/bin/env bash
# Pure helper functions for standardize.sh. Sourcing has no side effects.
# All functions print to stdout; no global mutation.

# Replace every artist separator with ', ' and trim/collapse whitespace.
# Separators (spec §1.2): ; , × / + &  (and full-width × U+00D7, × U+00D7).
normalize_artist_string() {
  local s="$1"
  # First split into newline-joined parts, then re-join with ', '.
  # `paste -sd` cycles through delimiter chars, so it cannot emit the
  # two-char ', ' between every pair; use awk to join instead.
  split_artists "$s" | awk 'NR>1{printf ", "} {printf "%s", $0} END{printf "\n"}'
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

# Sanitize a string for use as a filename. Replace \0 with _, / and : with -,
# strip leading dots, collapse whitespace runs. Preserve unicode.
sanitize_filename() {
  printf '%s' "$1" \
    | tr '\0' '_' \
    | tr '/' '-' \
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
  local want
  want="$(printf '%s' "$tag" | tr '[:upper:]' '[:lower:]')"
  metaflac --export-tags-to=- "$flac" 2>/dev/null \
    | awk -F'=' -v want="$want" '
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
