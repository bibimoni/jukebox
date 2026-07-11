use jukebox::mode::SourceMode;

#[test]
fn cycles_in_order() {
    assert_eq!(SourceMode::Local.cycle(), SourceMode::Youtube);
    assert_eq!(SourceMode::Youtube.cycle(), SourceMode::Mixed);
    assert_eq!(SourceMode::Mixed.cycle(), SourceMode::Local);
}

#[test]
fn round_trips_strings() {
    for m in [SourceMode::Local, SourceMode::Youtube, SourceMode::Mixed] {
        assert_eq!(SourceMode::from_str(m.as_str()), m);
    }
    // unknown → default Local (forward-compat with old state.db)
    assert_eq!(SourceMode::from_str("???"), SourceMode::Local);
}

#[test]
fn default_is_local() {
    assert_eq!(SourceMode::default(), SourceMode::Local);
}
