//! Tests for feedback UX overlays.
use jukebox::reco::explanations::Explanation;
use jukebox::reco::feedback::*;
use jukebox::reco::profile::UserProfile;
use jukebox::tui::view::explanation;
use jukebox::tui::view::icons::FontMode;

#[test]
fn explanation_overlay_renders() {
    let exp = Explanation {
        reason: "from your liked tracks".into(),
        detail: None,
    };
    let icons = jukebox::tui::view::icons::IconRenderer::new(FontMode::Unicode);
    let para = explanation::render(ratatui::layout::Rect::new(0, 0, 80, 24), &exp, &icons);
    let _ = para;
}

#[test]
fn feedback_like_has_correct_scope() {
    assert!(FeedbackAction::Like
        .scopes()
        .contains(&FeedbackScope::LongTermProfile));
}

#[test]
fn feedback_hidden_excluded_from_reco() {
    let mut p = UserProfile::new();
    apply_feedback(&FeedbackAction::HideTrack, "t1", None, &mut p);
    assert!(is_excluded("t1", None, &p));
}

#[test]
fn feedback_blocked_artist_excluded() {
    let mut p = UserProfile::new();
    apply_feedback(&FeedbackAction::BlockArtist, "t1", Some("Bad"), &mut p);
    assert!(is_excluded("t1", Some("Bad"), &p));
}

#[test]
fn no_feedback_is_provider_level() {
    assert!(!FeedbackAction::Like.is_provider_level());
    assert!(!FeedbackAction::HideTrack.is_provider_level());
}
