use anyhow::{anyhow, Context, Result};
use rubato::{FftFixedIn, Resampler};
use std::path::Path;
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error, formats::FormatOptions,
    io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};

pub const TARGET_RATE: u32 = 16_000;

/// Decodes any supported audio file to 16 kHz mono f32 samples in [-1, 1].
pub fn decode_to_16k_mono(path: &Path) -> Result<Vec<f32>> {
    let file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut format = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .context("unsupported audio format")?
        .format;
    let track = format
        .default_track()
        .ok_or_else(|| anyhow!("no audio track"))?
        .clone();
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("unsupported codec")?;

    let mut rate = track.codec_params.sample_rate.unwrap_or(0);
    let mut mono: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track.id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(Error::DecodeError(_)) => continue, // skip a corrupt frame
            Err(e) => return Err(e.into()),
        };
        let spec = *decoded.spec();
        rate = spec.rate;
        let ch = spec.channels.count().max(1);
        let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buf.copy_interleaved_ref(decoded);
        mono.extend(
            buf.samples()
                .chunks(ch)
                .map(|f| f.iter().sum::<f32>() / ch as f32),
        );
    }
    if rate == 0 || mono.is_empty() {
        return Err(anyhow!("no audio samples"));
    }
    resample(mono, rate)
}

fn resample(input: Vec<f32>, rate: u32) -> Result<Vec<f32>> {
    if rate == TARGET_RATE {
        return Ok(input);
    }
    const CHUNK: usize = 4096;
    let mut r = FftFixedIn::<f32>::new(rate as usize, TARGET_RATE as usize, CHUNK, 2, 1)?;
    let mut out = Vec::with_capacity(input.len() * TARGET_RATE as usize / rate as usize + CHUNK);
    let mut pos = 0;
    while input.len() - pos >= r.input_frames_next() {
        let n = r.input_frames_next();
        out.extend(&r.process(&[&input[pos..pos + n]], None)?[0]);
        pos += n;
    }
    if pos < input.len() {
        out.extend(&r.process_partial(Some(&[&input[pos..]]), None)?[0]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 16-bit PCM WAV, interleaved `channels`, one sine tone.
    fn wav(rate: u32, channels: u16, secs: u32, hz: f32) -> Vec<u8> {
        let frames = rate * secs;
        let mut data = Vec::new();
        for i in 0..frames {
            let s = ((i as f32 / rate as f32 * hz * std::f32::consts::TAU).sin() * 16000.0) as i16;
            for _ in 0..channels {
                data.extend(s.to_le_bytes());
            }
        }
        let mut w = Vec::new();
        w.extend(b"RIFF");
        w.extend((36 + data.len() as u32).to_le_bytes());
        w.extend(b"WAVEfmt ");
        w.extend(16u32.to_le_bytes());
        w.extend(1u16.to_le_bytes());
        w.extend(channels.to_le_bytes());
        w.extend(rate.to_le_bytes());
        w.extend((rate * channels as u32 * 2).to_le_bytes());
        w.extend((channels * 2).to_le_bytes());
        w.extend(16u16.to_le_bytes());
        w.extend(b"data");
        w.extend((data.len() as u32).to_le_bytes());
        w.extend(data);
        w
    }

    fn decode(bytes: Vec<u8>) -> Vec<f32> {
        let dir = std::env::temp_dir().join(format!("recapz-speech-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("t{}.wav", bytes.len()));
        std::fs::write(&p, bytes).unwrap();
        decode_to_16k_mono(&p).unwrap()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn stereo_44k_becomes_16k_mono_of_same_duration() {
        let out = decode(wav(44_100, 2, 2, 440.0));
        assert!(out.len().abs_diff(32_000) < 600, "len {}", out.len());
        // sine of amplitude ~0.49 has rms ~0.35; resampling must preserve it
        let r = rms(&out[2000..out.len() - 2000]);
        assert!((r - 0.346).abs() < 0.03, "rms {r}");
    }

    #[test]
    fn already_16k_mono_is_passed_through() {
        let out = decode(wav(16_000, 1, 1, 300.0));
        assert_eq!(out.len(), 16_000);
    }

    #[test]
    fn garbage_is_an_error() {
        let p = std::env::temp_dir().join(format!("recapz-bad-{}.wav", std::process::id()));
        std::fs::write(&p, b"not audio at all").unwrap();
        assert!(decode_to_16k_mono(&p).is_err());
    }
}
