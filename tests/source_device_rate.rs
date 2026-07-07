use jukebox::source::device_rate::{desired_switch, DeviceRateState, LoadKind};

#[test]
fn local_track_switches_to_its_rate() {
    let mut s = DeviceRateState::default();
    let r = desired_switch(&mut s, LoadKind::Local { sample_rate_hz: 192000, bit_depth: 24 }, true);
    assert_eq!(r, Some((192000, 24)));
    assert!(!s.in_yt_rate);
}

#[test]
fn consecutive_local_same_rate_no_reswitch() {
    let mut s = DeviceRateState::default();
    desired_switch(&mut s, LoadKind::Local { sample_rate_hz: 96000, bit_depth: 24 }, true);
    let r = desired_switch(&mut s, LoadKind::Local { sample_rate_hz: 96000, bit_depth: 24 }, true);
    assert_eq!(r, None);
}

#[test]
fn first_remote_switches_once() {
    let mut s = DeviceRateState::default();
    let r = desired_switch(&mut s, LoadKind::Remote { sample_rate: 48000 }, true);
    assert_eq!(r, Some((48000, 16)));
    assert!(s.in_yt_rate);
}

#[test]
fn consecutive_remote_same_rate_held() {
    let mut s = DeviceRateState::default();
    desired_switch(&mut s, LoadKind::Remote { sample_rate: 48000 }, true);
    let r = desired_switch(&mut s, LoadKind::Remote { sample_rate: 48000 }, true);
    assert_eq!(r, None);
    assert!(s.in_yt_rate);
}

#[test]
fn remote_rate_change_reswitches_once() {
    let mut s = DeviceRateState::default();
    desired_switch(&mut s, LoadKind::Remote { sample_rate: 48000 }, true);
    let r = desired_switch(&mut s, LoadKind::Remote { sample_rate: 44100 }, true);
    assert_eq!(r, Some((44100, 16)));
}

#[test]
fn local_after_remote_clears_yt_flag() {
    let mut s = DeviceRateState::default();
    desired_switch(&mut s, LoadKind::Remote { sample_rate: 48000 }, true);
    assert!(s.in_yt_rate);
    let r = desired_switch(&mut s, LoadKind::Local { sample_rate_hz: 192000, bit_depth: 24 }, true);
    assert_eq!(r, Some((192000, 24)));
    assert!(!s.in_yt_rate);
}

#[test]
fn switch_sample_rate_off_never_switches() {
    let mut s = DeviceRateState::default();
    let r = desired_switch(&mut s, LoadKind::Local { sample_rate_hz: 192000, bit_depth: 24 }, false);
    assert_eq!(r, None);
}
