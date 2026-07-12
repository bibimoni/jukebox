//! Degraded operation tests — offline, quota, errors.
use jukebox::catalog::Track;
use jukebox::reco::mixes::{generate_mix, MixType};
use jukebox::reco::profile::UserProfile;
use jukebox::reco::radio::{RadioSeed, RadioSession};

use jukebox::tui::view::icons::{FontMode, IconRenderer};
use jukebox::yt::state::YtState;
use std::path::PathBuf;

fn mk(id: &str, artist: &str) -> Track {
    Track {
        id: id.into(),
        artists: vec![artist.into()],
        primary_artist: artist.into(),
        title: id.into(),
        album: Some("Album".into()),
        track_number: Some(1),
        disc_number: Some(1),
        bit_depth: 16,
        sample_rate_hz: 44100,
        isrc: None,
        source_path: PathBuf::from("/t.flac"),
        symlinked_into_artists: vec![],
    }
}

#[test]
fn offline_state_is_not_ready() {
    assert!(!YtState::Unconfigured.is_ready());
    assert!(!YtState::ProviderError.is_ready());
}

#[test]
fn ready_stale_is_usable() {
    assert!(YtState::ReadyStale.is_ready());
}

#[test]
fn offline_mix_uses_local_catalog() {
    let events = vec![jukebox::reco::events::ListenEvent::Completed {
        track_id: "t1".into(),
        timestamp: 100,
    }];
    let p = UserProfile::build_from_events(&events);
    let catalog = vec![mk("t1", "A"), mk("t2", "B")];
    // Mix generation works offline (uses local catalog + profile).
    let mix = generate_mix(MixType::DailyMix, &p, &catalog);
    assert!(!mix.tracks.is_empty());
}

#[test]
fn offline_radio_works() {
    let events = vec![jukebox::reco::events::ListenEvent::Completed {
        track_id: "t1".into(),
        timestamp: 100,
    }];
    let p = UserProfile::build_from_events(&events);
    let catalog = vec![mk("t1", "A"), mk("t2", "B")];
    let mut r = RadioSession::new(RadioSeed::Track("t1".into()));
    r.initialize(&p, &catalog);
    assert!(!r.candidate_pool.is_empty());
}

#[test]
fn quota_exhausted_state_is_error() {
    assert!(YtState::RateLimited.is_error());
}

#[test]
fn offline_home_renders() {
    let icons = IconRenderer::new(FontMode::Unicode);
    let para = jukebox::tui::view::home::render_offline(&icons);
    let _ = para;
}

#[test]
fn empty_home_renders() {
    let icons = IconRenderer::new(FontMode::Unicode);
    let para = jukebox::tui::view::home::render_empty(&icons);
    let _ = para;
}

#[test]
fn all_yt_states_have_labels() {
    for s in [
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
        assert!(!s.human_label().is_empty());
    }
}

#[test]
fn all_yt_states_have_icons_or_none() {
    for s in [
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
        let _ = s.icon();
    }
}
