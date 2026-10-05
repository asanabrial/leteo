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
//! `tools/semantic/README.md` and `openspec/specs/search.md` §15. What this
//! module owns is what belongs to the model rather than to the search: where it
//! is looked for, the hashes it must have, its name, its width, how much of a
//! memory it reads, and the cosine below which two texts are not saying the same
//! thing.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use model2vec_rs::model::StaticModel;
use sha2::{Digest, Sha256};

pub mod install;

/// What computed a stored vector, and the model this binary accepts.
///
/// Written beside every vector and compared on every read: vectors from two
/// models are not comparable, so a build that accepts a different model finds
/// every stored row stale and embeds it again. The suffix is the start of the
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

/// Where the model is looked for, in order: the one list, and the only one.
///
/// 1. `explicit`, when the caller was given a path (`LETEO_MODEL_DIR`).
/// 2. `model/` beside the executable -- a release archive unpacked.
/// 3. `../share/leteo/model/` from the executable -- a prefix install, a package
///    manager's share directory, the Docker images.
/// 4. `model/` in the data directory -- where `leteo model install` puts it.
///
/// The executable is taken both as it was found and with symlinks resolved, in
/// that order of directories, because package managers link a binary into a
/// shared `bin/` from a versioned directory that holds the rest (Homebrew's
/// Cellar): beside the link is not beside the file, and the other way round.
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

fn sha256_hex(bytes: &[u8]) -> String {
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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
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
/// the int8 table expanded to f32, the memory `search.md` §15 states once. A
/// process that never reaches the semantic stage never pays either: that is every
/// hook and almost every search.
///
/// Re-checked when the files change (size or modification time), and not on every
/// call; the lookup on the way is a few `stat`s.
pub fn load(data_dir: &Path, explicit: Option<&Path>) -> Result<Arc<StaticModel>, Unavailable> {
    type Loaded = HashMap<PathBuf, (Stamp, Arc<StaticModel>)>;
    static LOADED: Mutex<Option<Loaded>> = Mutex::new(None);
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
                match build(&config, &weights, &tokenizer) {
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

fn build(config: &[u8], weights: &[u8], tokenizer_gz: &[u8]) -> Result<StaticModel, String> {
    let mut tokenizer = Vec::new();
    flate2::read::GzDecoder::new(tokenizer_gz)
        .read_to_end(&mut tokenizer)
        .map_err(|error| format!("tokenizer.json.gz did not decompress: {error}"))?;
    StaticModel::from_bytes(&tokenizer, weights, config, Some(true))
        .map_err(|error| format!("the model did not load: {error}"))
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
pub fn embed(model: &StaticModel, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
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
pub(crate) mod tests {
    use super::*;

    /// The model directory of this checkout, where the tests that need the real
    /// model find it. The packaged crate does not carry `assets/model/`, so a
    /// test that needs it says so and returns, in a tree that has none.
    pub(crate) fn repository_model() -> Option<PathBuf> {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("model");
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

    fn loaded() -> Option<Arc<StaticModel>> {
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
        let real = cellar.canonicalize().unwrap();
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
        assert!(
            list.iter().any(|p| *p == linked.join("model")),
            "and beside the link, which is where a shared bin/ would hold it: {list:?}"
        );
        let mut unique = list.clone();
        unique.dedup();
        assert_eq!(unique, list);
        assert_eq!(
            locations_for(None, &data, None),
            vec![data.join("model")],
            "no executable and no path is the data directory alone"
        );
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
