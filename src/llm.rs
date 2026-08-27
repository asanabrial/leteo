//! Semantic conflict judging through an agent CLI.
//!
//! Lexical scanning (`conflicts scan`) finds pairs that *look* related. Deciding
//! whether they actually conflict needs a language model, so Leteo shells out to
//! an agent CLI the developer already has installed and stores the verdict like
//! any other judgment.
//!
//! The prompt is deliberately frozen: changing it changes the meaning of every
//! verdict already persisted and makes cross-model comparison meaningless.

use std::{future::Future, time::Duration};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    Store,
    memory::model::{CandidateOptions, JudgeBySemanticParams},
    memory::normalize,
    store::{
        RELATION_COMPATIBLE, RELATION_CONFLICTS_WITH, RELATION_NOT_CONFLICT, RELATION_RELATED,
        RELATION_SCOPED, RELATION_SUPERSEDES,
    },
};

/// Relation verbs a runner may return.
///
/// The constants rather than the words, because this list has to agree with
/// [`crate::memory::rules::is_relation_verb`] — the one the store checks a
/// judgment against — and nothing but the shared names holds them together. A
/// verb here and not there is a model call paid for and thrown away when the
/// store refuses to store the verdict; a verb there and not here is one the
/// model is never told exists. The test below holds the two sets equal.
const RELATION_VOCABULARY: &[&str] = &[
    RELATION_CONFLICTS_WITH,
    RELATION_SUPERSEDES,
    RELATION_SCOPED,
    RELATION_RELATED,
    RELATION_COMPATIBLE,
    RELATION_NOT_CONFLICT,
];

/// The comparison prompt, frozen so verdicts stay comparable over time.
///
/// The relation verbs and the response shape are fixed by the wire format:
/// these judgements are stored as relations and replicate to peers that may be
/// running Engram, so both sides have to agree on the vocabulary. The wording
/// around them is Leteo's own.
///
/// Changing the wording changes the judgements, so it is worth being
/// deliberate: a verdict recorded under one prompt is not strictly comparable
/// to one recorded under another.
const PROMPT_TEMPLATE: &str = "\
Two notes were written at different moments by an engineer working on the same
codebase. Decide how the second one stands in relation to the first.

--- FIRST NOTE (A) ---
id: {a_id}
title: {a_title}
body: {a_content}

--- SECOND NOTE (B) ---
id: {b_id}
title: {b_title}
body: {b_content}

Pick the single verb that fits best. Use exactly one of these, spelled exactly
as written:

  conflicts_with  B asserts something that cannot be true if A is true.
  supersedes      B is the later word on the same point: A is now out of date.
  scoped          One covers a special case of what the other covers generally.
  related         Same subject, but neither corrects nor narrows the other.
  compatible      Both hold at once and add to each other.
  not_conflict    Different subjects; pairing them tells nobody anything.

Guidance:
- Prefer not_conflict when the only thing the notes share is vocabulary.
- Reserve conflicts_with for a real contradiction, not a difference in emphasis.
- supersedes needs one to be the newer answer to the same question, not merely
  the more recent note.
- Confidence is how sure you are of the verb, not how important the notes are.

