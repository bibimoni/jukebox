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
