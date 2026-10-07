//! Sentence embeddings for memory. The real model is BAAI/bge-small-en-v1.5 (MIT, 384-d) run with candle
//! on the CPU; `HashingEmbedder` is a deterministic, dependency-free stand-in for tests and for running
//! before the model is present. Each stored embedding records its `model_id`, and the engine re-embeds
//! from stored text when the model changes, so the two never get compared with each other.

use std::fmt::Display;
use std::path::Path;

use candle_core::{Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config, DTYPE};
use tokenizers::{Tokenizer, TruncationDirection, TruncationParams, TruncationStrategy};

use crate::error::{AutoPaperError, Result};
use crate::model::Concept;
use crate::ports::Embedder;
use crate::text;

/// Files the model directory must hold (from the pinned Hugging Face revision; see
/// `scripts/fetch-model.sh` and `docs/research/rust-linux.md` for URLs and SHA-256s).
pub const MODEL_FILES: [&str; 3] = ["config.json", "tokenizer.json", "model.safetensors"];

/// The text that represents a concept in memory: title, summary, setting, subject, elements, time of
/// day, weather, season, mood and palette, joined into one plain sentence-like string. Style,
/// composition and the prompt are left out (they say how, not what).
///
/// Each part becomes a sentence (lists joined with ", "; time of day, weather and season share one), in
/// that order, with empty parts skipped and no field labels, so concepts aren't made to look alike by
/// boilerplate. Fields keep their case (the model's tokenizer lower-cases). Example: "Black ocean, silver
/// structures. A black sea at night… open ocean. silver towers. waves, mist. night, clear, autumn. calm,
/// eerie. black, silver."
pub fn concept_text(concept: &Concept) -> String {
    let mut out = String::new();
    push_sentence(&mut out, &concept.title);
    push_sentence(&mut out, &concept.summary);
    push_sentence(&mut out, &concept.setting);
    push_sentence(&mut out, &concept.subject);
    push_sentence(&mut out, &join_parts(concept.elements.iter().map(String::as_str)));
    push_sentence(
        &mut out,
        &join_parts([concept.time_of_day.as_str(), concept.weather.as_str(), concept.season.as_str()]),
    );
    push_sentence(&mut out, &join_parts(concept.mood.iter().map(String::as_str)));
    push_sentence(&mut out, &join_parts(concept.palette.iter().map(String::as_str)));
    out
}

/// Non-empty trimmed parts joined with ", ", with a trailing full stop dropped from each.
fn join_parts<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    parts
        .into_iter()
        .map(|part| part.trim().trim_end_matches('.').trim_end())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Appends `piece` (trimmed, inner whitespace collapsed) as a sentence: space-separated, ending in
/// sentence punctuation. Empty pieces are skipped.
fn push_sentence(out: &mut String, piece: &str) {
    let piece = piece.split_whitespace().collect::<Vec<_>>().join(" ");
    if piece.is_empty() {
        return;
    }
    if !out.is_empty() {
        out.push(' ');
    }
    out.push_str(&piece);
    if !piece.ends_with(['.', '!', '?', '…', '。', '！', '？']) {
        out.push('.');
    }
}

/// Cosine similarity of two unit vectors (dot product), clamped to [-1, 1]. Mismatched lengths → 0, and
/// so does a result that isn't a number (a corrupt stored vector).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    if dot.is_nan() { 0.0 } else { dot.clamp(-1.0, 1.0) }
}

/// Scales `v` to unit length in place. Returns false (leaving `v` unchanged) when it has no direction:
/// all zeros, or not finite.
fn l2_normalize(v: &mut [f32]) -> bool {
    let norm = v.iter().map(|x| f64::from(*x) * f64::from(*x)).sum::<f64>().sqrt();
    if !(norm.is_finite() && norm > 0.0) {
        return false;
    }
    for x in v.iter_mut() {
        *x = (f64::from(*x) / norm) as f32;
    }
    true
}

