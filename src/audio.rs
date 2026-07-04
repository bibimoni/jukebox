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

    fn default_output_device() -> Result<AudioDeviceID> {
        let mut dev: AudioDeviceID = 0;
        let mut size = mem::size_of::<AudioDeviceID>() as u32;
        let addr = AudioObjectPropertyAddress {
            mSelector: kAudioHardwarePropertyDefaultOutputDevice,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMaster,
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
            mElement: kAudioObjectPropertyElementMaster,
        };
        // Two-phase: ask for size, then read.
        let mut size: u32 = 0;
        let status = unsafe {
            AudioObjectGetPropertyData(
                device,
                &addr,
                0,
                ptr::null(),
                &mut size,
                ptr::null_mut(),
            )
        };
        if status != 0 {
            return Err(anyhow!("AudioObjectGetPropertyData(streams size) -> {}", status));
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
    /// `AudioStreamBasicDescription`s (stripping the range envelope).
    fn available_physical_formats(stream: AudioStreamID) -> Result<Vec<AudioStreamBasicDescription>> {
        let addr = AudioObjectPropertyAddress {
            mSelector: kAudioStreamPropertyAvailablePhysicalFormats,
            mScope: kAudioObjectPropertyScopeOutput,
            mElement: kAudioObjectPropertyElementMaster,
        };
        let mut size: u32 = 0;
        let status = unsafe {
            AudioObjectGetPropertyData(stream, &addr, 0, ptr::null(), &mut size, ptr::null_mut())
        };
        if status != 0 {
            return Err(anyhow!("AudioObjectGetPropertyData(formats size) -> {}", status));
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

    fn set_physical_format(stream: AudioStreamID, format: AudioStreamBasicDescription) -> Result<()> {
        let addr = AudioObjectPropertyAddress {
            mSelector: kAudioStreamPropertyPhysicalFormat,
            mScope: kAudioObjectPropertyScopeOutput,
            mElement: kAudioObjectPropertyElementMaster,
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
        Ok(())
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
}

pub use inner::set_output_format;
