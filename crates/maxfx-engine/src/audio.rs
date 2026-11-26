//! Sound output: one shared device stream, many sounds mixed in-process, as retail mixes its
//! channels through `sndmfc.dll` on DirectSound. Retail WAVs are mostly Microsoft ADPCM.

use rodio::Source;
use std::cell::RefCell;
use std::io::Cursor;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

thread_local! {
    /// The device stream. `None` once opening failed, so a machine with no sound device stays
    /// quiet instead of retrying every sound. rodio's stream must stay on the thread that opened it.
    static OUTPUT: RefCell<Option<Option<(rodio::OutputStream, rodio::OutputStreamHandle)>>> =
        const { RefCell::new(None) };
}

fn with_output<T>(f: impl FnOnce(&rodio::OutputStreamHandle) -> Option<T>) -> Option<T> {
    OUTPUT.with(|output| {
        let mut output = output.borrow_mut();
        let opened = output.get_or_insert_with(|| rodio::OutputStream::try_default().ok());
        opened.as_ref().and_then(|(_, handle)| f(handle))
    })
}

/// A WAV file's bytes, shared between the sounds that play it.
pub type WavBytes = Arc<[u8]>;

/// The samples of `wav`, or `None` when it does not decode.
pub fn decode(wav: &WavBytes) -> Option<rodio::Decoder<Cursor<WavBytes>>> {
    rodio::Decoder::new(Cursor::new(wav.clone())).ok()
}

/// Left and right gains of an equal-power pan: `side` -1 is hard left, 0 the middle, 1 hard
/// right. HYPOTHESIS: retail leaves panning to the DirectSound3D driver; this is the sine/cosine
/// law of OpenAL Soft, which DSOAL runs on.
pub fn pan_gains(side: f32) -> [f32; 2] {
    let angle = (side.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    [angle.cos(), angle.sin()]
}

/// A mono source played on two channels at the pan in `side` (`f32` bits), which the game may
/// change while it plays.
pub struct Panned<S> {
    input: S,
    side: Arc<AtomicU32>,
    right: Option<f32>,
}

impl<S: Source<Item = i16>> Panned<S> {
    pub fn new(input: S, side: Arc<AtomicU32>) -> Self {
        Self {
            input,
            side,
            right: None,
        }
    }
}

impl<S: Source<Item = i16>> Iterator for Panned<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if let Some(right) = self.right.take() {
            return Some(right);
        }
        let sample = rodio::Sample::to_f32(self.input.next()?);
        let [l, r] = pan_gains(f32::from_bits(self.side.load(Ordering::Relaxed)));
        self.right = Some(sample * r);
        Some(sample * l)
    }
}

impl<S: Source<Item = i16>> Source for Panned<S> {
    fn current_frame_len(&self) -> Option<usize> {
        self.input.current_frame_len().map(|len| len * 2)
    }

    fn channels(&self) -> u16 {
        2
    }

    fn sample_rate(&self) -> u32 {
        self.input.sample_rate()
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        self.input.total_duration()
    }
}

/// One playing sound. Dropping it stops it.
pub struct Sound {
    sink: rodio::Sink,
    /// The pan of a positional mono sound; `None` plays the file as it is.
    side: Option<Arc<AtomicU32>>,
}

impl Sound {
    /// Start `wav` at `gain` (1 is the file's own level), panned to `side` when given and the
    /// file is mono. `None` with no device or a bad file.
    pub fn play(wav: &WavBytes, looping: bool, gain: f32, side: Option<f32>) -> Option<Self> {
        let source = decode(wav)?;
        let side = side
            .filter(|_| source.channels() == 1)
            .map(|side| Arc::new(AtomicU32::new(side.to_bits())));
        with_output(|handle| {
            let sink = rodio::Sink::try_new(handle).ok()?;
            sink.set_volume(gain);
            match (&side, looping) {
                (Some(side), true) => {
                    sink.append(Panned::new(source, side.clone()).repeat_infinite())
                }
                (Some(side), false) => sink.append(Panned::new(source, side.clone())),
                (None, true) => sink.append(source.repeat_infinite()),
                (None, false) => sink.append(source),
            }
            Some(Self { sink, side })
        })
    }

    pub fn set_gain(&self, gain: f32) {
        self.sink.set_volume(gain);
    }

    /// Move a panned sound to `side`; a sound started without a pan stays as it is.
    pub fn set_side(&self, side: f32) {
        if let Some(pan) = &self.side {
            pan.store(side.to_bits(), Ordering::Relaxed);
        }
    }

    pub fn gain(&self) -> f32 {
        self.sink.volume()
    }

    /// A one-shot that has played to its end.
    pub fn finished(&self) -> bool {
        self.sink.empty()
    }
}

impl Drop for Sound {
    fn drop(&mut self) {
        self.sink.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mono 16-bit PCM WAV of `samples`.
    fn wav(samples: &[i16]) -> WavBytes {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&22050u32.to_le_bytes());
        out.extend_from_slice(&44100u32.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out.into()
    }

    #[test]
    fn a_pcm_wav_decodes_to_its_samples() {
        let source = decode(&wav(&[0, 16384, -16384])).expect("decodes");
        assert_eq!(source.channels(), 1);
        assert_eq!(source.sample_rate(), 22050);
        assert_eq!(source.count(), 3);
    }

    #[test]
    fn equal_power_pan_splits_the_middle_and_hard_sides() {
        let [l, r] = pan_gains(0.0);
        assert!((l - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((r - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        let [l, r] = pan_gains(1.0);
        assert!(l.abs() < 1e-6 && (r - 1.0).abs() < 1e-6);
        let [l, r] = pan_gains(-1.0);
        assert!((l - 1.0).abs() < 1e-6 && r.abs() < 1e-6);
    }

    #[test]
    fn a_panned_mono_sound_plays_on_the_right_only() {
        let pan = Arc::new(std::sync::atomic::AtomicU32::new(1.0f32.to_bits()));
        let source = decode(&wav(&[16384, -16384])).expect("decodes");
        let panned = Panned::new(source, pan);
        assert_eq!(panned.channels(), 2);
        let samples: Vec<f32> = panned.collect();
        assert_eq!(samples.len(), 4);
        assert!(samples[0].abs() < 1e-6 && samples[2].abs() < 1e-6);
        assert!((samples[1] - 0.5).abs() < 1e-6 && (samples[3] + 0.5).abs() < 1e-6);
    }

    #[test]
    fn bytes_that_are_no_wav_do_not_decode() {
        assert!(decode(&Arc::from(&b"not a wav"[..])).is_none());
    }

    #[test]
    fn retail_adpcm_wavs_decode_when_the_game_is_installed() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        let Some(bytes) =
            crate::read_game_bytes(&game, r"\data\database\sounds\weapons\shoot_beretta.wav")
        else {
            return;
        };
        assert_eq!(&bytes[20..22], &2u16.to_le_bytes(), "Microsoft ADPCM");
        let source = decode(&Arc::from(bytes)).expect("decodes");
        assert_eq!(source.sample_rate(), 44100);
        assert!(source.count() > 1000);
    }
}