/// bge-small-en-v1.5 via candle (CPU): BERT forward pass, CLS pooling, L2 normalisation (the model's
/// documented recipe). Inputs truncated to 512 tokens. Thread-safe (`Send + Sync`): the weights are
/// read-only and each `embed` call builds its own tensors.
pub struct CandleEmbedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

impl CandleEmbedder {
    pub const MODEL_ID: &'static str = "bge-small-en-v1.5";
    /// Longest input in tokens, `[CLS]` and `[SEP]` included (BERT's position-embedding limit).
    pub const MAX_TOKENS: usize = 512;

    /// Loads from a directory holding `MODEL_FILES`. Fails with `Storage` if a file is missing or can't
    /// be read or parsed. Reads the 133 MB of weights into memory (no memory map, so no `unsafe`).
    pub fn load(model_dir: &Path) -> Result<Self> {
        for name in MODEL_FILES {
            if !model_dir.join(name).is_file() {
                return Err(storage(format!(
                    "embedding model file {name} is missing from {} (run scripts/fetch-model.sh)",
                    model_dir.display()
                )));
            }
        }
        let [config_file, tokenizer_file, weights_file] = MODEL_FILES.map(|name| model_dir.join(name));

        let config: Config = serde_json::from_slice(&read(&config_file)?)
            .map_err(|e| storage(format!("{}: {e}", config_file.display())))?;

        let mut tokenizer =
            Tokenizer::from_file(&tokenizer_file).map_err(|e| storage(format!("{}: {e}", tokenizer_file.display())))?;
        let max_length = Self::MAX_TOKENS.min(config.max_position_embeddings);
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length,
                strategy: TruncationStrategy::LongestFirst,
                stride: 0,
                direction: TruncationDirection::Right,
            }))
            .map_err(|e| storage(format!("{}: {e}", tokenizer_file.display())))?;
        tokenizer.with_padding(None);

        let device = Device::Cpu;
        let vb = VarBuilder::from_buffered_safetensors(read(&weights_file)?, DTYPE, &device)
            .map_err(|e| storage(format!("{}: {e}", weights_file.display())))?;
        let model = BertModel::load(vb, &config).map_err(|e| storage(format!("{}: {e}", weights_file.display())))?;
        Ok(Self { model, tokenizer, device })
    }

    fn forward(&self, text: &str) -> candle_core::Result<Vec<f32>> {
        let encoding = self.tokenizer.encode(text, true).map_err(candle_core::Error::wrap)?;
        let ids = Tensor::new(encoding.get_ids(), &self.device)?.unsqueeze(0)?;
        let type_ids = Tensor::new(encoding.get_type_ids(), &self.device)?.unsqueeze(0)?;
        let mask = Tensor::new(encoding.get_attention_mask(), &self.device)?.unsqueeze(0)?;
        // (1, tokens, 384) → the [CLS] token's hidden state, (384).
        let hidden = self.model.forward(&ids, &type_ids, Some(&mask))?;
        hidden.i((0, 0))?.to_vec1::<f32>()
    }
}

impl Embedder for CandleEmbedder {
    fn model_id(&self) -> &str {
        Self::MODEL_ID
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut vector = self.forward(text).map_err(internal)?;
        if !l2_normalize(&mut vector) {
            return Err(internal("the embedding model returned a vector with no direction"));
        }
        Ok(vector)
    }
}

fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| storage(format!("{}: {e}", path.display())))
}

fn storage(detail: impl Into<String>) -> AutoPaperError {
    AutoPaperError::Storage { detail: detail.into() }
}

fn internal(error: impl Display) -> AutoPaperError {
    AutoPaperError::Internal { detail: format!("embedding: {error}") }
}

