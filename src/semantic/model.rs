//! The model behind the semantic stage, read and pooled in the form it is
//! stored in.
//!
//! The weights are an int8 table, 49,203 rows of 256 values, and nothing else:
//! no per-token weights and no token mapping. `model2vec-rs` 0.3.0 — the crate
//! this reader replaces — loads that table by expanding every byte to an `f32`,
//! which is four bytes where the file has one, and that expansion is what made a
//! 12.6 MB model cost about 50 MB resident. This module keeps the table as the
//! bytes it is stored as and pools over it directly.
//!
//! That the result is the same is arithmetic and not luck: an `i8` is an exact
//! `f32`, the table carries no per-token weight, and the sum of at most
//! `MAX_TOKENS` of them is far below 2^24, where an `f32` stops counting
//! integers. So the mean, and the normalisation after it, are the mean and the
//! normalisation the expansion produced, to the bit. A test in the parent module
//! embeds a fixed set of texts with both readers and requires equality.
//!
//! What is reproduced from `model2vec-rs` is the part that is the model's
//! behaviour and not its arithmetic: the median token length that turns a token
//! budget into a character cut, the character cut applied before tokenizing,
//! dropping the unknown token from the ids, and mean pooling with the crate's
//! normalisation. Those are `compute_metadata`, `truncate_str`, `encode_with_args`
//! and `pool_ids` in its `model.rs`.

use safetensors::SafeTensors;
use safetensors::tensor::Dtype;
use tokenizers::Tokenizer;
use tokenizers::models::ModelWrapper;

use super::{DIMENSIONS, EmbedError};

/// The tensor the table is stored under.
const EMBEDDINGS: &str = "embeddings";

/// The model, verified and read once per process and directory.
///
/// The table is the only large thing here and it is the size of the file, not
/// four times it. Everything the crate used to keep beside it — the expanded
/// `f32` rows, the token mapping, the per-token weights — is either the file's
/// own bytes or absent from the model this build accepts.
#[derive(Debug)]
pub struct SemanticModel {
    tokenizer: Tokenizer,
    /// The table, row-major, one `i8` per stored byte. Read as `i8` where it is
    /// used, so the bytes never leave the form the file has them in.
    table: Vec<i8>,
    normalize: bool,
    median_token_length: usize,
    unk_token_id: Option<usize>,
}

impl SemanticModel {
    /// Read a model from the bytes of its three files.
    ///
    /// The tokenizer is the decompressed `tokenizer.json`; the weights are the
    /// `model.safetensors` bytes, read without being copied out of the form
    /// they are in; the config is `config.json`, read for whether the output is
    /// normalised (it is, and the pinned file says so).
    pub(crate) fn from_parts(
        config: &[u8],
        weights: &[u8],
        tokenizer_json: &[u8],
    ) -> Result<Self, String> {
        let tokenizer = Tokenizer::from_bytes(tokenizer_json)
            .map_err(|error| format!("the tokenizer did not load: {error}"))?;

        let normalize = serde_json::from_slice::<serde_json::Value>(config)
            .map_err(|error| format!("config.json did not parse: {error}"))?
            .get("normalize")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);

        let tensors = SafeTensors::deserialize(weights)
            .map_err(|error| format!("the weights did not parse: {error}"))?;
        let tensor = tensors
            .tensor(EMBEDDINGS)
            .map_err(|error| format!("the {EMBEDDINGS} tensor is not there: {error}"))?;
        let [rows, columns] = <[usize; 2]>::try_from(tensor.shape())
            .map_err(|_| "the embedding table is not two-dimensional".to_owned())?;
        if tensor.dtype() != Dtype::I8 {
            return Err(format!(
                "the embedding table is {:?}, not int8",
                tensor.dtype()
            ));
        }
        if columns != DIMENSIONS {
            return Err(format!(
                "the embedding table is {columns} values wide, not {DIMENSIONS}"
            ));
        }
        let table = tensor
            .data()
            .iter()
            .map(|&byte| byte as i8)
            .collect::<Vec<i8>>();
        if table.len() != rows * columns {
            return Err(format!(
                "the embedding table is {} values, not {rows} rows of {columns}",
                table.len()
            ));
        }

