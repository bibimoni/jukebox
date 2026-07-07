# YouTube Integration — Strict Judge Rubric

The implementation is not concluded until a fresh, isolated judge agent scores
the finished app against this rubric and approves **every dimension at max**.
Anything below max triggers a diagnose → fix → re-judge loop until all-max or
the user intervenes.

Spec: `docs/superpowers/specs/2026-07-08-youtube-integration-design.md`

## Scoring

Each dimension is 0–2: **2 (max)** = meets the bar; **1 (mid)** = works but
rough; **0 (zero)** = broken or weird behavior. The app passes only when all
eight are at max.

## Dimensions

1. **Keybinding consistency** — every transport/mode key behaves identically
   across Local/YouTube/Mixed; `f`/`s`/`S`/`/` work in every applicable view;
   no dead keys; `q`/`Esc` consistent. (`1`/`2`/`3`/`4` → A/P/Q/Y; `M` cycles
   mode; `c` cycles continue; transport keys `Enter`/`Space`/`>`/`<`/`,`.`/`+-`/`m`
   /`z`/`Z`/`r`.)

2. **Search coherence** — `/` track search + `f` list filter return sensible
   results; YT and local search forms are identical; empty/typo queries
   degrade gracefully (no "doesn't make sense" results).

3. **Streaming smoothness** — gapless local↔YT↔local handoff; no >~0.5s gap
   between consecutive YT tracks; no mid-stream stutter; buffering/rate-limit
   surfaced (not a freeze).

4. **Layout balance** — two-row player bar (row 1: now-playing + quality +
   volume; row 2: gauge + `·`-separated SHUF/RPT/CONT/MODE); footer hints
   present; `Y` view consistent with `P`; narrow (60–80) fallback renders; no
   crash ≤60×20.

5. **Seamless transitions** — no flicker/blank-flash on mode/view switch,
   overlay open/close, local→YT handoff; quality readout never disagrees with
   device rate (`Opus 160k · YT` / `AAC 256k · YT Premium` for remote,
   `24-bit / 96 kHz · bit-perfect` for local).

6. **Edge cases** — dead/expired YT track skipped (no halt); sidecar-down keeps
   local alive; auth-not-set shows setup hint, not empty screen; no panics.

7. **Premium parity** — Premium cookies → AAC 256k ad-free streams selected;
   quality readout reflects the actual stream, not a hardcoded label.

8. **CoreAudio cadence** — re-clock happens exactly once entering a YT session,
   held through consecutive YT tracks, restored on return to local — never
   mid-stream.

## Method

Launch the app live (`cargo run -- play`), drive it, and exercise each
dimension. Use a stubbed/fake YouTube session (the E2E tests at
`tests/e2e_yt.rs` show how) if real Premium cookies aren't available, but the
flows must be exercised — not just code-read.
