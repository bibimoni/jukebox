use jukebox::audio::{capture_default_format, restore_output_format};

#[test]
fn restore_with_none_is_noop() {
    // Never crashes; returns without touching the device.
    restore_output_format(None);
}

#[test]
fn capture_returns_something_or_none_without_panicking() {
    let _ = capture_default_format(); // may be None in CI; must not panic
}
