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
  [ "$(find "$OUT/Ado" -type l | wc -l | tr -d ' ')" = "1" ]
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

@test "non-flac/corrupt .flac file is skipped without aborting" {
  # one valid flac + one bogus .flac-named file (plain bytes, not a flac)
  mkflac "$SRC" "good.flac" "ARTIST=Ado" "TITLE=Freedom"
  printf 'not a flac' > "$SRC/bad.flac"
  run "$PROJECT_ROOT/scripts/standardize.sh" --source "$SRC" --out "$OUT"
  [ "$status" -eq 0 ]
  # valid track was indexed
  [ -L "$OUT/Ado/Ado - Freedom [16bit-44.1kHz].flac" ]
  # bogus file was skipped + logged, not indexed
  grep -q 'SKIP	ffprobe-empty	'"$SRC"'/bad.flac' "$OUT/_build.log"
  [ "$(jq '.tracks | length' "$OUT/catalog.json")" = "1" ]
}
