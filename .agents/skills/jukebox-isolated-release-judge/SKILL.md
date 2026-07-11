---
name: jukebox-isolated-release-judge
description: Use when independently judging a Jukebox release candidate in a fresh thread — receive only the rubric, requirements, repo state, and candidate commit; run your own tests; return reproducible blockers, scores, and a PASS / CONDITIONAL PASS / FAIL verdict without ever modifying the implementation.
---

# Jukebox Isolated Release Judge

## Overview
The implementer is not the final authority on their own work. A judge evaluates a candidate as if they did not participate in it, generates their own evidence, and returns a verdict. Two judges run per candidate (black-box product judge; adversarial engineering judge). Both must score ≥ 90, average ≥ 93, neither FAIL, no rubric category < 80%.

## When to Use
- After a release candidate is ready (all local gates green).
- Before merging `revamp/product-polish` to `main`.

## Isolation protocol (mandatory)
1. **Fresh thread.** Do NOT resume an implementer session. Do NOT carry prior context.
2. **Prefer a clean read-only worktree** at the candidate commit. Never modify the implementation.
3. **Receive ONLY**: the repository, candidate commit, product requirements, acceptance journeys, evaluation rubric, and credentials-free fixtures.
4. **Do NOT receive**: implementer work log, planned fixes, implementer's explanation, claimed root causes, claimed test results, or previous judge scores before your initial assessment.
5. **Generate your own evidence.** Run the build, tests, lint, and format gates yourself. Render TUI states yourself. Reproduce findings yourself.
6. **Record score history** in `docs/development/jukebox-revamp/JUDGE.md`.

## Verdict
- **PASS** — all mandatory gates met; both judges ≥ 90; avg ≥ 93; no rubric category < 80%; no P0/P1; no security leak.
- **CONDITIONAL PASS** — all core journeys pass but ≥ 1 gate pending (e.g. an external provider limitation). Clearly distinguish "not externally exercised" from "implemented and verified". This never permits skipping mock-based lifecycle tests.
- **FAIL** — any P0/P1, any security/credential leak, any false connected/ready state, any failed core journey, or any rubric category < 80%.

## What a judge must do
1. Discover how to build and run the app from the repo (don't assume).
2. Run: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features`, `bats scripts/test/*.bats`, `cargo build --release`.
3. Execute the critical journeys (Journey A–H) with deterministic fixtures + fake sidecar.
4. Inspect rendered TUI states at multiple sizes; inspect snapshots.
5. Score with `references/rubric.md` and issue a verdict.
6. Return reproducible blockers with evidence, not opinions.

## The main agent may reject a judge finding ONLY with concrete contradictory evidence recorded in `JUDGE.md`. Otherwise the finding stands and must be fixed.

## Required references
- `references/rubric.md` — scoring rubric, mandatory gates, and the two verbatim judge prompts (required for every judge run).
- `docs/development/jukebox-revamp/JUDGE.md` — score history (append, don't overwrite).
