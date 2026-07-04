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
