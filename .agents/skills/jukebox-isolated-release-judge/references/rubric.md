# Release Rubric & Judge Prompts (Jukebox)

## Scoring rubric (out of 100)

- Functional correctness and complete core journeys: **25**
- Provider, authentication, persistence, and recovery reliability: **20**
- UX clarity, discoverability, and interaction consistency: **20**
- Playback correctness and responsiveness: **10**
- Automated test depth and determinism: **10**
- Security and privacy: **5**
- Terminal compatibility and accessibility: **5**
- Maintainability and documentation: **5**

## Mandatory release gates

- No known P0 or P1 defect.
- No unresolved security or credential-leak issue.
- All required local, YouTube, and hybrid journeys pass or have a clearly identified external provider limitation with all local behavior complete.
- YouTube session restoration and playlist display pass in deterministic tests.
- No false connected/ready state.
- Command history works and is tested.
- Lyrics have a functional, truthful, non-blocking implementation.
- Input remains responsive during slow provider and lyrics operations.
- Full test, lint, format, and build gates pass.
- Snapshots or rendered-state assertions have been inspected.
- Both fresh judge scores are at least 90.
- The average judge score is at least 93.
- Neither judge returns FAIL.
- No rubric category receives less than 80% of its available points.
- Final verification is performed from a clean checkout or worktree at the candidate commit.

Continue iterating until every gate is satisfied.

If progress is blocked exclusively by an unavailable live credential or external service, complete all deterministic work, provide the manual verification procedure, and clearly distinguish "not externally exercised" from "implemented and verified." This does not permit skipping mock-based lifecycle tests.

## Judge 1 — black-box product judge (verbatim)

> You are an independent product release judge for a terminal music application. Do not modify the repository. Evaluate the candidate as though you did not participate in its implementation. Begin by discovering how to build and run it. Execute the critical user journeys using deterministic fixtures and mocks. Inspect rendered TUI states at multiple terminal sizes. Look for confusing states, dead ends, false success messages, missing feedback, inconsistent controls, accessibility problems, and behavior that would surprise a normal music-app user. Report only findings you can support with reproduction steps or direct artifact evidence. Score the candidate using the supplied rubric and issue PASS, CONDITIONAL PASS, or FAIL.

## Judge 2 — adversarial engineering judge (verbatim)

> You are an independent adversarial engineering release judge. Do not modify the repository. Assume the implementation contains subtle state, concurrency, persistence, authentication, provider, playback, and cleanup defects. Independently run tests, inspect risky boundaries, add temporary external test harnesses only outside the candidate tree when needed, and attempt to falsify its correctness claims. Pay special attention to restart persistence, expired credentials, pagination, stale asynchronous results, offline recovery, queue/now-playing divergence, terminal escape injection, secret leakage, migration safety, child-process cleanup, and large-input responsiveness. Return reproducible blockers, missing tests, scores, and a release verdict.

## Judge protocol rules

- Each judge must generate its own evidence.
- Spawn new fresh judge threads rather than asking the same judges to rationalize earlier conclusions.
- The main agent may reject a judge finding only with concrete contradictory evidence recorded in `docs/development/jukebox-revamp/JUDGE.md`.
- Record score history in `JUDGE.md` after each candidate.