Reply with one line of JSON, nothing before or after it:
{\"Relation\":\"<verb>\",\"Confidence\":<0.0-1.0>,\"Reasoning\":\"<200 chars or fewer>\"}";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub relation: String,
    pub confidence: f64,
    #[serde(default)]
    pub reasoning: String,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runner {
    Claude,
    OpenCode,
}

impl Runner {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_lowercase().as_str() {
            "claude" => Ok(Self::Claude),
            "opencode" => Ok(Self::OpenCode),
            other => bail!("unknown agent CLI {other:?}; use claude or opencode"),
        }
    }

    pub fn program(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::OpenCode => "opencode",
        }
    }

    fn arguments(self) -> Vec<&'static str> {
        match self {
            Self::Claude => vec![
                "-p",
                "--output-format",
                "json",
                "--model",
                "haiku",
                "--max-turns",
                "1",
            ],
            Self::OpenCode => vec!["run", "--format", "json", "--pure"],
        }
    }

    fn parse_output(self, output: &[u8]) -> Result<Verdict> {
        match self {
            Self::Claude => parse_claude_envelope(output),
            Self::OpenCode => parse_opencode_stream(output),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SemanticOptions {
    pub project: String,
    /// Upper bound on CLI invocations, because each one costs tokens.
    pub max_pairs: usize,
    pub concurrency: usize,
    pub timeout: Duration,
}

impl Default for SemanticOptions {
    fn default() -> Self {
        Self {
            project: String::new(),
            max_pairs: 100,
            concurrency: 5,
            timeout: Duration::from_secs(60),
        }
    }
}

impl SemanticOptions {
    fn validate(&self) -> Result<()> {
        if !(1..=20).contains(&self.concurrency) {
            bail!("concurrency must be between 1 and 20");
        }
        if self.timeout.is_zero() || self.timeout > Duration::from_secs(600) {
            bail!("the per-call timeout must be between 1 and 600 seconds");
        }
        if self.max_pairs == 0 {
            bail!("max-pairs must be at least 1");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SemanticSummary {
    pub project: String,
    pub inspected: usize,
    pub pairs: usize,
    pub judged: usize,
    pub skipped: usize,
    pub already_judged: usize,
    pub errors: usize,
    pub capped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub source_id: String,
    pub target_id: String,
    pub prompt: String,
}

/// Collects the pairs a semantic scan would judge, without calling any model.
/// Separated from the judging so the expensive part is easy to preview.
pub fn collect_pairs(
    store: &mut Store,
    options: &SemanticOptions,
) -> Result<(Vec<Pair>, usize, usize)> {
    options.validate()?;
    let project = normalize::project(&options.project);
    // Every memory in the project, not a context budget's worth.
    //
    // Asking for `None` here meant `max_context_results` — twenty — so a scan
    // of a project holding two thousand memories inspected twenty of them and
    // reported `inspected: 20` as though that were the project. The cost of
    // this scan is already bounded, and deliberately: `max_pairs` exists
    // because each judgement is a model call, and the loop below returns the
    // moment that budget is full. A second, accidental cap on the corpus only
    // made the budget unreachable.
    let observations = store.recent_observations(Some(&project), Some(i64::MAX as usize), false)?;
    let mut pairs = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut already_judged = 0_usize;
    let mut inspected = 0;

    for observation in &observations {
        inspected += 1;
        let candidates = store.find_candidates(
            observation.id,
            CandidateOptions {
                project: Some(project.clone()),
                scope: Some(observation.scope.clone()),
                limit: Some(10),
                skip_insert: true,
                ..CandidateOptions::default()
            },
        )?;
        for candidate in candidates {
            // The relation is symmetric, so judge each unordered pair once.
            let key = if observation.sync_id <= candidate.sync_id {
                (observation.sync_id.clone(), candidate.sync_id.clone())
            } else {
                (candidate.sync_id.clone(), observation.sync_id.clone())
            };
            if !seen.insert(key) {
                continue;
            }
            // A pair somebody has already ruled on is not a question.
            //
            // `find_candidates` hides settled pairs when it is filing one and
            // shows them to a preview, which is what `skip_insert` above asks
            // for — so this is the caller's to ask, and `scan_project` asks it
            // and reports the answer as `already_related`. This did not. Each
            // one is a model call spent on a question the store has already
            // answered, and then `judge_by_semantic` writes the model's answer
            // over the one on record: a `supersedes` an agent had put to the
            // user, quietly downgraded to `related`, takes the caveat off every
            // surface that was carrying it.
            //
            // Two of the first hundred pairs on a real store, which is small
            // and is not the point: that store holds 255 judged pairs and the
            // scan walks the newest memories first, so the share grows with
            // every scan somebody runs.
            if store
                .pair_is_judged(&observation.sync_id, &candidate.sync_id)
                .unwrap_or(false)
            {
                already_judged += 1;
                continue;
            }
            let Ok(target) = store.get_observation(candidate.id) else {
                continue;
            };
            pairs.push(Pair {
                source_id: observation.sync_id.clone(),
                target_id: candidate.sync_id.clone(),
                prompt: build_prompt(
                    &observation.sync_id,
                    &observation.title,
                    &observation.content,
                    &candidate.sync_id,
                    &target.title,
                    &target.content,
                ),
            });
            if pairs.len() >= options.max_pairs {
                return Ok((pairs, inspected, already_judged));
            }
        }
    }
    Ok((pairs, inspected, already_judged))
}

/// Judges candidate pairs and persists every verdict that is not
/// `not_conflict`.
///
/// `compare` is injected so the scan can be driven by a real CLI in production
/// and by a deterministic stub in tests.
pub async fn semantic_scan<F, Fut>(
    store: &mut Store,
    options: &SemanticOptions,
    compare: F,
) -> Result<SemanticSummary>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<Verdict>>,
{
    let (pairs, inspected, already_judged) = collect_pairs(store, options)?;
    let mut summary = SemanticSummary {
        project: normalize::project(&options.project),
        inspected,
        pairs: pairs.len(),
        already_judged,
        capped: pairs.len() >= options.max_pairs,
        ..SemanticSummary::default()
    };

    // Verdicts are persisted as they arrive so a failure halfway through still
    // keeps the work already paid for.
    for chunk in pairs.chunks(options.concurrency.max(1)) {
        let verdicts =
            futures_util::future::join_all(chunk.iter().map(|pair| compare(pair.prompt.clone())))
                .await;
        for (pair, verdict) in chunk.iter().zip(verdicts) {
            match verdict {
                Ok(verdict) if verdict.relation == RELATION_NOT_CONFLICT => summary.skipped += 1,
                Ok(verdict) => {
                    let persisted = store.judge_by_semantic(JudgeBySemanticParams {
                        source_id: pair.source_id.clone(),
                        target_id: pair.target_id.clone(),
                        relation: verdict.relation,
                        confidence: Some(verdict.confidence),
                        reasoning: Some(verdict.reasoning),
                        model: verdict.model,
                    });
                    match persisted {
                        Ok(_) => summary.judged += 1,
                        Err(error) => {
                            tracing::warn!(%error, "semantic verdict could not be stored");
                            summary.errors += 1;
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "semantic comparison failed");
                    summary.errors += 1;
                }
            }
        }
    }
    Ok(summary)
}

/// Runs one comparison through an agent CLI, enforcing the per-call timeout.
pub async fn cli_compare(runner: Runner, timeout: Duration, prompt: String) -> Result<Verdict> {
    let mut command = tokio::process::Command::new(runner.program());
    command
        .args(runner.arguments())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .with_context(|| format!("start the {:?} CLI", runner.program()))?;
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;

        stdin.write_all(prompt.as_bytes()).await.ok();
        stdin.shutdown().await.ok();
    }
    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .with_context(|| format!("the {} CLI timed out", runner.program()))?
        .with_context(|| format!("the {} CLI failed", runner.program()))?;
    if !output.status.success() {
        bail!(
            "the {} CLI exited with {}: {}",
            runner.program(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    runner.parse_output(&output.stdout)
}

pub fn build_prompt(
    a_id: &str,
    a_title: &str,
    a_content: &str,
    b_id: &str,
    b_title: &str,
    b_content: &str,
) -> String {
    PROMPT_TEMPLATE
        .replace("{a_id}", a_id)
        .replace("{a_title}", a_title)
        .replace("{a_content}", a_content)
        .replace("{b_id}", b_id)
        .replace("{b_title}", b_title)
        .replace("{b_content}", b_content)
}

/// Parses `claude --output-format json`, whose result field holds the model's
/// own JSON, sometimes wrapped in a Markdown fence.
fn parse_claude_envelope(output: &[u8]) -> Result<Verdict> {
    #[derive(Deserialize)]
    struct Envelope {
        #[serde(default)]
        result: String,
        #[serde(default)]
        #[serde(rename = "modelUsage")]
        model_usage: std::collections::BTreeMap<String, serde_json::Value>,
    }

    let envelope: Envelope =
        serde_json::from_slice(output).context("the Claude CLI returned invalid JSON")?;
    let mut verdict = parse_inner_verdict(&envelope.result)?;
    if verdict.model.is_none() {
        verdict.model = envelope.model_usage.keys().next().cloned();
    }
    Ok(verdict)
}

/// Parses `opencode run --format json`, a newline-delimited event stream where
/// the last text event carries the answer.
fn parse_opencode_stream(output: &[u8]) -> Result<Verdict> {
    #[derive(Deserialize)]
    struct Event {
        #[serde(default)]
        r#type: String,
        #[serde(default)]
        part: Option<Part>,
        #[serde(default)]
        metadata: Option<serde_json::Value>,
    }

    #[derive(Deserialize)]
    struct Part {
        #[serde(default)]
        text: String,
    }

    let text = String::from_utf8_lossy(output);
    let mut answer = None;
    let mut model = None;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(event) = serde_json::from_str::<Event>(line) else {
            continue;
        };
        if let Some(metadata) = event.metadata.as_ref().and_then(|value| value.get("model"))
            && let Some(value) = metadata.as_str()
        {
            model = Some(value.to_owned());
        }
        if event.r#type == "text"
            && let Some(part) = event.part
            && !part.text.trim().is_empty()
        {
            answer = Some(part.text);
        }
    }
    let answer = answer.context("the OpenCode CLI returned no text event")?;
    let mut verdict = parse_inner_verdict(&answer)?;
    if verdict.model.is_none() {
        verdict.model = model;
    }
    Ok(verdict)
}

/// Extracts the model's verdict object, tolerating Markdown fences and
/// surrounding prose, and rejecting verbs outside the locked vocabulary.
fn parse_inner_verdict(raw: &str) -> Result<Verdict> {
    #[derive(Deserialize)]
    struct Inner {
        #[serde(alias = "Relation")]
        relation: String,
        #[serde(alias = "Confidence", default)]
        confidence: f64,
        #[serde(alias = "Reasoning", default)]
        reasoning: String,
        #[serde(alias = "Model", default)]
        model: Option<String>,
    }

    let body = strip_code_fence(raw.trim());
    let json = extract_json_object(body).context("no JSON object in the model response")?;
    let inner: Inner =
        serde_json::from_str(json).context("the model response is not a verdict object")?;
    let relation = inner.relation.trim().to_lowercase();
    if !RELATION_VOCABULARY.contains(&relation.as_str()) {
        bail!("the model returned an unknown relation {relation:?}");
    }
    Ok(Verdict {
        relation,
        confidence: inner.confidence.clamp(0.0, 1.0),
        reasoning: inner.reasoning.trim().to_owned(),
        model: inner
            .model
            .map(|model| model.trim().to_owned())
            .filter(|model| !model.is_empty()),
    })
}

fn strip_code_fence(value: &str) -> &str {
    let Some(rest) = value.strip_prefix("```") else {
        return value;
    };
    let rest = rest.split_once('\n').map_or(rest, |(_, rest)| rest);
    rest.trim_end().strip_suffix("```").unwrap_or(rest).trim()
}

/// Returns the first balanced `{...}` span, so a model that adds a sentence
/// before its JSON still parses.
fn extract_json_object(value: &str) -> Option<&str> {
    let start = value.find('{')?;
    let mut depth = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, character) in value[start..].char_indices() {
        if in_string {
            match character {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&value[start..=start + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::{
        memory::model::{AddObservation, ListRelationsOptions},
        store::StoreConfig,
    };

    fn store_with_pair() -> (TempDir, Store) {
        let temp = TempDir::new().unwrap();
        let mut store = Store::open(StoreConfig::new(temp.path().join("llm.db"))).unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        // Filler, and it is load-bearing. A pair is only a candidate if it
        // scores past the floor `find_candidates` applies, and a bm25 term
        // weight grows with how rare the word is across the store — so on a
        // two-memory fixture even an exact repeat scores near zero and reaches
        // nothing. The scan under test then finds no pairs for a reason that
        // has nothing to do with the scan.
        for index in 0..40 {
            store
                .add_observation(AddObservation {
                    session_id: "s1".to_owned(),
                    kind: "discovery".to_owned(),
                    title: format!("Unrelated note {index} on deployment windows"),
                    content: format!("Body {index}: staged rollout, canaries and a rollback plan."),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap();
        }
        for title in ["Retry backoff policy", "Retry backoff policy revisited"] {
            store
                .add_observation(AddObservation {
                    session_id: "s1".to_owned(),
                    kind: "decision".to_owned(),
                    title: title.to_owned(),
                    content: "the retry backoff doubles every attempt".to_owned(),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap();
        }
        (temp, store)
    }

    #[test]
    fn the_prompt_carries_both_observations_and_the_locked_vocabulary() {
        let prompt = build_prompt("obs-a", "Title A", "Body A", "obs-b", "Title B", "Body B");

        // Every field is substituted, checked by value rather than by the
        // label around it, so rewording the prompt does not fail this.
        for value in ["obs-a", "Title A", "Body A", "obs-b", "Title B", "Body B"] {
            assert!(prompt.contains(value), "{value} is missing from the prompt");
        }
        assert!(
            !prompt.contains('{') || !prompt.contains("_id}"),
            "a placeholder was left unfilled: {prompt}"
        );
        // The vocabulary is fixed by the wire format, not by taste: these
        // verdicts replicate to peers that have to understand them.
        for relation in RELATION_VOCABULARY {
            assert!(prompt.contains(relation), "{relation}");
        }
    }

    #[test]
    fn claude_envelopes_are_parsed_with_and_without_fences() {
        let plain = br#"{"type":"result","result":"{\"Relation\":\"related\",\"Confidence\":0.82,\"Reasoning\":\"same topic\"}","modelUsage":{"claude-haiku-4-5":{}}}"#;
        let verdict = parse_claude_envelope(plain).unwrap();
        assert_eq!(verdict.relation, "related");
        assert_eq!(verdict.confidence, 0.82);
        assert_eq!(verdict.reasoning, "same topic");
        assert_eq!(verdict.model.as_deref(), Some("claude-haiku-4-5"));

        let fenced =
            br#"{"result":"```json\n{\"Relation\":\"supersedes\",\"Confidence\":1.5}\n```"}"#;
        let verdict = parse_claude_envelope(fenced).unwrap();
        assert_eq!(verdict.relation, "supersedes");
        assert_eq!(verdict.confidence, 1.0, "confidence is clamped");
        assert!(verdict.model.is_none());

        let chatty = br#"{"result":"Here is my answer: {\"Relation\":\"scoped\",\"Confidence\":0.4} - hope it helps"}"#;
        assert_eq!(parse_claude_envelope(chatty).unwrap().relation, "scoped");

        assert!(parse_claude_envelope(b"not json").is_err());
        let unknown = br#"{"result":"{\"Relation\":\"maybe\",\"Confidence\":0.9}"}"#;
        assert!(
            parse_claude_envelope(unknown)
                .unwrap_err()
                .to_string()
                .contains("unknown relation")
        );
    }

    #[test]
    fn opencode_streams_use_the_last_text_event() {
        let stream = br#"{"type":"start"}
{"type":"step_finish","metadata":{"model":"anthropic/claude-haiku-4-5"}}
{"type":"text","part":{"text":"thinking out loud"}}
{"type":"text","part":{"text":"{\"Relation\":\"compatible\",\"Confidence\":0.6,\"Reasoning\":\"both hold\"}"}}
"#;
        let verdict = parse_opencode_stream(stream).unwrap();

        assert_eq!(verdict.relation, "compatible");
        assert_eq!(verdict.reasoning, "both hold");
        assert_eq!(verdict.model.as_deref(), Some("anthropic/claude-haiku-4-5"));

        assert!(
            parse_opencode_stream(b"{\"type\":\"start\"}\n")
                .unwrap_err()
                .to_string()
                .contains("no text event")
        );
    }

    #[test]
    fn runners_map_to_their_command_lines() {
        assert_eq!(Runner::parse("Claude").unwrap(), Runner::Claude);
        assert_eq!(Runner::parse(" opencode ").unwrap(), Runner::OpenCode);
        assert!(Runner::parse("gpt").is_err());
        assert_eq!(Runner::Claude.program(), "claude");
        assert!(Runner::Claude.arguments().contains(&"--output-format"));
        assert!(Runner::OpenCode.arguments().contains(&"--pure"));
    }

    #[test]
    fn option_limits_are_validated() {
        let valid = SemanticOptions::default();
        valid.validate().unwrap();
        assert!(
            SemanticOptions {
                concurrency: 0,
                ..valid.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            SemanticOptions {
                concurrency: 21,
                ..valid.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            SemanticOptions {
                timeout: Duration::from_secs(0),
                ..valid.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            SemanticOptions {
                max_pairs: 0,
                ..valid
            }
            .validate()
            .is_err()
        );
    }

    /// A pair somebody has already ruled on is not sent to a model again.
    ///
    /// `find_candidates` hides settled pairs when it is filing one and shows
    /// them to a preview, which is what `skip_insert` asks for — so the caller
    /// has to ask. `scan_project` asks and reports the answer as
    /// `already_related`; this scan asked nothing, and each unasked pair is a
    /// paid model call whose verdict is then written over the one on record. A
    /// `supersedes` an agent had already settled, downgraded to `related`,
    /// takes the caveat off every surface carrying it — which is what
    /// `mem_search`, `mem_context`, `mem_get_observation`, `mem_timeline`,
    /// `mem_review` and the session-start block all attach to a superseded
    /// memory.
    #[test]
    fn a_pair_already_judged_is_not_sent_to_a_model_again() {
        let (_temp, mut store) = store_with_pair();
        let options = SemanticOptions {
            project: "leteo".to_owned(),
            ..SemanticOptions::default()
        };

        let (pairs, _, already) = collect_pairs(&mut store, &options).unwrap();
        assert_eq!(pairs.len(), 1, "hay un par que juzgar");
        assert_eq!(already, 0, "y todavía nadie lo ha juzgado");

        // Proposing is not judging. A `mem_save` files one *pending* row per
        // candidate it proposes — seventy-four in a real store — and those are
        // exactly the ones worth asking about. Without this the query would look
        // at the row and not at its state, and nothing would notice.
        let propuestos = store
            .find_candidates(
                store
                    .connection()
                    .query_row(
                        "SELECT id FROM observations WHERE sync_id = ?1",
                        [&pairs[0].source_id],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                crate::memory::model::CandidateOptions {
                    project: Some("leteo".to_owned()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(!propuestos.is_empty(), "se archivó una propuesta pendiente");
        let (todavía, _, already) = collect_pairs(&mut store, &options).unwrap();
        assert_eq!(
            todavía.len(),
            1,
            "propuesto y sin veredicto sigue siendo una pregunta: {todavía:?}"
        );
        assert_eq!(already, 0);

        store
            .judge_by_semantic(crate::memory::model::JudgeBySemanticParams {
                source_id: pairs[0].source_id.clone(),
                target_id: pairs[0].target_id.clone(),
                relation: "supersedes".to_owned(),
                confidence: Some(0.9),
                reasoning: Some("La segunda sustituye a la primera.".to_owned()),
                model: None,
            })
            .unwrap();

        let (después, _, already) = collect_pairs(&mut store, &options).unwrap();
        assert!(
            después.is_empty(),
            "ya está juzgado, no se vuelve a preguntar: {después:?}"
        );
        assert_eq!(already, 1, "y se dice cuántos se dejaron en paz");

        assert!(
            store
                .pair_is_judged(&pairs[0].source_id, &pairs[0].target_id)
                .unwrap()
        );
    }

    #[test]
    fn pairs_are_collected_once_per_unordered_pair_and_respect_the_cap() {
        let (_temp, mut store) = store_with_pair();
        let options = SemanticOptions {
            project: "leteo".to_owned(),
            ..SemanticOptions::default()
        };

        let (pairs, inspected, _) = collect_pairs(&mut store, &options).unwrap();

        // Everything in the project, filler included — what is asserted below
        // is that the one real pair is collected once, not how big the fixture
        // had to be for the floor to bite.
        assert_eq!(inspected, 42);
        assert_eq!(pairs.len(), 1, "the symmetric pair is judged once");
        assert!(pairs[0].prompt.contains("Retry backoff policy"));

        let (capped, _, _) = collect_pairs(
            &mut store,
            &SemanticOptions {
                max_pairs: 1,
                ..options
            },
        )
        .unwrap();
        assert_eq!(capped.len(), 1);
    }

    #[tokio::test]
    async fn semantic_scanning_persists_verdicts_and_counts_outcomes() {
        let (_temp, mut store) = store_with_pair();
        let options = SemanticOptions {
            project: "leteo".to_owned(),
            ..SemanticOptions::default()
        };

        let summary = semantic_scan(&mut store, &options, |_| async {
            Ok(Verdict {
                relation: "supersedes".to_owned(),
                confidence: 0.9,
                reasoning: "the revised note replaces the original".to_owned(),
                model: Some("test-model".to_owned()),
            })
        })
        .await
        .unwrap();

        assert_eq!(summary.pairs, 1);
        assert_eq!(summary.judged, 1);
        assert_eq!(summary.skipped, 0);
        assert_eq!(summary.errors, 0);
        let relations = store
            .list_relations(ListRelationsOptions::default())
            .unwrap();
        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0].relation, "supersedes");
        assert_eq!(relations[0].judgment_status, "judged");
    }

    /// A verdict the store refuses is counted, not swallowed.
    ///
    /// The scan is the one path where verdicts arrive from a model rather than
    /// a person, so it is the one that will propose a verb nobody documents —
    /// `overrides`, `supersede` without the `s`. The store refuses those, and
    /// if the scan did not count the refusal the summary would report "judged:
    /// 0, errors: 0" for a run that judged nothing: success, on a scan that
    /// achieved nothing at all.
    ///
    /// A comparison that never returns a verdict is already covered beside
    /// this. What was not is a verdict that returns and cannot be stored.
    #[tokio::test]
    async fn a_verdict_the_store_refuses_is_counted_rather_than_dropped() {
        let (_temp, mut store) = store_with_pair();
        let options = SemanticOptions {
            project: "leteo".to_owned(),
            ..SemanticOptions::default()
        };

        let summary = semantic_scan(&mut store, &options, |_| async {
            Ok(Verdict {
                relation: "overrides".to_owned(),
                confidence: 0.9,
                reasoning: "a verb the graph cannot read".to_owned(),
                model: None,
            })
        })
        .await
        .unwrap();

        assert_eq!(summary.judged, 0);
        assert_eq!(
            summary.errors, 1,
            "a refused verdict has to appear somewhere: {summary:?}"
        );
        assert!(
            store
                .list_relations(ListRelationsOptions::default())
                .unwrap()
                .iter()
                .all(|relation| relation.judgment_status != "judged"),
            "nothing was judged, and nothing should say it was"
        );
    }

    /// An impossible concurrency is refused before any work starts.
    ///
    /// `pairs.chunks(0)` panics in Rust, and the loop clamps with `.max(1)` —
    /// but that clamp is never reached, because `SemanticOptions::validate`
    /// rejects the value first with a sentence that says what the range is.
    /// Two mechanisms, one property, and only the outer one is load-bearing:
    /// a mutation removing the clamp survives this test, and the reason is
    /// redundancy rather than a hole.
    ///
    /// What is asserted is therefore the outcome somebody actually gets —
    /// an error naming the range, not a panic — so it holds whichever of the
    /// two is doing the work.
    #[tokio::test]
    async fn an_impossible_concurrency_is_refused_instead_of_panicking() {
        let (_temp, mut store) = store_with_pair();
        for concurrency in [0, 21] {
            let options = SemanticOptions {
                project: "leteo".to_owned(),
                concurrency,
                ..SemanticOptions::default()
            };
            let refused = semantic_scan(&mut store, &options, |_| async {
                panic!("no comparison should be attempted")
            })
            .await
            .expect_err("an impossible concurrency has to be refused");
            assert!(
                refused.to_string().contains("concurrency"),
                "the refusal has to name what is wrong: {refused}"
            );
        }
    }

    #[tokio::test]
    async fn unrelated_verdicts_are_skipped_and_failures_are_counted() {
        let (_temp, mut store) = store_with_pair();
        let options = SemanticOptions {
            project: "leteo".to_owned(),
            ..SemanticOptions::default()
        };

        let skipped = semantic_scan(&mut store, &options, |_| async {
            Ok(Verdict {
                relation: RELATION_NOT_CONFLICT.to_owned(),
                confidence: 1.0,
                reasoning: "unrelated".to_owned(),
                model: None,
            })
        })
        .await
        .unwrap();
        assert_eq!(skipped.skipped, 1);
        assert_eq!(skipped.judged, 0);
        assert!(
            store
                .list_relations(ListRelationsOptions::default())
                .unwrap()
                .is_empty(),
            "not_conflict is a successful no-op"
        );

        let failed = semantic_scan(&mut store, &options, |_| async {
            anyhow::bail!("the CLI is not installed")
        })
        .await
        .unwrap();
        assert_eq!(failed.errors, 1);
        assert_eq!(failed.judged, 0);
    }

    #[test]
    fn a_scan_inspects_the_whole_project_and_not_a_context_budget() {
        // The cost of this scan is bounded on purpose: `max_pairs` exists
        // because each judgement is a model call. Asking `recent_observations`
        // for `None` added a second, accidental cap of twenty on the corpus,
        // which made that budget unreachable and reported `inspected: 20` for a
        // project of any size.
        let temp = TempDir::new().unwrap();
        let mut store = Store::open(StoreConfig::new(temp.path().join("scan.db"))).unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        let budget = StoreConfig::new("unused").max_context_results;
        let wanted = budget * 3;
        for index in 0..wanted {
            store
                .add_observation(AddObservation {
                    session_id: "s1".to_owned(),
                    kind: "decision".to_owned(),
                    title: format!("Retry backoff policy {index}"),
                    content: "the retry backoff doubles every attempt".to_owned(),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap();
        }

        let (_pairs, inspected, _) = collect_pairs(
            &mut store,
            &SemanticOptions {
                project: "leteo".to_owned(),
                // Larger than any pair this corpus can produce, so nothing but
                // the corpus cap could stop the walk early.
                max_pairs: usize::MAX,
                ..SemanticOptions::default()
            },
        )
        .unwrap();

        assert_eq!(
            inspected, wanted,
            "every memory in the project is inspected"
        );
        assert!(inspected > budget);
    }

    #[test]
    fn the_verbs_offered_to_a_model_are_the_verbs_the_store_will_accept() {
        // Two lists of the same six words, in two modules, with nothing but
        // the shared constants between them. A verb offered here and refused
        // there is a model call paid for and thrown away; a verb the store
        // takes but the prompt never mentions is one the scan cannot produce.
        for verb in RELATION_VOCABULARY {
            assert!(
                crate::memory::rules::is_relation_verb(verb),
                "{verb} is offered to the model and the store would refuse it"
            );
        }
        for verb in [
            RELATION_RELATED,
            RELATION_COMPATIBLE,
            RELATION_SCOPED,
            RELATION_CONFLICTS_WITH,
            RELATION_SUPERSEDES,
            RELATION_NOT_CONFLICT,
        ] {
            assert!(
                RELATION_VOCABULARY.contains(&verb),
                "the store accepts {verb} and the model is never told it exists"
            );
        }
    }

    #[test]
    fn the_frozen_prompt_spells_every_verb_it_asks_for() {
        // The prompt is deliberately frozen, so a verb added to the vocabulary
        // and not to the wording would be accepted from a model that was never
        // offered it.
        let prompt = build_prompt("a", "A", "body", "b", "B", "body");
        for verb in RELATION_VOCABULARY {
            assert!(prompt.contains(verb), "the prompt never mentions {verb}");
        }
    }
}
