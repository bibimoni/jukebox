# Project Context — Jukebox

## Environment
- Language: Rust (edition 2021)
- Runtime: native binary; macOS coreaudio-sys on mac, libc on Unix
- Build: `cargo build --release` (slow first build: lindera embeds ~40MB IPADIC)
- Test: `cargo test --all-features`, `bats scripts/test/*.bats`
- Lint: `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --check`
- Package Manager: cargo (Cargo.lock committed)
- Optional runtime deps: `metaflac`, `ffprobe`, `jq`, `yq`, `mpv`/`afplay`, `python3` + `ytmusicapi` + `yt-dlp`

## Project Type
- [x] Application (Terminal UI music player)
- [ ] Library
- Repo: github.com/bibimoni/jukebox, version 0.3.0, MIT

## Infrastructure
- Container: None
- CI/CD: `.github/workflows/release.yml` only (no CI test workflow)
- Cloud: None

## Structure
- Source: `src/` — `main.rs`, `lib.rs`, `audio.rs`, `catalog.rs`, `cli.rs`, `config.rs`, `mode.rs`, `player.rs`, `prompt.rs`, `search.rs`, `state.rs`, `translit.rs`
  - `src/source/` — `mod.rs`, `match_local.rs`, `device_rate.rs` (source selection / hybrid)
  - `src/tui/` — `app.rs` (83 KB — huge), `context.rs`, `event.rs`, `input.rs` (32 KB), `mod.rs`, `queue.rs`, `view/`
  - `src/yt/` — `mod.rs`, `proto.rs`, `session.rs` (45 KB — huge), `sidecar.rs`
- Tests: `tests/` — extensive (app, columns, context, e2e_yt 41 KB, input, layout, player, player_bar, transport, tui, yt_sidecar, source_match, source_device_rate, state_ext, snapshots/)
- Scripts: `scripts/standardize.sh`, `scripts/yt/yt.py` (24 KB Python sidecar), `scripts/yt/requirements.txt`, `scripts/test/*.bats`, `scripts/lib/`
- Specs: `specs/2026-07-04-*.md` (design + plan), `specs/2026-07-06-tui-revamp-*.md` (prior TUI revamp design + plan)
- Docs: `docs/superpowers/` only
- Entry: `src/main.rs`

## Conventions (OBSERVE from existing code)
- Naming: snake_case (Rust standard)
- Imports: module-level `use` with absolute paths from crate root
- Error handling: `anyhow::Result` (panics via `.unwrap()`/`.expect()` in places — audit target)
- Testing: `cargo test` + `insta` snapshot tests + `bats` for shell helpers
- Architecture: TUI app owns state; YouTube talks to a Python sidecar over JSON protocol

## Git State
- Branch (start): `main` @ `0b0977a` (v0.3.0)
- Work branch: `revamp/product-polish` (created 2026-07-12)
- Only untracked at start: `.opencode/`, `.yolo.json` (no pre-existing user edits)
- Tags: v0.1.0, v0.1.1, v0.1.2, v0.2.0, v0.3.0

## Recent History (signals)
- v0.3.0 merged `feat/youtube-integration` (PR #1)
- Multiple `fix(yt)` commits after judge reviews: freezes, on_tick wiring, async refresh, gapless pre-resolve, deps-bearing python, auth browser cookies
- Last 5 commits suggest prior judge-driven polish of YouTube already happened; user reports issues persist (repeated logins, empty playlists, missing lyrics, command history, noisy feedback)

## Mission (from .opencode/prompt)
Full product revamp: audit, prioritize, fix, independently judge, iterate to release gates. Build 4 repo skills. 5 recon specialists → durable workspace `docs/development/jukebox-revamp/`. Two independent judges required; avg ≥ 93, neither FAIL, no rubric < 80%.

## Verification Frontier
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-features`
- `bats scripts/test/*.bats`
- release build: `cargo build --release`
- No CI test workflow exists — release.yml only

## Notes
- `.yolo.json` { enabled: true, aggressive: false } — auto-approve mode
- `.claude/settings.local.json` — bash permission allowlist for tmux/mpv/gh
- No `AGENTS.md` / `CLAUDE.md` / `GEMINI.md` in repo
- `docs/development/jukebox-revamp/` is the durable workspace (created)
