//! Truthful YouTube provider state machine.
//!
//! Replaces the old `yt_status: Option<String>` / `yt_error: Option<String>`
//! pair (which could claim "connected" before any data fetch verified the
//! credential — the "connected but empty" bug, yt-recon §8/§10) with a single
//! enum whose variants are the *actual* lifecycle states a provider can be in.
//!
//! ## State diagram
//!
//! ```text
//!   Unconfigured ──(:yt auth)──► Authenticating ──(spawn ok)──► AuthenticatedNotSynced
//!        │                              │                            │
//!        │                         (spawn fail)                 (fetch sent)
//!        │                              │                            │
//!        │                              ▼                            ▼
//!        │                          ProviderError              Synchronizing
//!        │                                                           │
//!        │                                              ┌─────────────┼──────────────┐
//!        │                                              ▼             ▼              ▼
//!        │                                            Ready       ProviderError   AuthExpired
//!        │                                              │             │              │
//!        │                                         (fetch fail)    │              │
//!        │                                              ▼             │              │
//!        │                                          ReadyStale       │              │
//!        │                                              │             │              │
//!        │                                         (fetch ok)        │              │
//!        │                                              └─────────────┴──────────────┘
//!        │                                                              │
//!        │                                                    (:yt logout)
//!        │                                                              ▼
//!        └───────────────────────────────────────────────────────── SignedOut
//! ```
//!
//! ## Key invariant
//!
//! `is_ready()` returns `true` ONLY for `Ready` and `ReadyStale` — states that
//! mean a data fetch has *succeeded* and the provider's data is usable. No
//! variant that precedes a successful fetch (Unconfigured, SignedOut,
//! Authenticating, AuthenticatedNotSynced, Synchronizing) is ever "ready."
//! This is the fix for the false-ready bug: the footer and Y-view status line
//! derive from this enum, so "connected" can no longer appear before data is
//! verified.
//!
//! ## Error vs auth
//!
//! `ProviderError` is a non-auth failure (network blip, sidecar crash,
//! ytmusicapi parse error). `AuthExpired` is specifically a credential problem
//! (expired/revoked cookie). `RateLimited` is a throttle. Each carries an
//! actionable recovery hint via `retry_hint()`.

use serde::{Deserialize, Serialize};

/// The lifecycle state of the YouTube provider. Ordered roughly by progression
/// from "nothing set up" → "ready" → "degraded."
///
/// `Default` is `Unconfigured` — the state at first launch before any auth
/// attempt.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum YtState {
    /// No session was ever created and no auth was ever attempted (first
    /// launch, or `:yt logout` with no cookies). The user needs to run
    /// `:yt auth` or `:yt auth browser <name>`.
    #[default]
    Unconfigured,
    /// The user explicitly logged out (`:yt logout`). Cookies file deleted,
    /// browser choice cleared. Distinct from `Unconfigured` so the footer can
    /// say "signed out" rather than "not configured" (the user took an action).
    SignedOut,
    /// Auth is in progress — the user is pasting cookies or we're spawning the
    /// sidecar with a browser profile. Transient; transitions to
    /// `AuthenticatedNotSynced` on spawn success or `ProviderError` on failure.
    Authenticating,
    /// The sidecar spawned and we have credentials, but NO data fetch has
    /// verified the credential actually works yet. This is the "don't claim
    /// connected" state: the old code set `yt_status = "connected"` here
    /// (yt-recon §8 locations 1/3/4), which was false-ready. The launch probe
    /// or `refresh_yt_lists` must succeed to promote this to `Ready`.
    AuthenticatedNotSynced,
    /// A data fetch (library playlists / playlist tracks) is in flight. Set
    /// when `send_refresh` or the launch probe fires; cleared to `Ready` on
    /// success or `ProviderError`/`AuthExpired`/`RateLimited` on failure.
    Synchronizing,
    /// A data fetch succeeded and the provider's data is usable. Playlists are
    /// loaded (or genuinely empty — the fetch *returned*, which is the
    /// contract). This is the ONLY fully-healthy state.
    Ready,
    /// Was `Ready`, but a later fetch failed (network blip after a successful
    /// session). Cached data is still displayed (the old `yt_lists`); the user
    /// is told they're offline/stale. Recovers to `Ready` on the next
    /// successful fetch.
    ReadyStale,
    /// YouTube rate-limited the request (HTTP 429 / ytmusicapi throttle). The
    /// user should wait and retry. Transient; `R` retries after a backoff.
    RateLimited,
    /// The credential is expired or revoked. The SAPISID cookie exists but
    /// YouTube rejects it (data fetch returns an auth-flavored error, or
    /// silently downgrades to guest with an empty result after a prior Ready).
    /// The user must re-authenticate (`:yt auth browser <name>`).
    AuthExpired,
    /// A non-auth provider failure: network unreachable, sidecar crash,
    /// ytmusicapi parse error, IPC timeout. The user can retry (`R`) or check
    /// diagnostics. Distinguished from `AuthExpired` so the recovery hint
    /// differs ("check connection" vs "re-authenticate").
    ProviderError,
    /// A terminal/generic failure that can't be retried — the sidecar can't
    /// be spawned at all (python3 missing, script not found, deps not
    /// installed). The user must run `:yt setup` or fix their environment.
    /// Distinct from `ProviderError` (transient/retryable) — `Failed` needs
    /// an environment fix, not a retry.
    Failed,
}

