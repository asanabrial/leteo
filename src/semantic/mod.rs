//! The model behind the semantic search stage: where it is, whether it is the
//! one this build accepts, and how a text becomes a vector.
//!
//! A static embedding model: one vector per WordPiece token, averaged. There is
//! no network at search time, no server and no inference runtime -- encoding a
//! question is a tokenizer and a mean. The model is three files that live beside
//! the binary and not inside it, because a model compiled into the binary ties
//! the size of the crate, and so whether it can be published at all, to the size
//! of the model. Every install loads it from a file and behaves the same at run
//! time; only how the file arrives differs, and `install` is the one path for the
//! installs that arrive without it.
//!
//! Which model, why, and where the bytes come from are in
//! `tools/semantic/README.md`. What this
//! module owns is what belongs to the model rather than to the search: where it
//! is looked for, the hashes it must have, its name, its width, how much of a
//! memory it reads, and the cosine below which two texts are not saying the same
//! thing.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use sha2::{Digest, Sha256};

pub mod install;
mod model;

pub use model::SemanticModel;

/// What computed a stored vector, and the model this binary accepts.
///
/// Written beside every vector, with [`WINDOWING`], and compared on every read:
/// vectors from two models are not comparable, so a build that accepts a
/// different model finds every stored row stale and embeds it again. The suffix is the start of the
/// weights' SHA-256, so the name cannot stay the same when the bytes change;
/// a test holds it to `tools/semantic/checksums.json`.
pub const MODEL_ID: &str = "static-similarity-mrl-multilingual-v1/256-int8-pruned@25d1b8a5";

/// The three files and the SHA-256 each must have, compiled in.
///
/// A file that is missing or does not hash to this is not loaded: the stage is
/// off and search is lexical only. These are the same hashes as
/// `tools/semantic/checksums.json`, which `tools/semantic/build_model.py` writes
/// and a test compares to this list, so the pipeline and the binary cannot drift.
/// They are of the files as stored -- `tokenizer.json.gz` is the gzip.
pub const MODEL_FILES: [(&str, &str); 3] = [
    (
        "config.json",
        "8500f7ac6a4c66d3cb249f579cc6272acb06161dd8fa47cd0cbe917740c990cb",
    ),
    (
        "model.safetensors",
        "25d1b8a51d496419f4f0cda355e0ac1dc4d671e6f4ef7330b2f90e62da104694",
    ),
    (
        "tokenizer.json.gz",
        "91a107a2a2d936609852477fa11110f19bb0765ce5c76d4c51c385809d67df3c",
    ),
];

/// The environment variable that names a model directory, tried first.
pub const MODEL_DIR_ENV: &str = "LETEO_MODEL_DIR";

/// The width of a vector, and of the row the table must hold.
pub const DIMENSIONS: usize = 256;

/// How many tokens one vector is the mean of.
///
/// 128, the title's and then the body's. Measured on this model over a copy of
/// a real store: 128 beat 64 and 256, and at 512 the semantic MRR of bodies fell
/// from .57 to .35. A mean over more text is a mean that says less, which is why
/// a long memory is read as several windows of this size ([`windows`]) rather
/// than as one longer mean.
pub const MAX_TOKENS: usize = 128;

/// How many of the title's tokens head every window.
///
/// The title says what the memory is about, and a window from the middle of a
/// body without it is a paragraph about nothing in particular. 48 leaves at
/// least 80 tokens of body in each window; a title longer than that is cut, so
/// a memory whose title is all it has still has its body read.
pub const TITLE_TOKENS: usize = 48;

/// The most windows one memory is read as, and so the most rows it keeps.
///
/// 32 windows hold between 2,560 body tokens (under a title of 48) and 4,096
/// (under none), between four and seven times the median body of 548
/// tokens on the measured store, where a memory took 6.6 windows on average and
/// 98% of bodies were longer than one window. A body is capped at 50,000
/// characters on the way in, which is more than 32 windows hold, so what lies
/// past the last window of the longest bodies is not read by meaning; it is
/// still found by its words. The cap bounds a memory at 32 rows and 32 KB of
/// vectors, and the scan's worst case at 32 rows a memory.
pub const MAX_WINDOWS: usize = 32;

