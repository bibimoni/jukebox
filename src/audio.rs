//! Switch the macOS default output device's physical format (sample rate +
//! bit depth) to match the track being played.
//!
//! On macOS this calls CoreAudio directly (`AudioObjectSetPropertyData` with
//! `kAudioStreamPropertyPhysicalFormat`) — the same API LosslessSwitcher uses
//! via its `SimplyCoreAudio` dependency. Because the catalog already knows
//! each track's `sample_rate_hz` + `bit_depth`, jukebox can switch to the
//! exact format before loading the track into the player, with no external
//! app or log-parsing required.
//!
//! On non-macOS targets this is a no-op so the cross-platform build still
//! compiles.

#[cfg(target_os = "macos")]
mod inner {
    use anyhow::{anyhow, Context, Result};
    use coreaudio_sys::*;
    use std::ffi::c_void;
    use std::mem;
    use std::ptr;

    /// Set the default output device's physical format to the closest supported
    /// match for `sample_rate_hz` + `bit_depth`. Returns Ok(()) on success or if
    /// the device already matches; Err if CoreAudio refuses the switch.
    pub fn set_output_format(sample_rate_hz: u32, bit_depth: u32) -> Result<()> {
        let device = default_output_device()
            .context("resolving default output device")?;
        let stream = first_output_stream(device)
            .context("finding an output stream on the default device")?;
        let available = available_physical_formats(stream)
            .context("listing available physical formats")?;
        let target = match_format(&available, sample_rate_hz, bit_depth)
            .ok_or_else(|| anyhow!(
                "device supports no format near {}Hz/{}bit",
                sample_rate_hz, bit_depth
            ))?;
        set_physical_format(stream, target)
            .context("setting the stream's physical format")?;
        Ok(())
    }

    /// Snapshot of the default output device's stream + its current physical
    /// format, captured up-front so jukebox can restore it on exit (or after a
    /// crash) and leave the user's device the way it found it. Best-effort:
    /// `None` if any CoreAudio read fails — never panics.
    pub struct CapturedFormat {
        stream: AudioStreamID,
        asbd: AudioStreamBasicDescription,
    }

    /// Read the default output device + its first output stream + that stream's
    /// *current* physical format. Returns `None` on any CoreAudio error — this
    /// is best-effort snapshotting, so jukebox never crashes on a quirky
    /// device (e.g. one that exposes no physical format, or gets unplugged
    /// mid-read). When `None`, `restore_output_format` is a no-op.
    pub fn capture_default_format() -> Option<CapturedFormat> {
        let device = default_output_device().ok()?;
        let stream = first_output_stream(device).ok()?;
        let asbd = current_physical_format(stream)?;
        // A zeroed/unknown stream is useless to restore against — treat as
        // "nothing to capture" so the caller's restore is a clean no-op.
        if stream == kAudioObjectUnknown {
            return None;
        }
        Some(CapturedFormat { stream, asbd })
    }

    /// Restore the device's physical format to whatever
    /// `capture_default_format` snapshotted earlier. Best-effort + silent:
    /// CoreAudio errors are ignored (this runs on shutdown, outside the alt-
    /// screen TUI loop, and the one invariant we need is "don't panic"). A
    /// `None` argument (capture failed, or non-macOS) is a no-op.
    pub fn restore_output_format(fmt: Option<CapturedFormat>) {
        if let Some(captured) = fmt {
            // Ignore the result — best-effort restore on shutdown.
            let _ = set_physical_format(captured.stream, captured.asbd);
        }
    }

    fn default_output_device() -> Result<AudioDeviceID> {
        let mut dev: AudioDeviceID = 0;
        let mut size = mem::size_of::<AudioDeviceID>() as u32;
        let addr = AudioObjectPropertyAddress {
            mSelector: kAudioHardwarePropertyDefaultOutputDevice,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };
        let status = unsafe {
            AudioObjectGetPropertyData(
                kAudioObjectSystemObject,
                &addr,
                0,
                ptr::null(),
                &mut size,
                &mut dev as *mut _ as *mut c_void,
            )
        };
        if status != 0 {
            return Err(anyhow!("AudioObjectGetPropertyData(default device) -> {}", status));
        }
        if dev == kAudioObjectUnknown {
            return Err(anyhow!("no default output device"));
        }
        Ok(dev)
    }

