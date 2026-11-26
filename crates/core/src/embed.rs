//! Text embeddings for Ask. `Embedder` is the seam for a real model; `HashEmbedder` is the
//! built-in baseline: hashed word and word-pair counts, L2-normalised. It needs no model download
//! and matches shared words, not meaning (FTS covers the lexical side of Ask either way).

/// Question filler that would match almost every chunk: English, then Vietnamese (with marks;
/// a word typed without marks may be a content word, so it is kept).
const STOP: &[&str] = &[
    "là", "của", "và", "các", "những", "một", "có", "không", "được", "trong", "cho", "với", "này",
    "đó", "gì", "nào", "khi", "thế", "sao", "ai", "đâu", "bao", "nhiêu", "tôi", "chúng", "ta",
    "mình", "đã", "sẽ", "đang", "ở", "để", "về", "từ", "như", "a", "an", "the", "is", "are", "was",
    "were", "be", "to", "of", "in", "on", "at", "for", "and", "or", "do", "did", "does", "what",
    "when", "where", "who", "why", "how", "which", "we", "i", "you", "it", "that", "this", "with",
    "about", "from", "by", "as",
];

/// Words of `text` without filler. Combining marks stay inside their word, so a Vietnamese
/// word typed with separate marks (decomposed Unicode) is not cut into pieces.
pub fn keywords(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_alphanumeric() || ('\u{300}'..='\u{36f}').contains(&c)))
        .filter(|w| !w.is_empty() && !is_stop(w))
}

pub fn is_stop(word: &str) -> bool {
    STOP.contains(&word.to_lowercase().as_str())
}

/// The seam for a real model. `id` names the model and its output size (for example
/// `hash-256`); it is stored with every vector, because vectors from different embedders cannot
/// be compared. Change `id` whenever the output would change. `embed` fails instead of returning a
/// wrong vector; callers must not store anything for a failed text.
pub trait Embedder: Send + Sync {
    fn id(&self) -> &'static str;
    fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>>;
    /// Vector matches below this cosine are noise, not hits. It belongs to the embedder because
    /// similarity scores are not comparable across models; measure it with `cargo run -p eval -- ask`.
    fn min_similarity(&self) -> f32 {
        DEFAULT_MIN_SIMILARITY
    }
}

/// Tuned for `HashEmbedder` (shared filler words score low); other embedders override it.
pub const DEFAULT_MIN_SIMILARITY: f32 = 0.2;

pub const HASH_DIM: usize = 256;

pub struct HashEmbedder;

fn bucket(s: &str) -> usize {
    // FNV-1a, stable across runs and platforms (stored vectors must stay comparable).
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    (h % HASH_DIM as u64) as usize
}

impl Embedder for HashEmbedder {
    fn id(&self) -> &'static str {
        "hash-256"
    }

    fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let words: Vec<String> = keywords(text).map(|w| w.to_lowercase()).collect();
        let mut v = vec![0f32; HASH_DIM];
        for (i, w) in words.iter().enumerate() {
            v[bucket(w)] += 1.0;
            if let Some(n) = words.get(i + 1) {
                v[bucket(&format!("{w} {n}"))] += 1.0;
            }
        }
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            v.iter_mut().for_each(|x| *x /= norm);
        }
        Ok(v)
    }
}

/// f32 little-endian, the `chunks.embedding` format.
pub fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub fn from_blob(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect()
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_round_trips() {
        let v = vec![0.5, -1.25, 3.0];
        assert_eq!(from_blob(&to_blob(&v)), v);
    }

    #[test]
    fn same_text_is_identical_and_related_text_beats_unrelated() {
        let e = HashEmbedder;
        let a = e
            .embed("we agreed to ship the pricing page on Friday")
            .unwrap();
        assert_eq!(a.len(), HASH_DIM);
        assert!((cosine(&a, &a) - 1.0).abs() < 1e-5);
        let related = e.embed("Pricing page ships Friday, agreed").unwrap();
        let other = e.embed("lunch menu includes noodles and tea").unwrap();
        assert!(cosine(&a, &related) > cosine(&a, &other) + 0.2);
    }

    #[test]
    fn empty_text_is_the_zero_vector() {
        assert!(HashEmbedder.embed("  ").unwrap().iter().all(|x| *x == 0.0));
    }
}