        let (median_token_length, unk_token_id) = metadata(&tokenizer)?;
        Ok(Self {
            tokenizer,
            table,
            normalize,
            median_token_length,
            unk_token_id,
        })
    }

    /// L2-normalised vectors for these texts, in order, one normalisation.
    ///
    /// The parent module's [`super::embed`] is the surface: it catches the
    /// tokenizer's panic on a malformed text and drops the zero vector a text
    /// with no known token would otherwise leave as a mean. This is the model's
    /// own answer, without either.
    pub(crate) fn encode_with_args(
        &self,
        texts: &[String],
        max_length: Option<usize>,
        batch_size: usize,
    ) -> Result<Vec<Vec<f32>>, EmbedError> {
        let mut vectors = Vec::with_capacity(texts.len());
        for batch in texts.chunks(batch_size) {
            let batch: Vec<&str> = batch.iter().map(String::as_str).collect();
            for ids in self.token_ids(&batch, max_length)? {
                vectors.push(self.pool(&ids));
            }
        }
        Ok(vectors)
    }

    /// The token ids the model would pool for each text, in order: cut by
    /// characters, tokenized, the unknown token dropped, and at most
    /// `max_length` kept.
    ///
    /// The half of [`Self::encode_with_args`] before the mean, so a memory can
    /// be cut into windows of ids that each pool exactly as a text of those ids
    /// would. It panics where the tokenizer does, and the caller in the parent
    /// module catches it.
    pub(crate) fn token_ids(
        &self,
        texts: &[&str],
        max_length: Option<usize>,
    ) -> Result<Vec<Vec<u32>>, EmbedError> {
        let truncated: Vec<&str> = texts
            .iter()
            .map(|text| match max_length {
                Some(max_tokens) => truncate(text, max_tokens, self.median_token_length),
                None => text,
            })
            .collect();
        let encodings = self
            .tokenizer
            .encode_batch_fast::<String>(truncated.into_iter().map(Into::into).collect(), false)
            .map_err(|error| EmbedError(format!("the tokenizer failed: {error}")))?;
        Ok(encodings
            .into_iter()
            .map(|encoding| {
                let mut ids = encoding.get_ids().to_vec();
                if let Some(unknown) = self.unk_token_id {
                    ids.retain(|&id| id as usize != unknown);
                }
                if let Some(max_tokens) = max_length {
                    ids.truncate(max_tokens);
                }
                ids
            })
            .collect())
    }

    /// Mean-pool token ids into one vector, over the int8 table.
    ///
    /// Each `i8` is an exact `f32` and the table has no per-token weight, so the
    /// sum is the sum of integers and the division and normalisation after it
    /// are the operations `model2vec-rs` performed on its expanded rows. A row
    /// for an id the table does not reach would be a defect in the tokenizer or
    /// the model rather than a text, and indexing rather than skipping keeps it
    /// a caught panic instead of a wrong vector.
    pub(crate) fn pool(&self, ids: &[u32]) -> Vec<f32> {
        let mut sum = vec![0.0f32; DIMENSIONS];
        let mut count = 0usize;
        for &id in ids {
            let row = id as usize * DIMENSIONS;
            for (total, value) in sum.iter_mut().zip(&self.table[row..row + DIMENSIONS]) {
                *total += f32::from(*value);
            }
            count += 1;
        }
        let divisor = count.max(1) as f32;
        for total in &mut sum {
            *total /= divisor;
        }
        if self.normalize {
            let norm = sum
                .iter()
                .map(|value| value * value)
                .sum::<f32>()
                .sqrt()
                .max(1e-12);
            for total in &mut sum {
                *total /= norm;
            }
        }
        sum
    }
}

/// The median token length and the unknown-token id, read the way `model2vec-rs`
/// read them.
///
/// The median length is what turns the token budget into the character cut
/// applied before tokenizing, taken from the vocabulary so the cut is a property
/// of the model rather than a second number that has to be kept in step with it.
fn metadata(tokenizer: &Tokenizer) -> Result<(usize, Option<usize>), String> {
    let mut lengths: Vec<usize> = tokenizer.get_vocab(false).keys().map(String::len).collect();
    lengths.sort_unstable();
    let median_token_length = lengths.get(lengths.len() / 2).copied().unwrap_or(1);

    let id = |token: &str| {
        tokenizer
            .token_to_id(token)
            .map(|id| id as usize)
            .ok_or_else(|| format!("the unknown token {token} is not in the vocabulary"))
    };
    let unk_token_id = match tokenizer.get_model() {
        ModelWrapper::BPE(model) => model.unk_token.as_deref().map(id).transpose()?,
        ModelWrapper::WordPiece(model) => Some(id(&model.unk_token)?),
        ModelWrapper::WordLevel(model) => Some(id(&model.unk_token)?),
        // `tokenizers` keeps Unigram's `unk_id` private, so it is read from the
        // serialized model, exactly as the crate did.
        ModelWrapper::Unigram(model) => serde_json::to_value(model)
            .map_err(|error| format!("the unigram model did not serialize: {error}"))?
            .get("unk_id")
            .and_then(serde_json::Value::as_u64)
            .map(|id| id as usize),
    };
    Ok((median_token_length, unk_token_id))
}