    fn first_output_stream(device: AudioDeviceID) -> Result<AudioStreamID> {
        let addr = AudioObjectPropertyAddress {
            mSelector: kAudioDevicePropertyStreams,
            mScope: kAudioObjectPropertyScopeOutput,
            mElement: kAudioObjectPropertyElementMain,
        };
        // Use AudioObjectGetPropertyDataSize for the size pass — some virtual
        // devices (eqMac loopbacks) return `!wh?` (kAudioHardwareUnknownPropertyError)
        // when AudioObjectGetPropertyData is called with a null out-buffer for the
        // size, even though the property is readable. The dedicated size API works.
        let mut size: u32 = 0;
        let status = unsafe {
            AudioObjectGetPropertyDataSize(device, &addr, 0, ptr::null(), &mut size)
        };
        if status != 0 {
            return Err(anyhow!("AudioObjectGetPropertyDataSize(streams) -> {}", status));
        }
        let count = (size / mem::size_of::<AudioStreamID>() as u32) as usize;
        if count == 0 {
            return Err(anyhow!("device has no output streams"));
        }
        let mut streams: Vec<AudioStreamID> = vec![0; count];
        let status = unsafe {
            AudioObjectGetPropertyData(
                device,
                &addr,
                0,
                ptr::null(),
                &mut size,
                streams.as_mut_ptr() as *mut c_void,
            )
        };
        if status != 0 {
            return Err(anyhow!("AudioObjectGetPropertyData(streams) -> {}", status));
        }
        streams
            .into_iter()
            .find(|&s| s != kAudioObjectUnknown)
            .ok_or_else(|| anyhow!("all streams are unknown"))
    }

    /// Return the available physical formats for a stream, as flat
    /// `AudioStreamBasicDescription`s (stripping the range envelope). Falls
    /// back to virtual formats if physical formats aren't exposed (some
    /// virtual/loopback devices only expose virtual).
    fn available_physical_formats(stream: AudioStreamID) -> Result<Vec<AudioStreamBasicDescription>> {
        let addr = AudioObjectPropertyAddress {
            mSelector: kAudioStreamPropertyAvailablePhysicalFormats,
            mScope: kAudioObjectPropertyScopeOutput,
            mElement: kAudioObjectPropertyElementMain,
        };
        let mut size: u32 = 0;
        let status = unsafe {
            AudioObjectGetPropertyDataSize(stream, &addr, 0, ptr::null(), &mut size)
        };
        // Fallback to virtual formats if the device reports no physical formats
        // (common for virtual/loopback devices like eqMac or BlackHole).
        if status != 0 || size == 0 {
            return available_virtual_formats(stream);
        }
        let count = (size / mem::size_of::<AudioStreamRangedDescription>() as u32) as usize;
        if count == 0 {
            return Ok(Vec::new());
        }
        let mut ranged: Vec<AudioStreamRangedDescription> = vec![unsafe { mem::zeroed() }; count];
        let status = unsafe {
            AudioObjectGetPropertyData(
                stream,
                &addr,
                0,
                ptr::null(),
                &mut size,
                ranged.as_mut_ptr() as *mut c_void,
            )
        };
        if status != 0 {
            return Err(anyhow!("AudioObjectGetPropertyData(formats) -> {}", status));
        }
        Ok(ranged.into_iter().map(|r| r.mFormat).collect())
    }

    /// Fallback: virtual formats when a device exposes no physical formats.
    /// Setting the virtual format still drives the underlying DAC at that rate
    /// (the virtual device re-clocks to its physical output).
    fn available_virtual_formats(stream: AudioStreamID) -> Result<Vec<AudioStreamBasicDescription>> {
        let addr = AudioObjectPropertyAddress {
            mSelector: kAudioStreamPropertyAvailableVirtualFormats,
            mScope: kAudioObjectPropertyScopeOutput,
            mElement: kAudioObjectPropertyElementMain,
        };
        let mut size: u32 = 0;
        let status = unsafe {
            AudioObjectGetPropertyDataSize(stream, &addr, 0, ptr::null(), &mut size)
        };
        if status != 0 {
            return Err(anyhow!("AudioObjectGetPropertyDataSize(virtual formats) -> {}", status));
        }
        let count = (size / mem::size_of::<AudioStreamRangedDescription>() as u32) as usize;
        if count == 0 {
            return Ok(Vec::new());
        }
        let mut ranged: Vec<AudioStreamRangedDescription> = vec![unsafe { mem::zeroed() }; count];
        let status = unsafe {
            AudioObjectGetPropertyData(
                stream,
                &addr,
                0,
                ptr::null(),
                &mut size,
                ranged.as_mut_ptr() as *mut c_void,
            )
        };
        if status != 0 {
            return Err(anyhow!("AudioObjectGetPropertyData(virtual formats) -> {}", status));
        }
        Ok(ranged.into_iter().map(|r| r.mFormat).collect())
    }

