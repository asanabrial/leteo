//! Reading and editing JSON that may carry comments and trailing commas.
//!
//! `opencode.jsonc` and VS Code's `mcp.json` are read by their clients with
//! comments allowed, so a person may keep notes in a file Leteo writes to.
//! Reading strips the comments and hands the result to `serde_json`. Writing
//! never re-serialises the file: it splices the one member that changed into
//! the original text, so everything else — comments, trailing commas,
//! indentation, key order — is still there afterwards, byte for byte. That is
//! more work than rendering a `Value` and is the whole point: the rewrite that
//! used to be here deleted things nobody had asked it to touch.

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;

/// The same text with comments and trailing commas taken out.
pub(super) fn strip(content: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(content.len());
    let mut index = 0;

    while index < content.len() {
        if content[index] == b'"' {
            output.push(content[index]);
            index += 1;
            while index < content.len() {
                output.push(content[index]);
                if content[index] == b'\\' && index + 1 < content.len() {
                    index += 1;
                    output.push(content[index]);
                } else if content[index] == b'"' {
                    index += 1;
                    break;
                }
                index += 1;
            }
            continue;
        }
        if content.get(index..index + 2) == Some(b"//") {
            index += 2;
            while index < content.len() && content[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if content.get(index..index + 2) == Some(b"/*") {
            index += 2;
            while index + 1 < content.len() && &content[index..index + 2] != b"*/" {
                index += 1;
            }
            index = (index + 2).min(content.len());
            continue;
        }
        output.push(content[index]);
        index += 1;
    }

    strip_trailing_commas(&output)
}

fn strip_trailing_commas(content: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(content.len());
    let mut index = 0;
    let mut in_string = false;

    while index < content.len() {
        let byte = content[index];
        if byte == b'"' {
            output.push(byte);
            index += 1;
            in_string = !in_string;
            continue;
        }
        if in_string && byte == b'\\' && index + 1 < content.len() {
            output.push(byte);
            index += 1;
            output.push(content[index]);
            index += 1;
            continue;
        }
        if !in_string && byte == b',' {
            let mut next = index + 1;
            while next < content.len() && content[next].is_ascii_whitespace() {
                next += 1;
            }
            if content
                .get(next)
                .is_some_and(|byte| matches!(byte, b'}' | b']'))
            {
                index += 1;
                continue;
            }
        }
        output.push(byte);
        index += 1;
    }

    output
}

pub(super) fn parse(content: &[u8]) -> Result<Value> {
    Ok(serde_json::from_slice(&strip(content))?)
}

/// Sets `name` to `value` inside the object found at `path`, creating the
/// objects on the way when they are missing and replacing a `null`.
///
/// `step` is the indentation one nesting level uses in this file, which is what
/// an inserted member is laid out with.
pub(super) fn set_member(
    text: &str,
    path: &[&str],
    name: &str,
    value: &Value,
    step: &str,
) -> Result<String> {
    let mut open = root_open(text)?;
    for (depth, key) in path.iter().enumerate() {
        match member_in(text, open, key)? {
            Some(item) => match text.as_bytes().get(item.value_start) {
                Some(b'{') => open = item.value_start,
                Some(b'n') if &text[item.value_start..item.end] == "null" => {
                    let nested = chain(&path[depth + 1..], name, value);
                    let indent = line_indent(text, item.start).unwrap_or_default();
                    let rendered = render(&nested, &indent, step)?;
                    return Ok(format!(
                        "{}{rendered}{}",
                        &text[..item.value_start],
                        &text[item.end..]
                    ));
                }
                _ => bail!(
                    "{} in this file must contain a JSON object",
                    path[..=depth].join(".")
                ),
            },
            None => {
                let nested = chain(&path[depth + 1..], name, value);
                return insert_member(text, open, key, &nested, step);
            }
        }
    }
    upsert_member(text, open, name, value, step)
}

/// Takes the member `name` out of the object at `path`, or answers `None` when
/// there is nothing there to take.
pub(super) fn remove_member(text: &str, path: &[&str], name: &str) -> Result<Option<String>> {
    let Ok(mut open) = root_open(text) else {
        return Ok(None);
    };
    for key in path {
        let Some(item) = member_in(text, open, key)? else {
            return Ok(None);
        };
        if text.as_bytes().get(item.value_start) != Some(&b'{') {
            return Ok(None);
        }
        open = item.value_start;
    }
    let items = items(text, open)?;
    let Some(position) = items
        .iter()
        .position(|item| item.key.as_deref() == Some(name))
    else {
        return Ok(None);
    };
    Ok(Some(remove_item(text, open, &items, position)))
}

/// Appends `values` to the array at `path`, creating the array when it is
/// missing and the objects on the way to it when they are.
pub(super) fn push_elements(
    text: &str,
    path: &[&str],
    values: &[Value],
    step: &str,
) -> Result<String> {
    if values.is_empty() {
        return Ok(text.to_owned());
    }
    let Some((last, parents)) = path.split_last() else {
        bail!("an array needs a path to sit at");
    };
    match value_at(text, path)? {
        // `null` is how a hand-edited or half-written client config says
        // "nothing here yet", and the client reads it as an empty array.
        None => set_member(text, parents, last, &Value::Array(values.to_vec()), step),
        Some(open)
            if text.as_bytes().get(open) == Some(&b'n')
                && &text[open..value_end(text.as_bytes(), open).unwrap_or(open)] == "null" =>
        {
            set_member(text, parents, last, &Value::Array(values.to_vec()), step)
        }
        Some(open) => {
            if text.as_bytes().get(open) != Some(&b'[') {
                bail!("{} must contain an array", path.join("."));
            }
            append_elements(text, open, values, step)
        }
    }
}

/// Takes the elements at `indices` out of the array at `path`.
///
/// Removed from the end, so the offsets of the ones still to go are the ones
/// the scan answered.
pub(super) fn remove_elements(text: &str, path: &[&str], indices: &[usize]) -> Result<String> {
    let Some(open) = value_at(text, path)? else {
        return Ok(text.to_owned());
    };
    if indices.is_empty() {
        return Ok(text.to_owned());
    }
    if text.as_bytes().get(open) != Some(&b'[') {
        bail!("{} must contain an array", path.join("."));
    }
    let mut edited = text.to_owned();
    let mut sorted = indices.to_vec();
    sorted.sort_unstable();
    for index in sorted.into_iter().rev() {
        // Scanned again for every removal: the comma that followed an element
        // is the previous element's once the one after it has gone, and a
        // stale list would leave the interior of an emptied array behind.
        let items = items(&edited, open)?;
        if index >= items.len() {
            bail!("{} holds no element at {index}", path.join("."));
        }
        edited = remove_item(&edited, open, &items, index);
    }
    Ok(edited)
}

/// The file ends the way a rendered one did: with its own line ending.
pub(super) fn with_final_newline(text: String) -> String {
    if text.ends_with('\n') {
        return text;
    }
    let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
    format!("{text}{ending}")
}

/// One member of an object, or one element of an array.
struct Item {
    key: Option<String>,
    /// The key's opening quote, or the element's first byte.
    start: usize,
    value_start: usize,
    end: usize,
    comma: Option<usize>,
}

fn skip(bytes: &[u8], mut at: usize) -> usize {
    loop {
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if at < bytes.len() && bytes[at..].starts_with(b"//") {
            at += 2;
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
            continue;
        }
        if at < bytes.len() && bytes[at..].starts_with(b"/*") {
            at += 2;
            while at + 1 < bytes.len() && !bytes[at..].starts_with(b"*/") {
                at += 1;
            }
            at = (at + 2).min(bytes.len());
            continue;
        }
        return at;
    }
}

fn string_end(bytes: &[u8], mut at: usize) -> Option<usize> {
    at += 1;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 2,
            b'"' => return Some(at + 1),
            _ => at += 1,
        }
    }
    None
}

