# Provider States & Test Matrix (Jukebox)

## Required provider states

The provider (YouTube) state machine must distinguish every state below. Replace ad-hoc `yt_status`/`yt_error` strings with a derived enum whose label is the single source of truth for the footer and the YT view.

| State | Meaning | Entry condition | User-facing label |
|---|---|---|---|
| **Unconfigured** | No cookie material configured | launch with no cookies/browser | "YouTube: sign in with `:yt auth` or `:yt browser`" |
| **SignedOut** | Configured but user logged out | `:yt logout` | "YouTube: signed out" |
| **Authenticating** | Sidecar spawned, probe in flight | spawn success, before first data call | "YouTube: connecting…" |
| **AuthenticatedNotSynced** | Probe succeeded, playlists not yet loaded | first data call ok | "YouTube: loading playlists…" |
| **Synchronizing** | Playlist/page fetch in flight | `refresh_yt_lists` sent | "YouTube: syncing…" |
| **Ready** | Playlists loaded and usable | first page rendered | "YouTube: ready" |
| **ReadyStale** | Ready but last sync failed/old | sync error with prior cache | "YouTube: ready (stale — last sync failed HH:MM)" |
| **RateLimited** | Sidecar returned rate-limit | 429 / too-many-requests | "YouTube: rate-limited, retrying in Ns" |
| **AuthExpired** | Cookies present but rejected | probe data call 401/403 | "YouTube: auth expired — re-sign in with `:yt auth`" |
| **ProviderUnavailable** | Sidecar crashed / won't spawn / network down | spawn fail or pipe broken | "YouTube: unavailable — <reason>" |
| **Failed** | Unrecoverable error, no cache | unexpected error, no prior data | "YouTube: failed — <reason>" |

### Transitions that must hold
- **Never** Ready/Connected until a real data call has succeeded (cookie presence ≠ validity).
- Transient probe failure → **keep the session**, go to ProviderUnavailable/AuthExpired with a retry action. Do NOT set `yt_session = None`.
- AuthExpired with a valid cached session still showing playlists → ReadyStale, not SignedOut.
- Sidecar respawn on tick must not overwrite a known error with an optimistic "connected".

## Deterministic test matrix (no live credentials)

All YouTube tests use a **fake sidecar** (see `tests/e2e_yt.rs`). Each scenario asserts the state label + the recovery action.

| # | Scenario | Inject | Assert |
|---|---|---|---|
| 1 | Restart with valid session | saved session, probe ok | Ready, playlists shown, no re-login |
| 2 | Refresh success | stale list + refresh ok | Ready, new content |
| 3 | Token expiry (cookies rejected) | probe returns 401 | AuthExpired, re-auth action shown, no false "connected" |
| 4 | Revoked credentials | data call 403 after probe ok | AuthExpired → re-sign-in recovers |
| 5 | Empty account | probe ok, zero playlists | Ready + truthful "no playlists", not "loading…" forever |
| 6 | Pagination | list > 25/100 | full list OR "load more" affordance, no silent truncation |
| 7 | Malformed sidecar line | bad JSON | ProviderUnavailable, no panic |
| 8 | Timeout | sidecar silent > deadline | ProviderUnavailable/Failed, retry action |
| 9 | Offline launch | cached data, no network | ReadyStale with offline indicator, no noisy retry loop |
| 10 | Rate limit | 429 | RateLimited with backoff, no spam |
| 11 | Stale cache recovery | cache + reconnect | transitions to Ready on next sync |
| 12 | Recovery after respawn | sidecar crash mid-session | respawns, returns to Ready or ProviderUnavailable, never lies "connected" |

## Local / Hybrid provider checks
- Local library empty → truthful empty state, not a crash.
- Mixed mode: `match_local` thresholds (title ≥ 0.88, artist ≥ 0.80) — wrong substitution is worse than streaming.
- Source fallback: preferred source fails → predictable fallback, queue/history/now-playing stay correct.
- CoreAudio re-clock (`src/source/device_rate.rs`): one switch per YT session, held across same-rate tracks, restored on local hi-res resume.