/// Cut a text to `max_tokens * median_length` characters before tokenizing.
///
/// By characters and not bytes, so a cut never lands inside one; the tokenizer
/// then counts the real tokens.
fn truncate(text: &str, max_tokens: usize, median_length: usize) -> &str {
    text.char_indices()
        .nth(max_tokens.saturating_mul(median_length))
        .map_or(text, |(byte, _)| &text[..byte])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::MAX_TOKENS;

    /// The three files built from a tokenizer and a table this test owns.
    ///
    /// The equivalence test in the parent module needs the real model and returns
    /// early in a tree that has none — the packaged crate — so without this the
    /// reader's arithmetic and its refusals would be proved only where
    /// `assets/model/` happens to be checked out. This is what makes them proved
    /// everywhere, on bytes whose every value the test chose.
    const TOKENIZER: &str = r#"{
        "version": "1.0",
        "truncation": null,
        "padding": null,
        "added_tokens": [],
        "normalizer": null,
        "pre_tokenizer": {"type": "Whitespace"},
        "post_processor": null,
        "decoder": null,
        "model": {
            "type": "WordPiece",
            "unk_token": "[UNK]",
            "continuing_subword_prefix": "%%",
            "max_input_chars_per_word": 100,
            "vocab": {"[UNK]": 0, "hello": 1, "world": 2}
        }
    }"#;

    /// Three rows: `[UNK]` is row 0, `hello` row 1, `world` row 2. Row 1 is
    /// `[3, -5, 0, ...]` and row 2 is `[7, 9, 0, ...]`, so the mean of the two
    /// before normalising is `[5, 2, 0, ...]` — small enough to be exact in
    /// `f32`, which is the arithmetic the change rests on.
    fn table() -> Vec<i8> {
        let mut table = vec![0i8; 3 * DIMENSIONS];
        table[DIMENSIONS] = 3;
        table[DIMENSIONS + 1] = -5;
        table[2 * DIMENSIONS] = 7;
        table[2 * DIMENSIONS + 1] = 9;
        table
    }

    fn weights_of(dtype: Dtype, shape: Vec<usize>, bytes: &[u8]) -> Vec<u8> {
        let view = safetensors::tensor::TensorView::new(dtype, shape, bytes).unwrap();
        safetensors::serialize(
            [(EMBEDDINGS, view)],
            &None::<std::collections::HashMap<String, String>>,
        )
        .unwrap()
    }

    fn int8_weights(table: &[i8]) -> Vec<u8> {
        let bytes: Vec<u8> = table.iter().map(|value| *value as u8).collect();
        weights_of(
            Dtype::I8,
            vec![table.len() / DIMENSIONS, DIMENSIONS],
            &bytes,
        )
    }

    fn built(table: &[i8]) -> Result<SemanticModel, String> {
        SemanticModel::from_parts(
            b"{\"normalize\": true}",
            &int8_weights(table),
            TOKENIZER.as_bytes(),
        )
    }

    /// The arithmetic the whole change rests on: the mean of the int8 rows the
    /// ids name, normalised, is the mean of the same rows expanded to `f32`.
    #[test]
    fn pooling_over_the_stored_bytes_is_the_mean_of_the_expanded_rows() {
        let model = built(&table()).unwrap();
        let vectors = model
            .encode_with_args(&["hello world".to_owned()], Some(MAX_TOKENS), 1024)
            .unwrap();
        assert_eq!(vectors[0].len(), DIMENSIONS);
        let norm = (5.0f32 * 5.0 + 2.0 * 2.0).sqrt();
        assert_eq!(vectors[0][0], 5.0 / norm);
        assert_eq!(vectors[0][1], 2.0 / norm);
        assert_eq!(vectors[0][2], 0.0);
    }

    /// The unknown token is dropped before the mean, so a text that tokenizes to
    /// a known word plus an unknown one pools exactly as the known word alone.
    #[test]
    fn an_unknown_token_is_dropped_before_the_mean() {
        let model = built(&table()).unwrap();
        let with_unknown = model
            .encode_with_args(&["hello zzz".to_owned()], Some(MAX_TOKENS), 1024)
            .unwrap();
        let alone = model
            .encode_with_args(&["hello".to_owned()], Some(MAX_TOKENS), 1024)
            .unwrap();
        assert_eq!(with_unknown[0], alone[0]);
    }

    /// The cut before tokenizing is by characters, not bytes, so it never lands
    /// inside a multi-byte character.
    #[test]
    fn the_cut_is_by_characters_and_not_bytes() {
        assert_eq!(truncate("abcdef", 2, 2), "abcd");
        assert_eq!(truncate("aéb", 1, 2), "aé");
        assert_eq!(truncate("abc", 10, 10), "abc");
    }

    /// Every way a file can be wrong is refused rather than loaded, so a swapped
    /// or truncated table cannot reach the stage.
    #[test]
    fn a_table_the_build_does_not_accept_is_refused() {
        let config = b"{\"normalize\": true}";
        assert!(built(&table()).is_ok());

        // A dtype that is not int8: the same shape, four bytes a value.
        let wide = vec![0u8; 3 * DIMENSIONS * 4];
        let not_int8 = weights_of(Dtype::F32, vec![3, DIMENSIONS], &wide);
        assert!(SemanticModel::from_parts(config, &not_int8, TOKENIZER.as_bytes()).is_err());

        // A table narrower than the vectors the stage scores against.
        let narrow = weights_of(Dtype::I8, vec![3, 4], &[0u8; 12]);
        assert!(SemanticModel::from_parts(config, &narrow, TOKENIZER.as_bytes()).is_err());

        // No safetensors at all, and a tokenizer that is not one.
        assert!(
            SemanticModel::from_parts(config, b"not safetensors", TOKENIZER.as_bytes()).is_err()
        );
        assert!(
            SemanticModel::from_parts(config, &int8_weights(&table()), b"not a tokenizer").is_err()
        );
    }
}
