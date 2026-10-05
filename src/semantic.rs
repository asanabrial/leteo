//! The model behind the semantic search stage, and nothing else about it.
//!
//! A static embedding model: one vector per WordPiece token, averaged. There is
//! no network, no server and no inference runtime — encoding a question is a
//! tokenizer and a mean — so the model is three files compiled into the binary
//! and loaded from memory the first time something asks for a vector.
//!
//! Which model, why, and where the bytes come from are in
//! `tools/semantic/README.md` and `openspec/specs/search.md` §15. What this
//! module owns is the numbers that belong to the model rather than to the
//! search: its name, its width, how much of a memory it reads, and the cosine
//! below which two texts are not saying the same thing.

use std::io::Read;
use std::sync::OnceLock;

use model2vec_rs::model::StaticModel;

static TOKENIZER: &[u8] = include_bytes!("../assets/model/tokenizer.json.gz");
static WEIGHTS: &[u8] = include_bytes!("../assets/model/model.safetensors");
static CONFIG: &[u8] = include_bytes!("../assets/model/config.json");

/// What computed a stored vector.
///
/// Written beside every vector and compared on every read: vectors from two
/// models are not comparable, so a build that ships a different model finds
/// every stored row stale and embeds it again. Change it whenever the bytes
/// under `assets/model/` change.
pub const MODEL_ID: &str = "static-similarity-mrl-multilingual-v1/256-int8-pruned";

/// The width of a vector, and of the row the table must hold.
pub const DIMENSIONS: usize = 256;

/// How many tokens of a memory are read.
///
/// The first 128, from the title and then the body. Measured on this model over
/// a copy of a real store: 128 beat 64 and 256, and at 512 the semantic MRR of
/// bodies fell from .57 to .35. A mean over more text is a mean that says less,
/// and the start of a memory is where it says what it is about.
pub const MAX_TOKENS: usize = 128;

/// The cosine below which a semantic answer to an empty question is not given.
///
/// Without a floor the stage answers every empty question, including those the
/// store cannot answer: asked in the other project, where its target does not
/// exist, 100% of them. At 0.30, on half the empty answers of the hard set
/// calibrated and reported on the other half, it keeps 43% of what can be
/// rescued at 43% precision and answers 24% of the controls (±12 points at 55
/// controls). 0.25 keeps 67% and answers 42%; 0.35 keeps 25% and answers 9%.
///
/// It costs Basque the most, which the model was not trained on: queries in
/// Basque score .177 without the floor and .044 with it.
pub const FLOOR: f32 = 0.30;

/// Why a vector could not be made.
#[derive(Debug)]
pub struct EmbedError(String);

impl std::fmt::Display for EmbedError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for EmbedError {}

/// The model, loaded once per process and only when a stage first asks.
///
/// Loading costs 13 to 15 ms and about 93 MB of resident memory against 14 MB
/// without it — the int8 table is expanded to f32 — so a process that never
/// reaches the semantic stage never pays either. That is every hook and almost
/// every search: the stage runs only on an empty or a `nearest` answer.
fn model() -> Result<&'static StaticModel, EmbedError> {
    static MODEL: OnceLock<Result<StaticModel, String>> = OnceLock::new();
    MODEL
        .get_or_init(|| {
            let mut tokenizer = Vec::new();
            flate2::read::GzDecoder::new(TOKENIZER)
                .read_to_end(&mut tokenizer)
                .map_err(|error| format!("the embedded tokenizer did not decompress: {error}"))?;
            StaticModel::from_bytes(&tokenizer, WEIGHTS, CONFIG, Some(true))
                .map_err(|error| format!("the embedded model did not load: {error}"))
        })
        .as_ref()
        .map_err(|message| EmbedError(message.clone()))
}

/// The text a memory is embedded from.
///
/// The title and then the body, one string, so the title — the part that says
/// what the memory is — is always inside the token budget however long the body
/// is.
pub fn document_text(title: &str, content: &str) -> String {
    format!("{title}\n{content}")
}

