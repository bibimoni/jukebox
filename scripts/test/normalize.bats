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