    fn set_physical_format(stream: AudioStreamID, format: AudioStreamBasicDescription) -> Result<()> {
        // Fast path: if the device is already at the target format, do nothing.
        // Consecutive tracks at the same rate (e.g. a 96k album) skip the
        // switch — and the settle delay — entirely, so there's no gap between
        // tracks of the same format.
        if current_physical_format(stream)
            .map(|cur| same_format(&cur, &format))
            .unwrap_or(false)
        {
            return Ok(());
        }

        let addr = AudioObjectPropertyAddress {
            mSelector: kAudioStreamPropertyPhysicalFormat,
            mScope: kAudioObjectPropertyScopeOutput,
            mElement: kAudioObjectPropertyElementMain,
        };
        let size = mem::size_of::<AudioStreamBasicDescription>() as u32;
        let status = unsafe {
            AudioObjectSetPropertyData(
                stream,
                &addr,
                0,
                ptr::null(),
                size,
                &format as *const _ as *const c_void,
            )
        };
        if status != 0 {
            return Err(anyhow!("AudioObjectSetPropertyData(physical format) -> {}", status));
        }
        // Wait for the device to actually apply the new format before
        // returning. SetPropertyData returns once the property is written, but
        // the DAC takes a few ms to re-clock to the new rate; loading the
        // track into the player before that lands the first audio frames
        // mid-transition, which is the stutter. Poll the current physical
        // format back until it matches the target (or ~250ms, then give up —
        // best-effort, never block playback indefinitely), plus one short
        // settle beat so the stream is ready when mpv pushes the first samples.
        verify_format_landed(stream, &format);
        std::thread::sleep(std::time::Duration::from_millis(60));
        Ok(())
    }