/// How a memory is cut into the texts its vectors are the means of, stored
/// beside every row and compared with [`MODEL_ID`] on every read.
///
/// Its own name and not a suffix on the model's, because the model's name is
/// the weights' (a test holds it to `tools/semantic/checksums.json`) and this is
/// not: changing either makes every stored row stale. Bump the trailing version
/// whenever [`windows`] would cut a memory differently.
pub const WINDOWING: &str = "windows-128-title-48-v1";

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
///
/// Those figures are of one vector a memory. The floor now meets the best of a
/// memory's windows, which is higher -- on a copy of a real store the best
/// cosine of a memory that was not the answer rose from a median .35 to .39,
/// and on a synthetic store of random-word bodies the stage answered questions
/// the single vector never cleared 0.30 for. Measured again through this binary
/// with `tools/semantic/hardset/evaluate.py`, the controls it answers with the
/// stage moved from 127 of 160 to 130 (absent questions) and from 74 of 117 to
/// 74 (the other project's questions), which is inside the noise of 160, and
/// the ratchet's no-answer questions stayed at 21 of 24 with none unhedged. So
/// it stays at 0.30. 0.32 brought the controls to 126 and 70 and cost the hard
/// set .085 to .078 of reciprocal rank and 103 more unanswered questions.
pub const FLOOR: f32 = 0.30;

/// How many semantic candidates may be merged into a `nearest` answer.
///
/// The lexical stage was answering anyway, so the meaning is added beside it
/// rather than instead of it: the page already holds the memories one shared
/// word reached, and everything the cosine brings after that is padding. On a
/// question the store cannot answer, that padding is what turns one shared word
/// into a full page of unrelated memories. The best semantic match is kept -- it
/// is the one the stage has an opinion about -- and the tail is dropped.
/// Measured against `tools/memory-bench`'s no-answer questions.
pub const MERGE_CAP: usize = 5;

/// Where the repository keeps the model, relative to its root.
///
/// What the default download address assumes -- the files committed here are the
/// ones fetched from a tag -- and what the tests that need the real model read it
/// from, so the layout and the address cannot drift apart.
pub const REPOSITORY_MODEL_DIR: &str = "assets/model";

/// The directory `LETEO_MODEL_DIR` names, if it names one.
///
/// The one place the variable is read: the store's configuration and `setup` both
/// ask this, so what a process takes from its environment is decided once. Under
/// `cfg(test)` it answers nothing, so the unit tests do not depend on what the
/// developer who runs them has exported; they hand the directory they mean in.
pub fn explicit_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    std::env::var_os(MODEL_DIR_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Where the model is looked for, in order: the one list, and the only one.
///
/// 1. `explicit`, when the caller was given a path (`LETEO_MODEL_DIR`, read by
///    [`explicit_dir`] and nowhere else).
/// 2. `model/` beside the executable -- an archive unpacked, or a build that put
///    it there.
/// 3. `../share/leteo/model/` from the executable -- a prefix install, a package
///    manager's share directory, the Docker images.
/// 4. `model/` in the data directory -- where `leteo model install` puts it.
///
/// The executable is taken with its symlinks resolved first, and then as it was
/// found, in that order of directories, because package managers link a binary
/// into a shared `bin/` from a versioned directory that holds the rest
/// (Homebrew's Cellar): beside the link is not beside the file, and the other way
/// round. The real file's directory is asked first because that is where whatever
/// installed it put what belongs to it.
/// Nothing in the list asks which channel installed the binary.
pub fn locations(data_dir: &Path, explicit: Option<&Path>) -> Vec<PathBuf> {
    locations_for(std::env::current_exe().ok().as_deref(), data_dir, explicit)
}

/// [`locations`] for a given executable, so the list can be tested without being
/// the executable.
pub fn locations_for(exe: Option<&Path>, data_dir: &Path, explicit: Option<&Path>) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    let mut add = |path: PathBuf| {
        if !found.contains(&path) {
            found.push(path);
        }
    };
    if let Some(path) = explicit.filter(|path| !path.as_os_str().is_empty()) {
        add(path.to_path_buf());
    }
    if let Some(exe) = exe {
        let resolved = exe.canonicalize().ok();
        let directories = [resolved.as_deref(), Some(exe)]
            .into_iter()
            .flatten()
            .filter_map(Path::parent)
            .map(Path::to_path_buf)
            .collect::<Vec<_>>();
        for directory in &directories {
            add(directory.join("model"));
        }
        for directory in &directories {
            add(directory
                .join("..")
                .join("share")
                .join("leteo")
                .join("model"));
        }
    }
    add(data_dir.join("model"));
    found
}

