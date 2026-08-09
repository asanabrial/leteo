//! How well a real store answers questions about what it holds.
//!
//! Read-only, against a database you name. Nothing here writes, and nothing
//! here ships in the binary.
//!
//! Every question is built from a memory the store already has, so the memory
//! that should come back is known without anybody labelling anything. That is
//! the whole trick, and it is also the trap: see the README before believing a
//! number.
//!
//! ```text
//! cargo run --manifest-path tools/retrieval/Cargo.toml -- ~/.leteo/leteo.db
//! cargo run --manifest-path tools/retrieval/Cargo.toml -- copy.db \
//!     --weights "20.0, 0.3, 0.0, 0.0, 0.0, 6.0"
//! ```

use rand::SeedableRng;
use rand::seq::IndexedRandom;
use rusqlite::Connection;

/// The word split the binary uses: anything that is not alphanumeric or an
/// underscore ends a word, and words shorter than this are dropped. Kept the
/// same on purpose — a harness that tokenises differently measures a search
/// nobody runs.
const MIN_WORD: usize = 3;
/// How far into a body a question starts. See [`words`].
const BODY_SKIP: usize = 25;
/// How deep the answer is looked at.
const DEPTH: usize = 10;

/// `count` distinct words of `text`, the way the store splits, after `skip`.
///
/// The split is the one `normalize::prompt_terms` performs, and
/// `the_split_is_the_one_the_binary_uses` below holds the two together rather
/// than trusting this comment. The harness this replaces described its own
/// character class as "the word split the binary uses" and it was not: it
/// treated Greek, Cyrillic and the Spanish ordinals as separators where the
/// binary treats them as letters. Measured over 4,055 real memories, that alone
/// moved first-place recall on titles from 78.3% to 80.1% — the trap its own
/// comment warns about, in its own code.
///
/// `skip` is what keeps a body question from being a title question wearing
/// another hat. Memories are written lead-sentence first, and that sentence
/// usually restates the title — so words taken from the top of a body ask the
/// same thing the title does, and any weight vector that favours titles wins on
/// both sets at once.
fn words(text: &str, count: usize, skip: usize) -> Vec<String> {
    let mut seen = Vec::new();
    for word in text.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
        if word.chars().count() < MIN_WORD {
            continue;
        }
        let word = word.to_lowercase();
        if !seen.contains(&word) {
            seen.push(word);
        }
    }
    if seen.len() <= skip {
        return Vec::new();
    }
    seen.drain(..skip);
    seen.truncate(count);
    seen
}

/// Where `wanted` lands for `query`, or `None` if it is not in the first
/// `DEPTH`.
///
/// Through `leteo`'s own statement, with the weights as its one parameter. The
/// tool that wrote its own copy of this query measured, for an afternoon, a
/// statement the product does not issue.
fn rank_of(
    connection: &Connection,
    weights: &str,
    query: &[String],
    wanted: i64,
) -> rusqlite::Result<Option<usize>> {
    let matched = query
        .iter()
        .map(|word| format!("\"{word}\""))
        .collect::<Vec<_>>()
        .join(" ");
    let sql = leteo::measure::matching_observations_sql(leteo::measure::FTS_STEMMED, weights);
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(
        rusqlite::params![
            matched,
            None::<String>,
            None::<String>,
            None::<String>,
            DEPTH as i64
        ],
        |row| row.get::<_, i64>(0),
    )?;
    let found: Vec<i64> = rows.collect::<Result<_, _>>()?;
    Ok(found.iter().position(|id| *id == wanted))
}

struct Measure {
    asked: usize,
    top1: f64,
    top3: f64,
    top10: f64,
    mrr: f64,
}

