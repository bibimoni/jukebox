# Unit Test Record: state.rs (is_error + Failed)

## Target File
`/Users/distiled/Dev/jukebox/src/yt/state.rs`

## Test File (INLINE — not deleted, kept in source)
`src/yt/state.rs` `#[cfg(test)] mod tests` section

## Test Code (Preserved inline in source)

The tests are inline in the source file (not a separate isolated test file) because:
1. `state.rs` is a pure data-enum module with NO external dependencies (no I/O, no side effects, no imports beyond `serde`)
2. The existing pattern in this file uses inline `#[cfg(test)] mod tests` (established by the prior session)
3. Adding a separate `.isolated.test.rs` file would import the same module — no isolation benefit since the module has zero external deps

### Tests added by this session (ses_7):

```rust
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
```

### Method added:

```rust
pub fn is_error(&self) -> bool {
    matches!(
        self,
        YtState::ProviderError | YtState::AuthExpired | YtState::RateLimited | YtState::Failed
    )
}
```

### Test added to existing `only_ready_and_ready_stale_are_ready`:

```rust
assert!(!YtState::Failed.is_ready());  // Added to cover the Failed variant
```

## Test Result
- Status: **BLOCKED** — cannot run `cargo test` due to concurrent M3 lyrics build errors (main.rs syntax error, overlay.rs/lyrics/mod.rs type mismatches). These errors are in files OUTSIDE this task's scope (T2.1 state machine).
- state.rs formatting: PASS (`rustfmt --check` exit 0)
- state.rs has zero compile errors (confirmed by `cargo build` error list — all 4 errors are in lyrics/overlay, none in state.rs)
- Session: ses_7
- Timestamp: 2026-07-12T02:31:00

## Notes
- The `Failed` variant and its match arms (human_label, retry_hint, icon) were already present from a prior session. This session added the `is_error()` method + 3 new tests + 1 assertion to an existing test.
- The rest of T2.1 (app.rs, footer.rs, columns.rs, main.rs state transitions) was completed by concurrent session ses_3.