    /// Read the stream's current physical format. Returns `None` if CoreAudio
    /// refuses the read (e.g. the device was unplugged mid-switch).
    fn current_physical_format(stream: AudioStreamID) -> Option<AudioStreamBasicDescription> {
        let addr = AudioObjectPropertyAddress {
            mSelector: kAudioStreamPropertyPhysicalFormat,
            mScope: kAudioObjectPropertyScopeOutput,
            mElement: kAudioObjectPropertyElementMain,
        };
        let mut asbd: AudioStreamBasicDescription = unsafe { mem::zeroed() };
        let mut size = mem::size_of::<AudioStreamBasicDescription>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                stream,
                &addr,
                0,
                ptr::null(),
                &mut size,
                &mut asbd as *mut _ as *mut c_void,
            )
        };
        if status == 0 { Some(asbd) } else { None }
    }

    /// True if `a` and `b` describe the same effective playback format — same
    /// sample rate + bit depth. (Other ASBD fields like channel count matter
    /// less for the rate-switch purpose; we match on the two fields we drive.)
    fn same_format(a: &AudioStreamBasicDescription, b: &AudioStreamBasicDescription) -> bool {
        (a.mSampleRate as u32) == (b.mSampleRate as u32)
            && a.mBitsPerChannel == b.mBitsPerChannel
    }

    /// Poll the device's current physical format until it matches `target`, or
    /// ~250ms elapses. The format change is asynchronous at the device level —
    /// this is the "did it actually take?" check that closes the gap between
    /// "property written" and "DAC running at the new rate".
    fn verify_format_landed(stream: AudioStreamID, target: &AudioStreamBasicDescription) {
        for _ in 0..25 {
            if current_physical_format(stream)
                .map(|cur| same_format(&cur, target))
                .unwrap_or(false)
            {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // Timed out waiting for the format to land — return anyway; the
        // settle sleep in the caller gives the device one last beat. We never
        // block playback indefinitely over a stubborn device.
    }

    /// Pure format matcher: pick the supported `AudioStreamBasicDescription`
    /// closest to `(sample_rate_hz, bit_depth)`. Prefers an exact sample-rate
    /// match, then the closest bit depth; if no exact rate, picks the nearest
    /// rate then nearest bit depth. Exposed for unit testing.
    pub fn match_format(
        available: &[AudioStreamBasicDescription],
        target_sr: u32,
        target_bd: u32,
    ) -> Option<AudioStreamBasicDescription> {
        // Only consider PCM float formats (the standard for DAC playback).
        let pcm: Vec<&AudioStreamBasicDescription> = available
            .iter()
            .filter(|f| f.mFormatID == kAudioFormatLinearPCM)
            .filter(|f| f.mSampleRate > 0.0 && f.mBitsPerChannel > 0)
            .collect();
        if pcm.is_empty() {
            return None;
        }
        // Prefer an exact sample-rate match; among those, closest bit depth.
        let exact_rate: Vec<&&AudioStreamBasicDescription> =
            pcm.iter().filter(|f| (f.mSampleRate as u32) == target_sr).collect();
        let pool: Vec<&&AudioStreamBasicDescription> = if !exact_rate.is_empty() {
            exact_rate
        } else {
            pcm.iter().collect()
        };
        // Closest sample rate (in Hz) as a tiebreaker, then closest bit depth.
        pool.iter()
            .min_by_key(|f| {
                let sr_delta = (f.mSampleRate as i64 - target_sr as i64).abs();
                let bd_delta = (f.mBitsPerChannel as i64 - target_bd as i64).abs();
                // Exact-rate pool already has sr_delta==0; for the fallback pool
                // weight rate more heavily than bit depth (a wrong rate is worse).
                (sr_delta, bd_delta)
            })
            .map(|f| **f)
            .copied()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn asbd(sr: f64, bd: u32) -> AudioStreamBasicDescription {
            let mut f: AudioStreamBasicDescription = unsafe { mem::zeroed() };
            f.mSampleRate = sr;
            f.mFormatID = kAudioFormatLinearPCM;
            f.mBitsPerChannel = bd;
            f
        }

        #[test]
        fn exact_match_preferred() {
            let avail = vec![asbd(44100.0, 16), asbd(96000.0, 24), asbd(48000.0, 16)];
            let m = match_format(&avail, 48000, 16).unwrap();
            assert_eq!(m.mSampleRate as u32, 48000);
            assert_eq!(m.mBitsPerChannel, 16);
        }

        #[test]
        fn closest_bit_depth_among_exact_rate() {
            let avail = vec![asbd(96000.0, 16), asbd(96000.0, 24), asbd(96000.0, 32)];
            let m = match_format(&avail, 96000, 24).unwrap();
            assert_eq!(m.mBitsPerChannel, 24);
        }

        #[test]
        fn no_exact_rate_picks_nearest_rate_then_bit() {
            let avail = vec![asbd(44100.0, 16), asbd(48000.0, 16), asbd(96000.0, 24)];
            // target 88200Hz/24bit — nearest rate is 96000, then nearest bit is 24.
            let m = match_format(&avail, 88200, 24).unwrap();
            assert_eq!(m.mSampleRate as u32, 96000);
            assert_eq!(m.mBitsPerChannel, 24);
        }

        #[test]
        fn ignores_non_pcm_and_zero_entries() {
            let mut bad = asbd(48000.0, 16);
            bad.mFormatID = 0; // not LinearPCM
            let avail = vec![bad, asbd(48000.0, 24)];
            let m = match_format(&avail, 48000, 24).unwrap();
            assert_eq!(m.mBitsPerChannel, 24);
        }

        #[test]
        fn empty_returns_none() {
            assert!(match_format(&[], 48000, 16).is_none());
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod inner {
    use anyhow::Result;
    pub fn set_output_format(_sample_rate_hz: u32, _bit_depth: u32) -> Result<()> {
        Ok(())
    }

    /// No-op capture on non-macOS: there's nothing to restore, so `None`.
    pub struct CapturedFormat;
    pub fn capture_default_format() -> Option<CapturedFormat> {
        None
    }
    pub fn restore_output_format(_fmt: Option<CapturedFormat>) {}
}

pub use inner::{capture_default_format, restore_output_format, set_output_format, CapturedFormat};