impl YtState {
    /// True only when the provider's data is *actually usable* — a data fetch
    /// has succeeded. `Ready` (fully healthy) and `ReadyStale` (cached data +
    /// offline indicator) both qualify because the user can browse cached
    /// playlists. No pre-fetch state is ever ready.
    ///
    /// This is the single source of truth the footer and Y-view status line
    /// should check instead of the old `yt_session.is_some()`.
    pub fn is_ready(&self) -> bool {
        matches!(self, YtState::Ready | YtState::ReadyStale)
    }

    /// True for the error/degraded states the user should be alerted to:
    /// `ProviderError`, `AuthExpired`, `RateLimited`, `Failed`. `ReadyStale`
    /// is degraded-but-usable (cached data still shown), so it is NOT an
    /// error — the user can still browse. Used by the footer to pick the alert
    /// color (yellow vs accent) so a problem is visually distinct from healthy
    /// / in-progress states, and by the Y-view body to decide whether to show
    /// the error detail. (`icon()` carries the NO_COLOR-safe glyph so the
    /// state is distinguishable without color too.)
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            YtState::ProviderError | YtState::AuthExpired | YtState::RateLimited | YtState::Failed
        )
    }

    /// True when credentials are present (the sidecar spawned with cookies or a
    /// browser profile), regardless of whether data has been verified. Used to
    /// distinguish "you're authed but we haven't synced" from "you need to
    /// auth." Distinct from `is_ready()` — being authed is necessary but not
    /// sufficient for being ready.
    pub fn is_authed(&self) -> bool {
        matches!(
            self,
            YtState::AuthenticatedNotSynced
                | YtState::Synchronizing
                | YtState::Ready
                | YtState::ReadyStale
                | YtState::RateLimited
                | YtState::AuthExpired
        )
    }

    /// True when pressing `R` (retry) is a meaningful action — the state is
    /// recoverable by re-running the probe. `Ready`/`ReadyStale` don't need a
    /// retry; `Unconfigured`/`SignedOut` need auth, not retry.
    pub fn can_retry(&self) -> bool {
        matches!(
            self,
            YtState::AuthenticatedNotSynced
                | YtState::Synchronizing
                | YtState::ReadyStale
                | YtState::RateLimited
                | YtState::AuthExpired
                | YtState::ProviderError
        )
    }

    /// True when the state is transient (expected to change on the next event).
    /// Used by the footer to decide whether to show a spinner vs a static label.
    pub fn is_transient(&self) -> bool {
        matches!(self, YtState::Authenticating | YtState::Synchronizing)
    }

    /// A short, user-facing label for the footer / Y-view status line. Lowercase,
    /// no trailing punctuation, fits the existing footer style ("YT: …").
    ///
    /// These labels are the ONLY user-facing provider-state strings; the old
    /// `yt_status` / `yt_error` free-text is replaced by `human_label()` +
    /// `detail` (the error message, shown alongside when present).
    pub fn human_label(&self) -> &'static str {
        match self {
            YtState::Unconfigured => "not configured — run :yt auth browser <chrome>",
            YtState::SignedOut => "signed out — run :yt auth to reconnect",
            YtState::Authenticating => "authenticating…",
            YtState::AuthenticatedNotSynced => "authenticated — syncing…",
            YtState::Synchronizing => "synchronizing…",
            YtState::Ready => "ready",
            YtState::ReadyStale => "offline — showing cached (press R to retry)",
            YtState::RateLimited => "rate limited — wait, then press R",
            YtState::AuthExpired => "authorization expired — run :yt auth browser <name>",
            YtState::ProviderError => "provider error — press R to retry",
            YtState::Failed => "failed — run :yt setup or check your installation",
        }
    }

    /// A one-line recovery hint for the footer when the state is not ready.
    /// `None` for healthy/terminal-no-action states. Shown alongside
    /// `human_label()` when the user needs to know what to do next.
    pub fn retry_hint(&self) -> Option<&'static str> {
        match self {
            YtState::Unconfigured => Some("run :yt auth browser <chrome|firefox|safari>"),
            YtState::SignedOut => Some("run :yt auth to reconnect"),
            YtState::AuthExpired => Some("run :yt auth browser <name> to re-authenticate"),
            YtState::RateLimited => Some("wait a moment, then press R"),
            YtState::ProviderError => Some("press R to retry, or check your connection"),
            YtState::Failed => Some("run :yt setup, or check python3 / the script path"),
            YtState::ReadyStale => Some("press R to retry the connection"),
            // Transient / healthy: no hint needed.
            YtState::Authenticating
            | YtState::AuthenticatedNotSynced
            | YtState::Synchronizing
            | YtState::Ready => None,
        }
    }

    /// A short ASCII-safe glyph for the footer (no Unicode dependence — usable
    /// under NO_COLOR and in minimal fonts). Paired with `human_label()` so
    /// the state is distinguishable without color (accessibility: not color-
    /// only). `None` for healthy states that don't need an alert glyph.
    pub fn icon(&self) -> Option<&'static str> {
        match self {
            YtState::Unconfigured | YtState::SignedOut => Some("[!]"),
            YtState::Authenticating | YtState::AuthenticatedNotSynced | YtState::Synchronizing => {
                Some("[~]")
            }
            YtState::Ready => None,
            YtState::ReadyStale => Some("[stale]"),
            YtState::RateLimited => Some("[throttle]"),
            YtState::AuthExpired => Some("[reauth]"),
            YtState::ProviderError => Some("[err]"),
            YtState::Failed => Some("[fail]"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_unconfigured() {
        assert_eq!(YtState::default(), YtState::Unconfigured);
    }

    #[test]
    fn only_ready_and_ready_stale_are_ready() {
        assert!(YtState::Ready.is_ready());
        assert!(YtState::ReadyStale.is_ready());
        // Everything else is not ready — the core invariant.
        assert!(!YtState::Unconfigured.is_ready());
        assert!(!YtState::SignedOut.is_ready());
        assert!(!YtState::Authenticating.is_ready());
        assert!(!YtState::AuthenticatedNotSynced.is_ready());
        assert!(!YtState::Synchronizing.is_ready());
        assert!(!YtState::RateLimited.is_ready());
        assert!(!YtState::AuthExpired.is_ready());
        assert!(!YtState::ProviderError.is_ready());
        assert!(!YtState::Failed.is_ready());
    }

    #[test]
    fn is_error_true_only_for_error_states() {
        // The four states the user must be alerted to (footer turns yellow).
        assert!(YtState::ProviderError.is_error());
        assert!(YtState::AuthExpired.is_error());
        assert!(YtState::RateLimited.is_error());
        assert!(YtState::Failed.is_error());
    }

    #[test]
    fn is_error_false_for_healthy_in_progress_and_degraded_usable() {
        // Ready is healthy; the transient states are in-progress (not errors);
        // Unconfigured/SignedOut need auth (not errors); ReadyStale is
        // degraded-but-usable (cached data shown) — NOT an error.
        assert!(!YtState::Ready.is_error());
        assert!(!YtState::Unconfigured.is_error());
        assert!(!YtState::SignedOut.is_error());
        assert!(!YtState::Authenticating.is_error());
        assert!(!YtState::AuthenticatedNotSynced.is_error());
        assert!(!YtState::Synchronizing.is_error());
        assert!(!YtState::ReadyStale.is_error());
    }

    #[test]
    fn failed_is_not_authed_and_not_retryable() {
        // Failed = hard startup failure (deps/python/script missing). It is
        // NOT authed (nothing works) and NOT retryable via R (retry would just
        // re-fail). The hint directs the user to :yt setup instead.
        assert!(!YtState::Failed.is_authed());
        assert!(!YtState::Failed.can_retry());
        assert_eq!(
            YtState::Failed.retry_hint(),
            Some("run :yt setup, or check python3 / the script path")
        );
    }

    #[test]
    fn authenticated_not_synced_is_authed_but_not_ready() {
        // The false-ready fix: AuthenticatedNotSynced means we have credentials
        // but haven't verified data. It must be authed (so we know to sync) but
        // NOT ready (so the footer doesn't claim "connected").
        assert!(YtState::AuthenticatedNotSynced.is_authed());
        assert!(!YtState::AuthenticatedNotSynced.is_ready());
    }

    #[test]
    fn unconfigured_and_signed_out_are_not_authed() {
        assert!(!YtState::Unconfigured.is_authed());
        assert!(!YtState::SignedOut.is_authed());
    }

    #[test]
    fn ready_is_authed() {
        assert!(YtState::Ready.is_authed());
    }

    #[test]
    fn auth_expired_is_authed_but_not_ready() {
        // Expired credentials are still "authed" in the sense that the user did
        // authenticate — they just need to re-do it. This lets the footer say
        // "authorization expired" (distinct from "not configured").
        assert!(YtState::AuthExpired.is_authed());
        assert!(!YtState::AuthExpired.is_ready());
    }

    #[test]
    fn ready_does_not_need_retry() {
        assert!(!YtState::Ready.can_retry());
    }

    #[test]
    fn error_states_can_retry() {
        assert!(YtState::ProviderError.can_retry());
        assert!(YtState::AuthExpired.can_retry());
        assert!(YtState::RateLimited.can_retry());
        assert!(YtState::ReadyStale.can_retry());
        assert!(YtState::AuthenticatedNotSynced.can_retry());
    }

    #[test]
    fn unconfigured_cannot_retry() {
        // Unconfigured needs auth, not retry. The hint tells the user to run
        // :yt auth, not press R.
        assert!(!YtState::Unconfigured.can_retry());
        assert_eq!(
            YtState::Unconfigured.retry_hint(),
            Some("run :yt auth browser <chrome|firefox|safari>")
        );
    }

    #[test]
    fn transient_states_are_transient() {
        assert!(YtState::Authenticating.is_transient());
        assert!(YtState::Synchronizing.is_transient());
        assert!(!YtState::Ready.is_transient());
        assert!(!YtState::ProviderError.is_transient());
    }

    #[test]
    fn human_labels_are_lowercase_no_trailing_punct() {
        for state in [
            YtState::Unconfigured,
            YtState::SignedOut,
            YtState::Authenticating,
            YtState::AuthenticatedNotSynced,
            YtState::Synchronizing,
            YtState::Ready,
            YtState::ReadyStale,
            YtState::RateLimited,
            YtState::AuthExpired,
            YtState::ProviderError,
            YtState::Failed,
        ] {
            let label = state.human_label();
            assert!(
                !label.ends_with('.') && !label.ends_with('!'),
                "label {:?} should not end with punctuation",
                label
            );
        }
    }

    #[test]
    fn ready_has_no_icon() {
        // Ready is the only state that doesn't need an alert glyph — it's the
        // "all good" state.
        assert_eq!(YtState::Ready.icon(), None);
    }

    #[test]
    fn error_states_have_icons() {
        // Accessibility: every non-ready state has an ASCII-safe icon so the
        // state is distinguishable without color (NO_COLOR / color-blindness).
        assert!(YtState::Unconfigured.icon().is_some());
        assert!(YtState::SignedOut.icon().is_some());
        assert!(YtState::AuthExpired.icon().is_some());
        assert!(YtState::ProviderError.icon().is_some());
        assert!(YtState::Failed.icon().is_some());
        assert!(YtState::ReadyStale.icon().is_some());
        assert!(YtState::RateLimited.icon().is_some());
    }

    #[test]
    fn failed_is_not_ready_not_authed_not_retryable() {
        // Failed is terminal: the sidecar can't spawn (python3 missing, deps
        // not installed). Needs :yt setup, not R. Not authed (no credentials
        // were ever validated). Not ready. Not transient.
        assert!(!YtState::Failed.is_ready());
        assert!(!YtState::Failed.is_authed());
        assert!(!YtState::Failed.can_retry());
        assert!(!YtState::Failed.is_transient());
    }

    #[test]
    fn icons_are_ascii_safe() {
        // No Unicode dependence — every icon is plain ASCII so it renders in
        // any font / NO_COLOR mode.
        for state in [
            YtState::Unconfigured,
            YtState::SignedOut,
            YtState::Authenticating,
            YtState::AuthenticatedNotSynced,
            YtState::Synchronizing,
            YtState::ReadyStale,
            YtState::RateLimited,
            YtState::AuthExpired,
            YtState::ProviderError,
            YtState::Failed,
        ] {
            if let Some(icon) = state.icon() {
                for c in icon.chars() {
                    assert!(
                        c.is_ascii(),
                        "icon {:?} for {:?} must be ASCII-safe, found non-ASCII char",
                        icon,
                        state
                    );
                }
            }
        }
    }

    #[test]
    fn auth_expired_hint_tells_user_to_reauth() {
        assert_eq!(
            YtState::AuthExpired.retry_hint(),
            Some("run :yt auth browser <name> to re-authenticate")
        );
    }

    #[test]
    fn provider_error_hint_tells_user_to_retry() {
        assert_eq!(
            YtState::ProviderError.retry_hint(),
            Some("press R to retry, or check your connection")
        );
    }
}
