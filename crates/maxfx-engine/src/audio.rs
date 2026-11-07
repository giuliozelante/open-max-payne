//! Sound output: one shared device stream, many sounds mixed in-process, as retail mixes its
//! channels through `sndmfc.dll` on DirectSound. Retail WAVs are mostly Microsoft ADPCM.

use rodio::Source;
use std::cell::RefCell;
use std::io::Cursor;
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

/// One playing sound. Dropping it stops it.
pub struct Sound {
    sink: rodio::Sink,
}

impl Sound {
    /// Start `wav` at `gain` (1 is the file's own level). `None` with no device or a bad file.
    pub fn play(wav: &WavBytes, looping: bool, gain: f32) -> Option<Self> {
        let source = decode(wav)?;
        with_output(|handle| {
            let sink = rodio::Sink::try_new(handle).ok()?;
            sink.set_volume(gain);
            if looping {
                sink.append(source.repeat_infinite());
            } else {
                sink.append(source);
            }
            Some(Self { sink })
        })
    }

    pub fn set_gain(&self, gain: f32) {
        self.sink.set_volume(gain);
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