fn measure(
    connection: &Connection,
    weights: &str,
    sample: &[(i64, String, String)],
    titles: bool,
    count: usize,
    skip: usize,
) -> rusqlite::Result<Measure> {
    let mut places = Vec::new();
    for (id, title, content) in sample {
        let query = words(if titles { title } else { content }, count, skip);
        if query.is_empty() {
            continue;
        }
        places.push(rank_of(connection, weights, &query, *id)?);
    }
    let asked = places.len().max(1);
    let at = |n: usize| {
        100.0 * places.iter().flatten().filter(|place| **place < n).count() as f64 / asked as f64
    };
    // Mean reciprocal rank over every question, a miss counting zero — one
    // number that moves with both whether the memory comes back and how high.
    let mrr = places
        .iter()
        .flatten()
        .map(|place| 1.0 / (*place as f64 + 1.0))
        .sum::<f64>()
        / asked as f64;
    Ok(Measure {
        asked: places.len(),
        top1: at(1),
        top3: at(3),
        top10: at(DEPTH),
        mrr,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let Some(database) = arguments.next() else {
        eprintln!("usage: leteo-retrieval <leteo.db> [--sample N] [--seed N] [--weights \"…\"]…");
        return Ok(());
    };
    let mut sample_size = 300_usize;
    let mut seed = 7_u64;
    let mut extra = Vec::new();
    while let Some(flag) = arguments.next() {
        let value = arguments.next().unwrap_or_default();
        match flag.as_str() {
            "--sample" => sample_size = value.parse()?,
            "--seed" => seed = value.parse()?,
            "--weights" => extra.push(value),
            other => return Err(format!("unknown option {other}").into()),
        }
    }

    let connection = Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;
    let mut statement = connection.prepare(
        "SELECT id, title, content FROM observations
          WHERE deleted_at IS NULL AND length(title) > 25 AND length(content) > 80",
    )?;
    let rows: Vec<(i64, String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<Result<_, _>>()?;
    if rows.len() < sample_size {
        eprintln!("only {} memories are long enough to ask about", rows.len());
    }
    let take = sample_size.min(rows.len());
    let mut first = rand::rngs::StdRng::seed_from_u64(seed);
    let sample: Vec<_> = rows
        .choose_multiple(&mut first, take)
        .cloned()
        .collect::<Vec<_>>();
    // A second draw with a different seed. A weight vector that wins on the set
    // it was fitted to and not on this one has fitted the sample.
    let mut second = rand::rngs::StdRng::seed_from_u64(seed + 1000);
    let held_out: Vec<_> = rows.choose_multiple(&mut second, take).cloned().collect();

    let shipped = leteo::measure::BM25_WEIGHTS.to_owned();
    for weights in std::iter::once(&shipped).chain(extra.iter()) {
        let label = if *weights == shipped {
            "shipped".to_owned()
        } else {
            weights.clone()
        };
        println!();
        println!("bm25({label})");
        for (name, titles, count, skip, questions) in [
            ("titles", true, 6, 0, &sample),
            ("titles, held out", true, 6, 0, &held_out),
            // Past the lead sentence, which restates the title.
            ("bodies", false, 12, BODY_SKIP, &sample),
            ("bodies, held out", false, 12, BODY_SKIP, &held_out),
        ] {
            let result = measure(&connection, weights, questions, titles, count, skip)?;
            println!(
                "  {name:<18} n={:<4} top1={:5.1}%  top3={:5.1}%  top10={:5.1}%  mrr={:.4}",
                result.asked, result.top1, result.top3, result.top10, result.mrr
            );
        }
    }
    if !extra.is_empty() {
        println!();
        println!("Read the two rows for bodies before believing a win on titles:");
        println!("questions drawn from titles reward weighting titles by construction.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// This splits words the way the binary does.
    ///
    /// Not a comment saying so: `prompt_terms` is the product's own splitter,
    /// and up to its own cap the two have to agree exactly. A harness that
    /// tokenises differently measures a search nobody runs.
    #[test]
    fn the_split_is_the_one_the_binary_uses() {
        for text in [
            "Un indice que perdio sus disparadores, y nadie dijo nada",
            "SEARCH plan: idx_obs_project_order walks every memory",
            "mem_save con scope personnal y type implementation",
            "1a 2o rue nino ANO ano_ ano-ano C++ C# a bb ccc dddd",
            "αβγ данные memoria",
        ] {
            let ours = super::words(text, 32, 0);
            let theirs = leteo::measure::prompt_terms(text);
            assert_eq!(ours, theirs, "for {text:?}");
        }
    }
}