/// What one directory holds, judged against [`MODEL_FILES`].
enum Directory {
    /// None of the three files is there: nothing was ever put in it.
    Empty,
    /// Something is there and it is not the model: which file, and how.
    Wrong(Vec<String>),
    /// All three, hashing to what they must, as bytes in the order of the list.
    Verified([Vec<u8>; 3]),
}

/// The one SHA-256 the pins are compared with, for loading a model and for
/// deciding whether removal may take a file; two copies is how one of them would
/// come to accept what the other refuses.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn inspect(directory: &Path) -> Directory {
    let mut problems = Vec::new();
    let mut files: Vec<Vec<u8>> = Vec::new();
    let mut any = false;
    for (name, expected) in MODEL_FILES {
        match std::fs::read(directory.join(name)) {
            Ok(bytes) => {
                any = true;
                if sha256_hex(&bytes) != expected {
                    problems.push(format!("{name} does not match the file this build accepts"));
                }
                files.push(bytes);
            }
            // A path that is not there, and a path that has a file where its
            // directory should be (`NotADirectory`: `LETEO_MODEL_DIR` pointing at a
            // file, say), are the same fact about this location -- nothing is
            // installed here -- and not a model that is wrong.
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                problems.push(format!("{name} is missing"));
            }
            Err(error) => {
                any = true;
                problems.push(format!("{name} could not be read: {error}"));
            }
        }
    }
    if !any {
        return Directory::Empty;
    }
    match <[Vec<u8>; 3]>::try_from(files) {
        Ok(files) if problems.is_empty() => Directory::Verified(files),
        _ => Directory::Wrong(problems),
    }
}

/// Where the model stands, for the people who have to be told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Found, every file hashing to what this build accepts, and here.
    Verified(PathBuf),
    /// Nowhere: these are the places that were looked in.
    Missing(Vec<PathBuf>),
    /// Somewhere, and not the model this build accepts: here, and what is wrong.
    Mismatch {
        directory: PathBuf,
        problems: Vec<String>,
    },
}

impl Status {
    /// The sentence `doctor` and `setup` give, ending in what to do.
    pub fn explain(&self) -> String {
        match self {
            Self::Verified(directory) => {
                format!(
                    "the semantic model is installed and verified at {}",
                    directory.display()
                )
            }
            Self::Missing(searched) => format!(
                "the semantic model is not installed, so search is by words only; looked in {}; install it with `leteo model install`, or `leteo model install --from <directory>` from a copy",
                searched
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Mismatch {
                directory,
                problems,
            } => format!(
                "the semantic model at {} is not the one this build accepts ({}), so search is by words only; replace it with `leteo model install`, or `leteo model install --from <directory>` from a copy",
                directory.display(),
                problems.join("; ")
            ),
        }
    }
}

/// The first location holding a verified model, else the first that holds
/// something wrong, else every place that was looked.
pub fn status(data_dir: &Path, explicit: Option<&Path>) -> Status {
    let places = locations(data_dir, explicit);
    let mut wrong = None;
    for place in &places {
        match inspect(place) {
            Directory::Verified(_) => return Status::Verified(place.clone()),
            Directory::Wrong(problems) => {
                wrong.get_or_insert(Status::Mismatch {
                    directory: place.clone(),
                    problems,
                });
            }
            Directory::Empty => {}
        }
    }
    wrong.unwrap_or(Status::Missing(places))
}

/// Why a vector could not be made.
#[derive(Debug)]
pub struct EmbedError(String);

impl std::fmt::Display for EmbedError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for EmbedError {}

/// The stage cannot run because there is no model it may use. Not a failure of
/// a healthy install: `doctor` says which condition holds.
#[derive(Debug)]
pub struct Unavailable(pub Status);

impl std::fmt::Display for Unavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0.explain())
    }
}

