---
name: jukebox-provider-reliability
description: Evaluate jukebox's local, YouTube, and hybrid provider behavior. Trace authentication, token persistence, refresh, revocation, playlist sync, pagination, caching, retries, cancellation, rate limits, source fallback, offline behavior, and secret handling. Use when fixing YouTube reliability or designing provider tests.
---

# Jukebox Provider Reliability

Evaluate provider behavior as a complete lifecycle. Require explicit state machines and actionable errors.

## When to use

- When fixing YouTube reliability (repeated logins, "connected but empty", pagination).
- When designing deterministic provider tests (without a live account).
- When auditing secret handling (cookies, tokens, file permissions).

## Procedure

1. **Read the recon:** `docs/development/jukebox-revamp/yt-recon.md` (auth, pagination, caching, expiry, logout, false-ready locations).
2. **Trace the auth lifecycle:** spawn → cookie read → first fetch → data available → expiry → refresh → revocation → logout. At each step, what state does the UI show? Is it truthful?
3. **Check pagination:** `get_library_playlists` (25 default), `get_playlist` (100 default). Are continuation loops present? Is truncation communicated?
4. **Check caching:** `track_cache` (unbounded?), `url_cache` (cap 2), `yt_lists` (not persisted). What survives restart? What's lost?
5. **Check cancellation:** Are there generation ids? Can stale results overwrite fresh state? (Search for `sync_epoch` or generation patterns.)
6. **Check offline:** What happens when the network drops mid-session? Does the probe nuke the session? Are cached playlists shown with a stale indicator?
7. **Check secrets:** Cookie file perms (0600?), env var visibility, temp file cleanup, log redaction.
8. **Build deterministic tests:** Use fake Python sidecars (see `tests/e2e_yt.rs` pattern). No real credentials. No real network.

## Required state machine

A provider must not collapse "token exists," "API succeeded," "sync completed," and "UI ready" into one `connected` bool. Use:

```
Unconfigured → SignedOut → Authenticating → AuthenticatedNotSynced → Synchronizing → Ready
                                                                                    ↓
                                                              ReadyStale(offline) ←──┘
                                                              RateLimited
                                                              AuthExpired
                                                              ProviderError
                                                              Failed
```

## Key defect patterns

- **Probe discards session** (`main.rs:151`): single failure → `yt_session = None` for whole run.
- **auth_status lies** (`yt.py:329`): `ok = _has_auth()` checks presence, not validity.
- **No pagination** (`yt.py:345,364`): silent 25/100 truncation.
- **No generation ids** (`session.rs:714`): stale refresh overwrites fresh data.
- **False-ready** (`main.rs:117/129`, `app.rs:988/1014/1093`): "connected" set on spawn.

## References

- `references/provider-states.md` — required provider states + deterministic test matrix (required for every provider audit).
- `references/provider-test-patterns.md` — fake sidecar test patterns + key scenarios.
- `docs/development/jukebox-revamp/yt-recon.md` — full YouTube recon (example of evidence-backed findings).
- `docs/development/jukebox-revamp/DECISIONS.md` — D2-D5 (state machine, probe, pagination, generation ids).