/// L2-normalised vectors for these texts, in order.
///
/// An empty vector for a text with no token the model knows: its mean is
/// undefined, and a zero vector would score zero against everything, which is
/// the same answer said without the reason.
///
/// The tokenizer expects its input to be well-formed and panics when it is not
/// (`encode_batch_fast(..).expect(..)` in the crate). A panic inside a search
/// would take the MCP server down for a question that has an answer without
/// this stage, so it is turned into the error it is.
pub fn embed(texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
    let model = model()?;
    let vectors = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        model.encode_with_args(texts, Some(MAX_TOKENS), 1024)
    }))
    .map_err(|_| EmbedError("the tokenizer failed on this text".to_owned()))?;
    Ok(vectors.into_iter().map(normalised).collect())
}

fn normalised(mut vector: Vec<f32>) -> Vec<f32> {
    let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if vector.len() != DIMENSIONS || !norm.is_finite() || norm < 1e-6 {
        return Vec::new();
    }
    for component in &mut vector {
        *component /= norm;
    }
    vector
}

/// A vector as the bytes the table keeps: little-endian f32, in order.
pub fn encode(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// The cosine of a stored vector with a question, or `None` for a row that is
/// not a vector of this model's width.
///
/// Both sides are unit length, so the cosine is the dot product. Read from the
/// stored bytes without building a `Vec`: the scan runs over every memory in
/// scope, 6.6 ms over 5,336 on the measured store, and an allocation per row
/// would be most of that.
pub fn cosine(stored: &[u8], question: &[f32]) -> Option<f32> {
    if stored.len() != DIMENSIONS * 4 || question.len() != DIMENSIONS {
        return None;
    }
    let (components, _) = stored.as_chunks::<4>();
    let mut sum = 0.0f32;
    for (bytes, q) in components.iter().zip(question) {
        sum += f32::from_le_bytes(*bytes) * q;
    }
    Some(sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    /// The model the binary carries loads, and says what it should.
    ///
    /// A model that failed to load would not fail a build: the stage degrades
    /// to silence and every other test passes. This is the one place that says
    /// the bytes under `assets/model/` are a model.
    #[test]
    fn the_embedded_model_loads_and_has_the_published_width() {
        let vectors = embed(&texts(&["database connection pool exhaustion"])).unwrap();
        assert_eq!(vectors.len(), 1);
        assert_eq!(vectors[0].len(), DIMENSIONS);
        let norm: f32 = vectors[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "not unit length: {norm}");
    }

    /// Meaning rather than words: a question that shares no word with its
    /// target is closer to it than to an unrelated memory, and so is the same
    /// question in another language. This is the property the whole stage rests
    /// on, and it is asserted with a margin rather than an order, because the
    /// floor the stage applies is a number and not a ranking.
    #[test]
    fn a_question_in_other_words_is_nearer_its_target_than_an_unrelated_memory() {
        let vectors = embed(&texts(&[
            "Product catalog cache stampede on cold start\nWhen the catalog cache expired, \
             hundreds of requests recomputed it at once.",
            "Rotate JWT signing keys every 30 days with kid header\nSigning keys live in KMS \
             and tokens carry a kid header.",
            "How to bake sourdough bread\nMix flour and water, let the starter ferment \
             overnight, bake in a hot oven.",
            "thundering herd when the cache expires",
            "rotación de las claves de firma",
        ]))
        .unwrap();
        let (cache, keys, bread) = (
            encode(&vectors[0]),
            encode(&vectors[1]),
            encode(&vectors[2]),
        );
        let herd = cosine(&cache, &vectors[3]).unwrap();
        let herd_far = cosine(&bread, &vectors[3]).unwrap();
        assert!(
            herd > FLOOR && herd > herd_far + 0.2,
            "{herd} against {herd_far}"
        );
        let spanish = cosine(&keys, &vectors[4]).unwrap();
        let spanish_far = cosine(&bread, &vectors[4]).unwrap();
        assert!(
            spanish > FLOOR && spanish > spanish_far + 0.2,
            "{spanish} against {spanish_far}"
        );
    }

    #[test]
    fn a_text_with_no_known_token_has_no_vector_rather_than_a_zero_one() {
        let vectors = embed(&texts(&["\u{200b}\u{200b}"])).unwrap();
        assert!(vectors[0].is_empty());
    }

    #[test]
    fn a_stored_row_of_the_wrong_width_scores_nothing() {
        let question = vec![0.0f32; DIMENSIONS];
        assert_eq!(cosine(&[0u8; 12], &question), None);
        assert_eq!(cosine(&vec![0u8; DIMENSIONS * 4], &question), Some(0.0));
    }
}