impl std::error::Error for Unavailable {}

/// What a loaded model was loaded from, so a change to the files is noticed.
type Stamp = Vec<(u64, Option<SystemTime>)>;

/// The models loaded in this process, one per directory they were found in.
///
/// One per directory rather than one per process, because two stores on one
/// machine can name different model directories. `load` writes it and
/// `is_loaded` reads it, so the backfill can ask whether the model is up without
/// being the reason it comes up.
type Loaded = HashMap<PathBuf, (Stamp, Arc<SemanticModel>)>;
static LOADED: Mutex<Option<Loaded>> = Mutex::new(None);

fn stamp(directory: &Path) -> Stamp {
    MODEL_FILES
        .iter()
        .map(|(name, _)| {
            std::fs::metadata(directory.join(name))
                .map(|meta| (meta.len(), meta.modified().ok()))
                .unwrap_or((0, None))
        })
        .collect()
}

/// The model, verified and loaded once per process and directory.
///
/// Every file is read, hashed and compared before a byte of it is used, and the
/// bytes that were hashed are the bytes that are loaded, so a file swapped in
/// between cannot be loaded unverified. Loading then costs 13 to 15 ms and, with
/// the int8 table kept as the bytes it is stored as, the memory that costs.
///
/// A process that never reaches the semantic stage never pays either: that is
/// every hook, which writes its memory without a vector and leaves the vector
/// to the backfill (`StoreConfig::embed_on_write`), and almost every search,
/// which answers from the words before this is asked for.
///
/// Re-checked when the files change (size or modification time), and not on every
/// call; the lookup on the way is a few `stat`s.
pub fn load(data_dir: &Path, explicit: Option<&Path>) -> Result<Arc<SemanticModel>, Unavailable> {
    let places = locations(data_dir, explicit);
    let mut cache = LOADED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let cache = cache.get_or_insert_with(HashMap::new);
    let mut wrong = None;
    for place in &places {
        let stamped = stamp(place);
        if let Some((seen, model)) = cache.get(place)
            && *seen == stamped
        {
            return Ok(Arc::clone(model));
        }
        match inspect(place) {
            Directory::Verified([config, weights, tokenizer]) => {
                let built = build(&config, &weights, &tokenizer);
                // The three files are 13 MB and none of them is the model: the
                // table is copied out as the int8 it already is and the buffers
                // go before the model is cached. Freeing them here is the second
                // half of what this reader saves -- the first is not expanding
                // the table to f32 -- and keeps the two from coexisting past the
                // build that needs both.
                drop((config, weights, tokenizer));
                match built {
                    Ok(model) => {
                        let model = Arc::new(model);
                        cache.insert(place.clone(), (stamped, Arc::clone(&model)));
                        return Ok(model);
                    }
                    Err(problem) => {
                        wrong.get_or_insert(Status::Mismatch {
                            directory: place.clone(),
                            problems: vec![problem],
                        });
                    }
                }
            }
            Directory::Wrong(problems) => {
                wrong.get_or_insert(Status::Mismatch {
                    directory: place.clone(),
                    problems,
                });
            }
            Directory::Empty => {}
        }
    }
    Err(Unavailable(wrong.unwrap_or(Status::Missing(places))))
}

/// Whether a verified model is already loaded for this store, without loading
/// one.
///
/// The background backfill asks this so it is never the reason the model comes
/// up: it keeps nothing until a search or a write has already paid for the load.
/// The places are the same ordered list `load` searches, so an answer here and
/// an answer there cannot disagree about which directory the model is in.
pub fn is_loaded(data_dir: &Path, explicit: Option<&Path>) -> bool {
    let places = locations(data_dir, explicit);
    let cache = LOADED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(cache) = cache.as_ref() else {
        return false;
    };
    places.iter().any(|place| cache.contains_key(place))
}

