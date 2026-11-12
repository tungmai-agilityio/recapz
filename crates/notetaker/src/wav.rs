//! Writes captured 16 kHz mono PCM as a WAV file, so the recording is playable and the speech
//! step can read it back. The size fields are patched on `finish`, so a crash leaves a file whose
//! header says zero length; the samples are still on disk.
use std::io::{self, Seek, SeekFrom, Write};

const RATE: u32 = 16_000;

pub struct WavWriter<W: Write + Seek> {
    out: W,
    bytes: u32,
}

impl<W: Write + Seek> WavWriter<W> {
    pub fn new(mut out: W) -> io::Result<Self> {
        let mut h = Vec::with_capacity(44);
        h.extend(b"RIFF");
        h.extend(0u32.to_le_bytes());
        h.extend(b"WAVEfmt ");
        h.extend(16u32.to_le_bytes());
        h.extend(1u16.to_le_bytes()); // PCM
        h.extend(1u16.to_le_bytes()); // mono
        h.extend(RATE.to_le_bytes());
        h.extend((RATE * 2).to_le_bytes());
        h.extend(2u16.to_le_bytes());
        h.extend(16u16.to_le_bytes());
        h.extend(b"data");
        h.extend(0u32.to_le_bytes());
        out.write_all(&h)?;
        Ok(Self { out, bytes: 0 })
    }

    pub fn write(&mut self, samples: &[i16]) -> io::Result<()> {
        let buf: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        self.out.write_all(&buf)?;
        // ponytail: WAV sizes are 32-bit, so a recording tops out near 37 hours; split files if needed.
        self.bytes += buf.len() as u32;
        Ok(())
    }

    pub fn finish(mut self) -> io::Result<W> {
        self.out.seek(SeekFrom::Start(4))?;
        self.out.write_all(&(36 + self.bytes).to_le_bytes())?;
        self.out.seek(SeekFrom::Start(40))?;
        self.out.write_all(&self.bytes.to_le_bytes())?;
        self.out.seek(SeekFrom::End(0))?;
        self.out.flush()?;
        Ok(self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn header_sizes_match_the_samples_written() {
        let mut w = WavWriter::new(Cursor::new(Vec::new())).unwrap();
        w.write(&[1, -2, 3]).unwrap();
        w.write(&[256]).unwrap();
        let b = w.finish().unwrap().into_inner();
        assert_eq!(b.len(), 44 + 8);
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 36 + 8);
        assert_eq!(&b[8..16], b"WAVEfmt ");
        let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let u16_at = |o: usize| u16::from_le_bytes(b[o..o + 2].try_into().unwrap());
        assert_eq!(
            (u32_at(16), u16_at(20), u16_at(22)),
            (16, 1, 1),
            "fmt size, PCM, mono"
        );
        assert_eq!(
            (u32_at(24), u32_at(28)),
            (16_000, 32_000),
            "rate, byte rate"
        );
        assert_eq!((u16_at(32), u16_at(34)), (2, 16), "block align, bits");
        assert_eq!(&b[36..40], b"data");
        assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()), 8);
        let s: Vec<i16> = b[44..]
            .chunks(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(s, vec![1, -2, 3, 256]);
    }

    #[test]
    fn empty_recording_is_a_valid_header_only_file() {
        let b = WavWriter::new(Cursor::new(Vec::new()))
            .unwrap()
            .finish()
            .unwrap()
            .into_inner();
        assert_eq!(b.len(), 44);
        assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()), 0);
    }
}