/// Deterministic bag-of-features embedding: stems of words (see `text::stem`) and adjacent-stem bigrams,
/// hashed (FNV-1a, fixed seed) into 384 signed buckets, L2-normalised. Related phrasings overlap; it is
/// not semantic, so novelty thresholds are calibrated against the real model, not this one.
///
/// Each stem adds 1 and each bigram 0.5 to its bucket; the bucket is the hash modulo 384 and the sign its
/// top bit. Text with no words (or whose features happen to cancel out) embeds to the zero vector, whose
/// cosine with everything is 0.
pub struct HashingEmbedder;

impl HashingEmbedder {
    pub const MODEL_ID: &'static str = "hashing-v1";
    pub const DIMS: usize = 384;

    /// FNV-1a (64-bit) with the standard offset basis as its fixed seed. Part of `MODEL_ID`'s definition:
    /// changing it means a new model id.
    fn hash(feature: &str) -> u64 {
        const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        feature
            .bytes()
            .fold(OFFSET_BASIS, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(PRIME))
    }

    fn add(vector: &mut [f32], feature: &str, weight: f32) {
        let hash = Self::hash(feature);
        // Both casts are lossless: the remainder is below DIMS, and DIMS fits in u64.
        let bucket = (hash % Self::DIMS as u64) as usize;
        let sign = if hash >> 63 == 1 { -1.0 } else { 1.0 };
        vector[bucket] += sign * weight;
    }
}