fn build(config: &[u8], weights: &[u8], tokenizer_gz: &[u8]) -> Result<SemanticModel, String> {
    let mut tokenizer = Vec::new();
    flate2::read::GzDecoder::new(tokenizer_gz)
        .read_to_end(&mut tokenizer)
        .map_err(|error| format!("tokenizer.json.gz did not decompress: {error}"))?;
    SemanticModel::from_parts(config, weights, &tokenizer)
}

/// The token windows a memory is read as: each one the title's first
/// [`TITLE_TOKENS`] ids and then the next run of body ids, so that every window
/// holds at most [`MAX_TOKENS`], with no overlap and at most [`MAX_WINDOWS`] of
/// them. A body with no token is one window of the title alone.
///
/// No overlap, because the windows were measured that way: on 140 blind
/// questions about a fact in the second half of a body, asked of a copy of a
/// real store, top-10 went from 50.7% to 62.1% in the memory's language and from
/// 3.6% to 25.0% in another one, while first place on natural questions moved
/// from 72.9% to between 69.3% and 72.1%.
///
/// The first window is what the single vector before it read, with two
/// differences. A title of more than 48 tokens is cut to 48 here, where the
/// single vector read all of it up to 128. And the single vector cut the joined
/// text to `128 * median token length` characters before tokenizing, so a text
/// whose first characters held fewer than 128 tokens was read short and its
/// last word could be read as the fragment the cut left; here the title and
/// the body are cut and tokenized separately, against the budget of all the
/// windows. Tokenizing them apart is otherwise the same as tokenizing them
/// joined by the newline the single vector used, because the pre-tokenizer
/// splits on whitespace before anything else.
pub fn windows(title: &[u32], body: &[u32]) -> Vec<Vec<u32>> {
    let head = &title[..title.len().min(TITLE_TOKENS)];
    let run = MAX_TOKENS - head.len();
    if body.is_empty() {
        return vec![head.to_vec()];
    }
    body.chunks(run)
        .take(MAX_WINDOWS)
        .map(|part| [head, part].concat())
        .collect()
}

/// L2-normalised vectors for a memory, one per window of [`windows`], in order.
///
/// A window with no token the model knows has an empty vector, as [`embed`]
/// gives a text with none. The title is cut to the characters of
/// [`TITLE_TOKENS`] tokens and the body to those of every window's worth, the
/// same character cut the single vector took and for the same reason: a body
/// may hold 50,000 characters, and what no window reads need not be tokenized.
pub fn embed_document(
    model: &SemanticModel,
    title: &str,
    content: &str,
) -> Result<Vec<Vec<f32>>, EmbedError> {
    let pooled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let title = model.token_ids(&[title], Some(TITLE_TOKENS))?.remove(0);
        let body = model
            .token_ids(&[content], Some(MAX_WINDOWS * MAX_TOKENS))?
            .remove(0);
        Ok::<_, EmbedError>(
            windows(&title, &body)
                .iter()
                .map(|ids| model.pool(ids))
                .collect::<Vec<_>>(),
        )
    }))
    .map_err(|_| EmbedError("the tokenizer failed on this text".to_owned()))??;
    Ok(pooled.into_iter().map(normalised).collect())
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
pub fn embed(model: &SemanticModel, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
    let vectors = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        model.encode_with_args(texts, Some(MAX_TOKENS), 1024)
    }))
    .map_err(|_| EmbedError("the tokenizer failed on this text".to_owned()))??;
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
/// stored bytes without building a `Vec`: the scan runs over every window of
/// every memory in scope -- what that costs is measured beside
/// `semantic_candidates` -- and an allocation per row would be most of it.
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
pub(crate) mod tests {
    use super::*;

