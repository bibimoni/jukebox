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

# Skip a test when metaflac is not installed (integration tests need real flacs).
skip_if_no_metaflac() {
  command -v metaflac >/dev/null || skip "metaflac not installed"
}