impl Embedder for HashingEmbedder {
    fn model_id(&self) -> &str {
        Self::MODEL_ID
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let stems: Vec<String> = text::words(text).iter().map(|word| text::stem(word)).collect();
        let mut vector = vec![0.0_f32; Self::DIMS];
        for stem in &stems {
            Self::add(&mut vector, stem, 1.0);
        }
        // Stems never contain spaces, so a bigram's key can't collide with a stem's.
        for pair in stems.windows(2) {
            Self::add(&mut vector, &format!("{} {}", pair[0], pair[1]), 0.5);
        }
        if !l2_normalize(&mut vector) {
            vector.fill(0.0);
        }
        Ok(vector)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn concept() -> Concept {
        Concept {
            title: "Black ocean, silver structures".into(),
            summary: "A black sea at night with tall silver lattice towers rising from still water.".into(),
            setting: "open ocean far from shore".into(),
            subject: "silver lattice towers".into(),
            elements: vec!["still water".into(), " ".into(), "faint stars.".into()],
            time_of_day: "night".into(),
            weather: String::new(),
            season: "autumn".into(),
            mood: vec!["calm".into(), "eerie".into()],
            palette: vec!["black".into(), "silver".into()],
            style: "matte painting".into(),
            composition: "low horizon".into(),
            keywords_used: vec!["ocean".into()],
            wildcards: vec![],
            prompt: "A wide matte painting of…".into(),
        }
    }

    #[test]
    fn concept_text_joins_what_not_how() {
        assert_eq!(
            concept_text(&concept()),
            "Black ocean, silver structures. A black sea at night with tall silver lattice towers rising from \
             still water. open ocean far from shore. silver lattice towers. still water, faint stars. night, \
             autumn. calm, eerie. black, silver."
        );
    }

    #[test]
    fn concept_text_skips_empty_parts_and_keeps_punctuation() {
        let sparse = Concept {
            title: "  Lantern   festival! ".into(),
            summary: "Lanterns drift over a river?".into(),
            mood: vec!["".into()],
            ..Concept::default()
        };
        assert_eq!(concept_text(&sparse), "Lantern festival! Lanterns drift over a river?");
        assert_eq!(concept_text(&Concept::default()), "");
    }

    #[test]
    fn concept_text_ignores_style_composition_and_prompt() {
        let a = concept();
        let b = Concept {
            style: "photograph".into(),
            composition: "centred".into(),
            prompt: "something else".into(),
            keywords_used: vec![],
            ..concept()
        };
        assert_eq!(concept_text(&a), concept_text(&b));
    }

    #[test]
    fn cosine_of_unit_vectors() {
        let a = [0.6_f32, 0.8];
        let b = [0.8_f32, 0.6];
        assert!((cosine(&a, &a) - 1.0).abs() < 1e-6);
        assert!((cosine(&a, &b) - 0.96).abs() < 1e-6);
        assert!((cosine(&a, &[-0.6, -0.8]) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_edge_cases() {
        assert_eq!(cosine(&[1.0, 0.0], &[1.0]), 0.0);
        assert_eq!(cosine(&[], &[]), 0.0);
        assert_eq!(cosine(&[f32::NAN], &[1.0]), 0.0);
        // Slightly-off unit vectors don't escape [-1, 1].
        assert_eq!(cosine(&[1.000_01], &[1.000_01]), 1.0);
        assert_eq!(cosine(&[-1.000_01], &[1.000_01]), -1.0);
    }

    fn norm(v: &[f32]) -> f32 {
        v.iter().map(|x| x * x).sum::<f32>().sqrt()
    }

    #[test]
    fn hashing_embedder_is_deterministic_unit_length_and_384_wide() {
        let e = HashingEmbedder;
        assert_eq!(e.model_id(), "hashing-v1");
        let a = e.embed("Rainy ruins at the blue hour").unwrap();
        let b = e.embed("Rainy ruins at the blue hour").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), HashingEmbedder::DIMS);
        assert!((norm(&a) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn hashing_embedder_matches_by_stem() {
        let e = HashingEmbedder;
        // Same stems in the same order → identical vectors.
        let a = e.embed("rainy ruins").unwrap();
        let b = e.embed("Rain, RUIN!").unwrap();
        assert!((cosine(&a, &b) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn hashing_embedder_ranks_related_text_above_unrelated() {
        let e = HashingEmbedder;
        let base = e.embed("ancient ruins in the rain at night, lanterns glowing").unwrap();
        let related = e.embed("rainy ancient ruins at night with a glowing lantern").unwrap();
        let unrelated = e.embed("a sunny meadow of tulips by a windmill").unwrap();
        let close = cosine(&base, &related);
        let far = cosine(&base, &unrelated);
        assert!(close > 0.5, "related: {close}");
        assert!(far < 0.3, "unrelated: {far}");
        assert!(close > far);
    }

    #[test]
    fn hashing_embedder_uses_word_order_through_bigrams() {
        let e = HashingEmbedder;
        let a = e.embed("blue hour sky").unwrap();
        let b = e.embed("sky hour blue").unwrap();
        let c = cosine(&a, &b);
        assert!(c < 0.999, "bigrams differ: {c}");
        assert!(c > 0.5, "stems are shared: {c}");
    }

    #[test]
    fn hashing_embedder_gives_zero_vector_for_wordless_text() {
        let e = HashingEmbedder;
        let v = e.embed("  …, !! ").unwrap();
        assert_eq!(v, vec![0.0; HashingEmbedder::DIMS]);
        assert_eq!(cosine(&v, &e.embed("rain").unwrap()), 0.0);
    }

    #[test]
    fn fnv1a_matches_reference_values() {
        // Published FNV-1a 64-bit test vectors.
        assert_eq!(HashingEmbedder::hash(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(HashingEmbedder::hash("a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(HashingEmbedder::hash("foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn candle_embedder_reports_missing_files_as_storage_errors() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), "{}").unwrap();
        let Err(AutoPaperError::Storage { detail }) = CandleEmbedder::load(dir.path()) else {
            panic!("expected a storage error");
        };
        assert!(detail.contains("tokenizer.json"), "{detail}");
    }

    #[test]
    fn candle_embedder_reports_unreadable_files_as_storage_errors() {
        let dir = tempfile::tempdir().unwrap();
        for name in MODEL_FILES {
            std::fs::write(dir.path().join(name), "not what it should be").unwrap();
        }
        assert!(matches!(CandleEmbedder::load(dir.path()), Err(AutoPaperError::Storage { .. })));
    }

    #[test]
    fn candle_embedder_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CandleEmbedder>();
    }
}