    /// The model directory of this checkout, where the tests that need the real
    /// model find it. The packaged crate does not carry `assets/model/`, so a
    /// test that needs it says so and returns, in a tree that has none.
    pub(crate) fn repository_model() -> Option<PathBuf> {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join(REPOSITORY_MODEL_DIR);
        directory
            .join("model.safetensors")
            .is_file()
            .then_some(directory)
    }

    /// `let Some(model) = needs_model!() else { return };`
    macro_rules! needs_model {
        () => {{
            let found = $crate::semantic::tests::repository_model();
            if found.is_none() {
                eprintln!("skipped: this tree has no assets/model (the packaged crate ships none)");
            }
            found
        }};
    }
    pub(crate) use needs_model;

    pub(crate) fn texts(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    fn loaded() -> Option<Arc<SemanticModel>> {
        let directory = needs_model!()?;
        let scratch = std::env::temp_dir().join("leteo-no-such-data-dir");
        Some(
            load(&scratch, Some(&directory))
                .expect("the repository's model is the one this build accepts"),
        )
    }

    /// The hashes compiled in are the hashes the pipeline wrote down, and the
    /// model's name carries the start of the weights' hash. Two lists that have
    /// to agree are one list and a test.
    #[test]
    fn the_pinned_hashes_are_the_ones_the_pipeline_records() {
        let recorded: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/semantic/checksums.json"),
            )
            .unwrap(),
        )
        .unwrap();
        for (name, hash) in MODEL_FILES {
            assert_eq!(recorded["files"][name].as_str(), Some(hash), "{name}");
        }
        assert_eq!(
            recorded["files"].as_object().unwrap().len(),
            MODEL_FILES.len()
        );
        let weights = MODEL_FILES[1].1;
        assert!(
            MODEL_ID.ends_with(&format!("@{}", &weights[..8])),
            "{MODEL_ID} does not carry the start of {weights}"
        );
        assert_eq!(recorded["model_id"].as_str(), Some(MODEL_ID));
    }

    /// And the files in the repository are those hashes: what a release packs.
    #[test]
    fn the_files_in_the_repository_are_the_model_this_build_accepts() {
        let Some(directory) = needs_model!() else {
            return;
        };
        assert_eq!(
            status(Path::new("/nonexistent"), Some(&directory)),
            Status::Verified(directory)
        );
    }

    #[test]
    fn the_model_loads_and_has_the_published_width() {
        let Some(model) = loaded() else { return };
        let vectors = embed(&model, &texts(&["database connection pool exhaustion"])).unwrap();
        assert_eq!(vectors.len(), 1);
        assert_eq!(vectors[0].len(), DIMENSIONS);
        let norm: f32 = vectors[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "not unit length: {norm}");
    }

    /// The reader this crate now pools with and the one it replaced give the
    /// same vector for the same text -- equal, not close.
    ///
    /// The equality is the point: the arithmetic is exact (`model::pool`), so
    /// there is no tolerance to set and no cosine under which a real difference
    /// could hide. The sample covers the shapes that exercise what was
    /// reproduced from the crate: a short text, a multi-line one, one in another
    /// language, one with no known token, one with characters outside ASCII, and
    /// one long enough to be cut before it is tokenized.
    #[test]
    fn the_int8_reader_agrees_with_the_crate_it_replaced() {
        let Some(directory) = needs_model!() else {
            return;
        };
        let config = std::fs::read(directory.join("config.json")).unwrap();
        let weights = std::fs::read(directory.join("model.safetensors")).unwrap();
        let compressed = std::fs::read(directory.join("tokenizer.json.gz")).unwrap();
        let mut tokenizer = Vec::new();
        flate2::read::GzDecoder::new(&compressed[..])
            .read_to_end(&mut tokenizer)
            .unwrap();

        let ours = SemanticModel::from_parts(&config, &weights, &tokenizer).unwrap();
        let reference =
            model2vec_rs::model::StaticModel::from_bytes(&tokenizer, &weights, &config, Some(true))
                .unwrap();

        let mut sample = texts(&[
            "database connection pool exhaustion",
            "Rotate JWT signing keys every 30 days with kid header\nSigning keys live in \
             KMS and tokens carry a kid header.",
            "thundering herd when the cache expires",
            "rotación de las claves de firma",
            "How to bake sourdough bread\nMix flour and water, let the starter ferment \
             overnight, bake in a hot oven.",
            "\u{200b}\u{200b}",
            "Ünïcödé, punctuation — and an em dash.",
        ]);
        sample.push("word ".repeat(2_000));

        let ours = ours
            .encode_with_args(&sample, Some(MAX_TOKENS), 1024)
            .unwrap();
        let reference = reference.encode_with_args(&sample, Some(MAX_TOKENS), 1024);
        assert_eq!(ours.len(), reference.len());
        for (index, (left, right)) in ours.iter().zip(&reference).enumerate() {
            assert_eq!(
                left.len(),
                right.len(),
                "text {index} has a different width"
            );
            // `f32: PartialEq` calls `-0.0` and `0.0` equal; the claim is bit
            // equality, so the bits are what is compared.
            let (left, right): (Vec<u32>, Vec<u32>) = (
                left.iter().map(|value| value.to_bits()).collect(),
                right.iter().map(|value| value.to_bits()).collect(),
            );
            assert_eq!(left, right, "text {index} is not bit-identical");
        }
    }

    /// Meaning rather than words: a question that shares no word with its
    /// target is closer to it than to an unrelated memory, and so is the same
    /// question in another language. This is the property the whole stage rests
    /// on, and it is asserted with a margin rather than an order, because the
    /// floor the stage applies is a number and not a ranking.
    #[test]
    fn a_question_in_other_words_is_nearer_its_target_than_an_unrelated_memory() {
        let Some(model) = loaded() else { return };
        let vectors = embed(
            &model,
            &texts(&[
                "Product catalog cache stampede on cold start\nWhen the catalog cache expired, \
                 hundreds of requests recomputed it at once.",
                "Rotate JWT signing keys every 30 days with kid header\nSigning keys live in KMS \
                 and tokens carry a kid header.",
                "How to bake sourdough bread\nMix flour and water, let the starter ferment \
                 overnight, bake in a hot oven.",
                "thundering herd when the cache expires",
                "rotación de las claves de firma",
            ]),
        )
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
        let Some(model) = loaded() else { return };
        let vectors = embed(&model, &texts(&["\u{200b}\u{200b}"])).unwrap();
        assert!(vectors[0].is_empty());
    }

    /// On hand-built ids, so it runs in a tree with no model: the store test of
    /// the same behaviour needs `assets/model` and returns early without it.
    #[test]
    fn a_memory_is_cut_into_title_headed_windows_without_overlap() {
        let title: Vec<u32> = (1000..1010).collect();
        let body: Vec<u32> = (0..300).collect();
        let cut = windows(&title, &body);
        let run = MAX_TOKENS - title.len();
        assert_eq!(cut.len(), body.len().div_ceil(run), "{cut:?}");
        for window in &cut {
            assert!(window.len() <= MAX_TOKENS, "{}", window.len());
            assert_eq!(&window[..title.len()], &title[..]);
        }
        let read: Vec<u32> = cut
            .iter()
            .flat_map(|window| window[title.len()..].to_vec())
            .collect();
        assert_eq!(read, body, "every body id once, in order");

        let long_title: Vec<u32> = (2000..2100).collect();
        let cut = windows(&long_title, &body);
        assert!(
            cut.iter()
                .all(|window| window[..TITLE_TOKENS] == long_title[..TITLE_TOKENS])
        );
        assert_eq!(cut[0].len(), MAX_TOKENS, "the title is cut, the body read");

        assert_eq!(windows(&title, &[]), vec![title.clone()]);
        assert_eq!(windows(&[], &[]), vec![Vec::<u32>::new()]);

        let huge: Vec<u32> = (0..100_000).collect();
        assert_eq!(windows(&title, &huge).len(), MAX_WINDOWS);
    }

    #[test]
    fn a_stored_row_of_the_wrong_width_scores_nothing() {
        let question = vec![0.0f32; DIMENSIONS];
        assert_eq!(cosine(&[0u8; 12], &question), None);
        assert_eq!(cosine(&vec![0u8; DIMENSIONS * 4], &question), Some(0.0));
    }

    /// The list is one list, in the order the maintainer set, with the explicit
    /// path first and the data directory last, symlinks resolved, no duplicates.
    #[test]
    fn the_places_the_model_is_looked_for_are_one_ordered_list() {
        let scratch = tempfile::TempDir::new().unwrap();
        let cellar = scratch.path().join("Cellar/leteo/1.0/bin");
        std::fs::create_dir_all(&cellar).unwrap();
        std::fs::write(cellar.join("leteo"), b"").unwrap();
        let linked = scratch.path().join("bin");
        std::fs::create_dir_all(&linked).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(cellar.join("leteo"), linked.join("leteo")).unwrap();
        #[cfg(not(unix))]
        std::fs::write(linked.join("leteo"), b"").unwrap();

        let data = scratch.path().join("data");
        let explicit = scratch.path().join("explicit");
        let list = locations_for(Some(&linked.join("leteo")), &data, Some(&explicit));
        // The directory the executable resolves into, taken the way `locations_for`
        // takes it. On Unix that is the Cellar behind the link; where the test
        // cannot make a symlink the "link" is a plain file and resolves into its own
        // directory, which Windows also spells with a `\\?\` prefix.
        let real = linked
            .join("leteo")
            .canonicalize()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let expected_first = explicit.clone();
        assert_eq!(list.first(), Some(&expected_first));
        assert_eq!(list.last(), Some(&data.join("model")));
        let beside = list
            .iter()
            .position(|p| *p == real.join("model"))
            .expect("beside the real file");
        let share = list
            .iter()
            .position(|p| *p == real.join("..").join("share").join("leteo").join("model"))
            .expect("the share directory of the real file's prefix");
        assert!(
            beside < share,
            "beside the executable comes before ../share: {list:?}"
        );
        #[cfg(unix)]
        {
            let beside_link = list
                .iter()
                .position(|p| *p == linked.join("model"))
                .expect("and beside the link, which is where a shared bin/ would hold it");
            assert!(
                beside < beside_link,
                "the real file's directory is asked before the link's: {list:?}"
            );
        }
        let mut unique = list.clone();
        unique.dedup();
        assert_eq!(unique, list);
        assert_eq!(
            locations_for(None, &data, None),
            vec![data.join("model")],
            "no executable and no path is the data directory alone"
        );
    }

    /// A path with a file where its directory should be holds nothing, which is
    /// not the same as holding something wrong: `LETEO_MODEL_DIR` aimed at a file
    /// must not be reported as a model that does not verify.
    #[test]
    fn a_path_that_is_a_file_is_empty_and_not_wrong() {
        let scratch = tempfile::TempDir::new().unwrap();
        let file = scratch.path().join("not-a-directory");
        std::fs::write(&file, b"x").unwrap();
        let outcome = status(scratch.path(), Some(&file));
        assert!(matches!(outcome, Status::Missing(_)), "{outcome:?}");
        assert!(matches!(inspect(&file), Directory::Empty));
    }

    #[test]
    fn a_directory_is_missing_wrong_or_verified_and_each_says_so() {
        let scratch = tempfile::TempDir::new().unwrap();
        let data = scratch.path();
        assert!(
            matches!(status(data, None), Status::Missing(_)),
            "{:?}",
            status(data, None)
        );
        assert!(status(data, None).explain().contains("leteo model install"));

        std::fs::create_dir_all(data.join("model")).unwrap();
        std::fs::write(data.join("model/config.json"), b"{}").unwrap();
        let Status::Mismatch {
            directory,
            problems,
        } = status(data, None)
        else {
            panic!("a file that is not the model is a mismatch");
        };
        assert_eq!(directory, data.join("model"));
        assert!(
            problems
                .iter()
                .any(|p| p.contains("config.json does not match")),
            "{problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("model.safetensors is missing")),
            "{problems:?}"
        );
        assert!(
            load(data, None).is_err(),
            "an unverified model is never loaded"
        );
    }
}
