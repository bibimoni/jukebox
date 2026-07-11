---
name: jukebox-product-audit
description: Audit the jukebox terminal music app as a complete product. Map critical user journeys, compare implemented behavior with core capabilities, find inconsistent states, dead ends, and UX gaps. Use before making product decisions or judging release readiness.
---

# Jukebox Product Audit

Audit jukebox as a complete product, not just code. Map journeys, find dead ends, compare against core music-app expectations.

## When to use

- Before making product decisions (feature priority, UX changes).
- Before a release candidate (does the product hang together?).
- When user reports are vague ("something feels off").

## Procedure

1. **Read the durable workspace:** `docs/development/jukebox-revamp/JOURNEYS.md` (journeys + capability matrix), `ACCEPTANCE.md` (criteria).
2. **Map the journey:** Walk each critical journey (A-H in JOURNEYS.md). For each step, note the expected state, the actual state, and any defect.
3. **Check capability matrix:** For each capability in JOURNEYS.md §Capability Matrix, verify: works / defective / missing. Classify: Core / Important / Defective / Out-of-scope / External-limitation.
4. **Find dead ends:** Look for: actions with no feedback (silent no-ops), states with no exit (infinite "loading…"), controls that don't do what they advertise (README vs Help overlay), empty states with no guidance ("why is this empty?").
5. **Check state consistency:** Footer status vs. col2 body vs. now-playing — do they agree? (The "connected but empty" pattern: footer says connected, view says empty.)
6. **Distinguish core from bloat:** Favorites, recommendations, casting are NOT core. Browse, search, play, queue, playlist, lyrics, transport ARE core. Don't flag missing bloat as a defect.
7. **Produce findings:** Each finding has: severity (P0/P1/P2/P3), file:line evidence, reproduction steps, affected journey, acceptance criterion.

## Key patterns to look for

- **False-ready:** status set on spawn/process-start, not on data-availability. (Search for `yt_status =` assignments.)
- **Silent no-op:** key pressed, nothing happens, no feedback. (Search for `_ => {}` in input dispatch.)
- **Infinite loading:** loading flag set, never cleared on failure. (Search for `_loading = true` without a clear-on-error path.)
- **Doc mismatch:** README vs. Help overlay vs. actual behavior. (Diff keybindings.)
- **Color-only signal:** selection or state visible only via color, breaks NO_COLOR. (Check `theme.rs` + `NO_COLOR`.)

## References

- `references/journey-checklist.md` — critical journeys A–H + capability matrix + per-journey audit template (required for every audit pass).
- `docs/development/jukebox-revamp/AUDIT.md`, `tui-recon.md` — existing evidence base (example of `file:line`-backed findings).
- `docs/development/jukebox-revamp/ACCEPTANCE.md` — acceptance criteria.