/// The index of the bracket that closes the one at `at`.
fn matching_close(bytes: &[u8], at: usize) -> Option<usize> {
    let opener = *bytes.get(at)?;
    let closer = match opener {
        b'{' => b'}',
        b'[' => b']',
        _ => return None,
    };
    let mut depth = 0usize;
    let mut index = at;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => index = string_end(bytes, index)?,
            b'/' if bytes[index..].starts_with(b"//") => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes[index..].starts_with(b"/*") => {
                index += 2;
                while index + 1 < bytes.len() && !bytes[index..].starts_with(b"*/") {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
            }
            byte if byte == opener => {
                depth += 1;
                index += 1;
            }
            byte if byte == closer => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn value_end(bytes: &[u8], at: usize) -> Option<usize> {
    match *bytes.get(at)? {
        b'"' => string_end(bytes, at),
        b'{' | b'[' => matching_close(bytes, at).map(|close| close + 1),
        _ => {
            let mut index = at;
            while index < bytes.len()
                && !matches!(bytes[index], b',' | b'}' | b']')
                && !bytes[index].is_ascii_whitespace()
                && !bytes[index..].starts_with(b"//")
                && !bytes[index..].starts_with(b"/*")
            {
                index += 1;
            }
            Some(index)
        }
    }
}

fn items(text: &str, open: usize) -> Result<Vec<Item>> {
    let bytes = text.as_bytes();
    let close = matching_close(bytes, open).context("an unterminated object or array")?;
    let object = bytes[open] == b'{';
    let mut items = Vec::new();
    let mut at = skip(bytes, open + 1);
    while at < close {
        let start = at;
        let key = if object {
            if bytes.get(at) != Some(&b'"') {
                bail!("expected a key at byte {at}");
            }
            let key_end = string_end(bytes, at).context("an unterminated key")?;
            let key: String =
                serde_json::from_str(&text[at..key_end]).context("a key that is not a string")?;
            at = skip(bytes, key_end);
            if bytes.get(at) != Some(&b':') {
                bail!("expected ':' at byte {at}");
            }
            at = skip(bytes, at + 1);
            Some(key)
        } else {
            None
        };
        let value_start = at;
        let end = value_end(bytes, value_start).context("an unterminated value")?;
        at = skip(bytes, end);
        let comma = if bytes.get(at) == Some(&b',') {
            let comma = at;
            at = skip(bytes, at + 1);
            Some(comma)
        } else {
            None
        };
        items.push(Item {
            key,
            start,
            value_start,
            end,
            comma,
        });
    }
    Ok(items)
}

fn root_open(text: &str) -> Result<usize> {
    let bytes = text.as_bytes();
    let at = skip(bytes, 0);
    match bytes.get(at) {
        Some(b'{') => Ok(at),
        _ => bail!("the configuration must be a JSON object"),
    }
}

fn member_in(text: &str, open: usize, name: &str) -> Result<Option<Item>> {
    Ok(items(text, open)?
        .into_iter()
        .find(|item| item.key.as_deref() == Some(name)))
}

/// The opening bracket or first byte of the value at `path`, when it is there
/// and every object on the way to it is one.
fn value_at(text: &str, path: &[&str]) -> Result<Option<usize>> {
    let Ok(mut open) = root_open(text) else {
        return Ok(None);
    };
    for (depth, key) in path.iter().enumerate() {
        let Some(item) = member_in(text, open, key)? else {
            return Ok(None);
        };
        if depth + 1 == path.len() {
            return Ok(Some(item.value_start));
        }
        if text.as_bytes().get(item.value_start) != Some(&b'{') {
            return Ok(None);
        }
        open = item.value_start;
    }
    Ok(None)
}

fn chain(path: &[&str], name: &str, value: &Value) -> Value {
    let mut node = Value::Object(serde_json::Map::from_iter([(
        name.to_owned(),
        value.clone(),
    )]));
    for key in path.iter().rev() {
        node = Value::Object(serde_json::Map::from_iter([((*key).to_owned(), node)]));
    }
    node
}

fn render(value: &Value, indent: &str, step: &str) -> Result<String> {
    let mut output = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(
        &mut output,
        serde_json::ser::PrettyFormatter::with_indent(step.as_bytes()),
    );
    value.serialize(&mut serializer)?;
    let rendered = String::from_utf8(output)?;
    Ok(rendered.replace('\n', &format!("\n{indent}")))
}

fn newline(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

/// The whitespace a line begins with, whatever sits after it.
fn leading_indent(text: &str, at: usize) -> String {
    let bytes = text.as_bytes();
    let start = line_start(bytes, at);
    let mut end = start;
    while end < at && (bytes[end] == b' ' || bytes[end] == b'\t') {
        end += 1;
    }
    text[start..end].to_owned()
}

fn line_start(bytes: &[u8], at: usize) -> usize {
    let mut start = at;
    while start > 0 && bytes[start - 1] != b'\n' {
        start -= 1;
    }
    start
}

/// The indentation in front of `at`, when there is nothing but whitespace
/// between it and the start of its line.
fn line_indent(text: &str, at: usize) -> Option<String> {
    let bytes = text.as_bytes();
    let start = line_start(bytes, at);
    bytes[start..at]
        .iter()
        .all(|byte| *byte == b' ' || *byte == b'\t')
        .then(|| text[start..at].to_owned())
}

/// One nesting level deeper than the container's own line, or — when the
/// container already holds something — the indentation its first item uses.
fn item_indent(text: &str, open: usize, items: &[Item], step: &str) -> String {
    if let Some(first) = items.first()
        && let Some(indent) = line_indent(text, first.start)
    {
        return indent;
    }
    format!("{}{step}", leading_indent(text, open))
}

fn insert_member(text: &str, open: usize, name: &str, value: &Value, step: &str) -> Result<String> {
    let items = items(text, open)?;
    let close = matching_close(text.as_bytes(), open).context("an unterminated object")?;
    let indent = item_indent(text, open, &items, step);
    let rendered = render(value, &indent, step)?;
    let key = serde_json::to_string(name)?;
    let newline = newline(text);
    if items.is_empty() {
        if text[open + 1..close].trim().is_empty() {
            let close_indent = leading_indent(text, open);
            return Ok(format!(
                "{}{newline}{indent}{key}: {rendered}{newline}{close_indent}{}",
                &text[..open + 1],
                &text[close..]
            ));
        }
        return Ok(format!(
            "{}{newline}{indent}{key}: {rendered}{}",
            &text[..close],
            &text[close..]
        ));
    }
    let last = items.last().expect("an empty list was answered above");
    match last.comma {
        // The comma already separates this member from the new one, so the new
        // one goes before whatever is left — which keeps a trailing comment
        // attached to the member it was written under.
        Some(_) => {
            let closer_line = line_start(bytes(text), close);
            if closer_line > last.end {
                return Ok(format!(
                    "{}{indent}{key}: {rendered}{newline}{}",
                    &text[..closer_line],
                    &text[closer_line..]
                ));
            }
            let comma = last.comma.expect("the arm answered a comma");
            Ok(format!(
                "{}{newline}{indent}{key}: {rendered}{}",
                &text[..comma + 1],
                &text[comma + 1..]
            ))
        }
        None => Ok(format!(
            "{},{newline}{indent}{key}: {rendered}{}",
            &text[..last.end],
            &text[last.end..]
        )),
    }
}

fn bytes(text: &str) -> &[u8] {
    text.as_bytes()
}

fn upsert_member(text: &str, open: usize, name: &str, value: &Value, step: &str) -> Result<String> {
    match member_in(text, open, name)? {
        Some(item) => {
            let indent = line_indent(text, item.start).unwrap_or_default();
            let rendered = render(value, &indent, step)?;
            Ok(format!(
                "{}{rendered}{}",
                &text[..item.value_start],
                &text[item.end..]
            ))
        }
        None => insert_member(text, open, name, value, step),
    }
}

fn append_elements(text: &str, open: usize, values: &[Value], step: &str) -> Result<String> {
    let items = items(text, open)?;
    let close = matching_close(text.as_bytes(), open).context("an unterminated array")?;
    let indent = item_indent(text, open, &items, step);
    let rendered = values
        .iter()
        .map(|value| render(value, &indent, step))
        .collect::<Result<Vec<_>>>()?;
    let newline = newline(text);
    let joined = rendered.join(&format!(",{newline}{indent}"));
    if items.is_empty() {
        if text[open + 1..close].trim().is_empty() {
            let close_indent = leading_indent(text, open);
            return Ok(format!(
                "{}{newline}{indent}{joined}{newline}{close_indent}{}",
                &text[..open + 1],
                &text[close..]
            ));
        }
        return Ok(format!(
            "{}{newline}{indent}{joined}{}",
            &text[..close],
            &text[close..]
        ));
    }
    let last = items.last().expect("an empty list was answered above");
    match last.comma {
        Some(_) => {
            let closer_line = line_start(text.as_bytes(), close);
            if closer_line > last.end {
                return Ok(format!(
                    "{}{indent}{joined}{newline}{}",
                    &text[..closer_line],
                    &text[closer_line..]
                ));
            }
            let comma = last.comma.expect("the arm answered a comma");
            Ok(format!(
                "{}{newline}{indent}{joined}{}",
                &text[..comma + 1],
                &text[comma + 1..]
            ))
        }
        None => Ok(format!(
            "{},{newline}{indent}{joined}{}",
            &text[..last.end],
            &text[last.end..]
        )),
    }
}

fn remove_item(text: &str, open: usize, items: &[Item], index: usize) -> String {
    let item = &items[index];
    let bytes = text.as_bytes();
    if let Some(comma) = item.comma {
        let own_line = line_indent(text, item.start)
            .map(|_| line_start(bytes, item.start))
            .unwrap_or(item.start);
        let mut to = comma + 1;
        while matches!(bytes.get(to), Some(b' ') | Some(b'\t')) {
            to += 1;
        }
        // A member alone on its line takes the line's newline with it, or the
        // file keeps an empty line where Leteo's entry was.
        if own_line != item.start {
            if text[to..].starts_with("\r\n") {
                to += 2;
            } else if text[to..].starts_with('\n') {
                to += 1;
            }
        }
        return format!("{}{}", &text[..own_line], &text[to..]);
    }
    if index > 0 {
        let comma = items[index - 1]
            .comma
            .expect("a comma separates two members");
        let from = line_indent(text, item.start).map(|_| line_start(bytes, item.start));
        // A comment in the gap belongs to whoever wrote it, not to the entry
        // being removed. It stays, and the member's own line goes instead.
        if text[comma + 1..item.start].trim().is_empty()
            || from.is_none_or(|from| from <= comma + 1)
        {
            return format!("{}{}", &text[..comma], &text[item.end..]);
        }
        let from = from.expect("answered above");
        let mut to = item.end;
        if text[to..].starts_with("\r\n") {
            to += 2;
        } else if text[to..].starts_with('\n') {
            to += 1;
        }
        return format!(
            "{}{}{}",
            &text[..comma],
            &text[comma + 1..from],
            &text[to..]
        );
    }
    let close = matching_close(bytes, open).expect("the container was scanned");
    if text[open + 1..item.start].trim().is_empty() && text[item.end..close].trim().is_empty() {
        return format!("{}{}", &text[..open + 1], &text[close..]);
    }
    format!("{}{}", &text[..item.start], &text[item.end..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A fixture as the lines of the file it stands for.
    ///
    /// Written this way, no line of the fixture is a run of indentation inside
    /// a string literal, which is the shape the repository's guard reads as a
    /// sentence broken by the line it was written on.
    fn document(lines: &[&str]) -> String {
        format!("{}\n", lines.join("\n"))
    }

    #[test]
    fn a_member_is_added_beside_the_ones_already_there() {
        let text = "{\n  \"theme\": \"dark\",\n  // mine\n}\n";
        let edited =
            set_member(text, &[], "mcp", &json!({"leteo": {"type": "local"}}), "  ").unwrap();
        assert_eq!(
            edited,
            "{\n  \"theme\": \"dark\",\n  // mine\n  \"mcp\": {\n    \"leteo\": {\n      \"type\": \"local\"\n    }\n  }\n}\n"
        );
        assert_eq!(
            parse(edited.as_bytes()).unwrap()["mcp"]["leteo"]["type"],
            "local"
        );
    }

    #[test]
    fn a_trailing_comma_is_reused_rather_than_doubled() {
        let text = "{\n  \"theme\": \"dark\",\n}\n";
        let edited = set_member(text, &[], "mcp", &json!({}), "  ").unwrap();
        assert_eq!(edited, "{\n  \"theme\": \"dark\",\n  \"mcp\": {}\n}\n");
    }

    #[test]
    fn an_existing_member_is_replaced_in_place_and_the_comment_stays() {
        let before = document(&[
            "{",
            "  \"mcp\": {",
            "    // our server",
            "    \"leteo\": {",
            "      \"command\": [\"old\"]",
            "    }",
            "  }",
            "}",
        ]);
        let edited = set_member(
            &before,
            &["mcp"],
            "leteo",
            &json!({"command": ["new"]}),
            "  ",
        )
        .unwrap();
        assert_eq!(
            edited,
            document(&[
                "{",
                "  \"mcp\": {",
                "    // our server",
                "    \"leteo\": {",
                "      \"command\": [",
                "        \"new\"",
                "      ]",
                "    }",
                "  }",
                "}",
            ])
        );
    }

    #[test]
    fn the_objects_on_the_way_are_created_when_they_are_missing() {
        let text = "{\n  \"theme\": \"dark\"\n}\n";
        let edited = set_member(text, &["a", "b"], "c", &json!(1), "  ").unwrap();
        assert_eq!(
            edited,
            "{\n  \"theme\": \"dark\",\n  \"a\": {\n    \"b\": {\n      \"c\": 1\n    }\n  }\n}\n"
        );
    }

    #[test]
    fn a_removed_member_takes_its_comma_and_leaves_the_comment() {
        let text =
            "{\n  \"a\": 1,\n  // keep me\n  \"leteo\": {\n    \"x\": 1\n  },\n  \"b\": 2\n}\n";
        let edited = remove_member(text, &[], "leteo").unwrap().unwrap();
        assert_eq!(edited, "{\n  \"a\": 1,\n  // keep me\n  \"b\": 2\n}\n");
    }

    #[test]
    fn a_removed_last_member_takes_the_comma_before_it() {
        let text = "{\n  \"a\": 1,\n  \"leteo\": {\"x\": 1}\n}\n";
        let edited = remove_member(text, &[], "leteo").unwrap().unwrap();
        assert_eq!(edited, "{\n  \"a\": 1\n}\n");
    }

    #[test]
    fn the_only_member_leaves_an_empty_object() {
        let text = "{\n  \"mcp\": {\n    \"leteo\": {\"x\": 1}\n  }\n}\n";
        let edited = remove_member(text, &["mcp"], "leteo").unwrap().unwrap();
        assert_eq!(edited, "{\n  \"mcp\": {}\n}\n");
    }

    #[test]
    fn elements_are_appended_and_removed_by_index() {
        let text = "{\n  \"hooks\": {\n    \"Stop\": [\n      {\n        \"command\": \"mine\"\n      }\n    ]\n  }\n}\n";
        let appended = push_elements(
            text,
            &["hooks", "Stop"],
            &[json!({"command": "leteo hook stop"})],
            "  ",
        )
        .unwrap();
        assert_eq!(
            appended,
            "{\n  \"hooks\": {\n    \"Stop\": [\n      {\n        \"command\": \"mine\"\n      },\n      {\n        \"command\": \"leteo hook stop\"\n      }\n    ]\n  }\n}\n"
        );
        let removed = remove_elements(&appended, &["hooks", "Stop"], &[1]).unwrap();
        assert_eq!(removed, text);
        let emptied = remove_elements(&appended, &["hooks", "Stop"], &[0, 1]).unwrap();
        assert_eq!(emptied, "{\n  \"hooks\": {\n    \"Stop\": []\n  }\n}\n");
    }

    #[test]
    fn a_missing_array_is_created_with_its_first_element() {
        let text = "{\n  \"hooks\": {}\n}\n";
        let edited = push_elements(text, &["hooks", "Stop"], &[json!({"a": 1})], "  ").unwrap();
        assert_eq!(
            edited,
            "{\n  \"hooks\": {\n    \"Stop\": [\n      {\n        \"a\": 1\n      }\n    ]\n  }\n}\n"
        );
    }

    #[test]
    fn the_comma_before_a_multiline_last_member_goes_with_it() {
        let before = document(&[
            "{",
            "  \"mcp\": {",
            "    \"other\": {",
            "      \"a\": 1,",
            "    },",
            "    \"leteo\": {",
            "      \"type\": \"local\"",
            "    }",
            "  }",
            "}",
        ]);
        let edited = remove_member(&before, &["mcp"], "leteo").unwrap().unwrap();
        assert_eq!(
            edited,
            document(&[
                "{",
                "  \"mcp\": {",
                "    \"other\": {",
                "      \"a\": 1,",
                "    }",
                "  }",
                "}",
            ])
        );
    }

    #[test]
    fn a_comment_between_the_last_two_members_survives_the_last_ones_removal() {
        let text = "{\n  \"a\": 1,\n  // about b\n  \"b\": 2\n}\n";
        let edited = remove_member(text, &[], "b").unwrap().unwrap();
        assert_eq!(edited, "{\n  \"a\": 1\n  // about b\n}\n");
    }

    #[test]
    fn the_scanner_reads_strings_comments_and_scalars_where_they_end() {
        let text = r#"{
  "url": "https://example.com/a//b", // the slashes are not a comment
  "braces": "{\"a\": [1, 2]}",
  "block": /* } , // */ 1,
  "after": true/* still the comment */,
  "key\"": "value"
}"#;
        let edited = set_member(text, &[], "added", &json!({"ok": true}), "  ").unwrap();
        let parsed = parse(edited.as_bytes()).unwrap();
        assert_eq!(parsed["url"], "https://example.com/a//b");
        assert_eq!(parsed["braces"], "{\"a\": [1, 2]}");
        assert_eq!(parsed["block"], 1);
        assert_eq!(parsed["after"], true);
        assert_eq!(parsed["key\""], "value");
        assert_eq!(parsed["added"]["ok"], true);
        assert!(
            edited.contains("/* still the comment */"),
            "the comment after a scalar is not part of it:\n{edited}"
        );
    }

    #[test]
    fn a_shape_the_editor_cannot_edit_says_which_key_it_was() {
        let error = set_member("{\"mcp\": []}", &["mcp"], "leteo", &json!({}), "  ")
            .unwrap_err()
            .to_string();
        assert_eq!(error, "mcp in this file must contain a JSON object");
        let error = push_elements(
            "{\"hooks\": {\"Stop\": {}}}",
            &["hooks", "Stop"],
            &[json!(1)],
            "  ",
        )
        .unwrap_err()
        .to_string();
        assert_eq!(error, "hooks.Stop must contain an array");
    }

    #[test]
    fn windows_line_endings_are_the_ones_the_file_had() {
        let text = "{\r\n  \"a\": 1\r\n}\r\n";
        let edited = set_member(text, &[], "b", &json!(2), "  ").unwrap();
        assert!(edited.contains("\"a\": 1,\r\n  \"b\": 2"), "{edited:?}");
    }
}
