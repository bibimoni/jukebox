//! YouTube sidecar: a long-lived Python process (`scripts/yt/yt.py`) speaking
//! newline-delimited JSON over stdin/stdout.
//!
//! - [`proto`] — the request/response wire types (Task 6).
//! - [`sidecar`] — the Rust subprocess client (Task 8).
//! - [`session`] — auth/cookies/cache + the autoplay radio cursor (Task 9).

pub mod proto;
