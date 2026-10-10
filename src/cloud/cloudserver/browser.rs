use super::*;
use crate::cloud::cloudstore::{
    BrowseMemory, BrowseProject, BrowseSession, Limited, MEMORY_LIST_LIMIT, MEMORY_SEARCH_LIMIT,
    PROJECT_LIST_LIMIT, SESSION_LIST_LIMIT,
};

pub(super) fn dashboard_cookie(value: &str, secure: bool) -> String {
    format!(
        "{DASHBOARD_COOKIE}={value}; Path=/dashboard; HttpOnly; SameSite=Lax; Max-Age=28800{}",
        if secure { "; Secure" } else { "" }
    )
}

pub(super) fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|cookie| {
                let (key, value) = cookie.trim().split_once('=')?;
                (key == name).then_some(value)
            })
        })
}

fn forwarded_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|protocol| protocol.trim().eq_ignore_ascii_case("https"))
        })
}

/// Decides whether the dashboard session cookie is marked `Secure`.
///
/// Trusting `X-Forwarded-Proto` alone fails open: the documented deployment
/// puts a TLS proxy in front of this service, and a proxy that forgets to set
/// that header — nginx does not add it on its own — would hand an
/// administrator a session cookie the browser happily sends over plaintext.
///
/// So the cookie is marked `Secure` unless the request is plainly local, which
/// keeps `http://localhost` development working on every browser without
/// weakening a single real deployment.
pub(super) fn requires_secure_cookie(headers: &HeaderMap) -> bool {
    if forwarded_https(headers) {
        return true;
    }
    !request_host_is_loopback(headers)
}

fn request_host_is_loopback(headers: &HeaderMap) -> bool {
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let host = host.trim();
    let host = match host.strip_prefix('[') {
        Some(rest) => rest.split_once(']').map_or(rest, |(host, _)| host),
        None => host.split_once(':').map_or(host, |(host, _)| host),
    };
    matches!(host.to_ascii_lowercase().as_str(), "localhost" | "::1")
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|address| address.is_loopback())
}

pub(super) fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub(super) fn nonempty_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value.trim()
    }
}

/// How much of a memory a list shows before the link to the whole of it.
const MEMORY_EXCERPT_CHARS: usize = 300;

/// Percent-encodes a value for a query string, keeping only unreserved characters.
pub(super) fn url_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn excerpt(text: &str) -> String {
    let mut characters = text.chars();
    let head: String = characters.by_ref().take(MEMORY_EXCERPT_CHARS).collect();
    if characters.next().is_some() {
        format!("{head}...")
    } else {
        head
    }
}

fn project_link(project: &str) -> String {
    format!(
        r#"<a href="/dashboard/project?project={}">{}</a>"#,
        url_encode(project),
        escape_html(project)
    )
}

fn memory_link(memory: &BrowseMemory) -> String {
    format!(
        r#"<a href="/dashboard/memory?project={}&amp;key={}">{}</a>"#,
        url_encode(&memory.project),
        url_encode(&memory.key),
        escape_html(nonempty_or(&memory.title, "(untitled)"))
    )
}

fn search_form(term: &str, project: Option<&str>) -> String {
    let scope = project.map_or_else(String::new, |project| {
        format!(
            r#"<input type="hidden" name="project" value="{}">"#,
            escape_html(project)
        )
    });
    format!(
        r#"<form action="/dashboard/search" method="get">{scope}<input name="q" value="{}" placeholder="Search memories" maxlength="200" required><button type="submit">Search</button></form>"#,
        escape_html(term)
    )
}

fn limit_note(truncated: bool, limit: usize, what: &str) -> String {
    if truncated {
        format!("<p class=\"note\">Showing the first {limit} {what}; more exist.</p>")
    } else {
        String::new()
    }
}

fn page(title: &str, signed_in_as: &str, body: &str) -> String {
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{title} - Leteo Cloud</title><style>body{{font:16px system-ui;max-width:64rem;margin:2rem auto;padding:1rem;background:#f4f1e8;color:#17211b}}nav,form{{display:flex;gap:1rem;align-items:center;margin:1rem 0}}input{{font:inherit;padding:.5rem;flex:1}}button{{font:inherit;padding:.5rem 1rem;background:#17211b;color:white;border:0}}li{{margin:.8rem 0;padding:1rem;background:white;border-left:4px solid #27684a;list-style:none}}ul{{padding:0}}pre{{white-space:pre-wrap;background:white;padding:1rem}}.note,small{{color:#555}}</style></head><body><nav><a href="/dashboard">Dashboard</a><a href="/dashboard/projects">Projects</a><span>Signed in as {}</span></nav>{body}</body></html>"#,
        escape_html(signed_in_as),
        title = escape_html(title),
    )
}

pub(super) fn render_projects(signed_in_as: &str, projects: &Limited<BrowseProject>) -> String {
    let items: String = projects
        .items
        .iter()
        .map(|project| {
            format!(
                "<li>{} <small>last activity {}</small></li>",
                project_link(&project.name),
                escape_html(&project.last_activity)
            )
        })
        .collect();
    let list = if projects.items.is_empty() {
        "<p>No project you have access to holds any memory yet.</p>".to_owned()
    } else {
        format!("<ul>{items}</ul>")
    };
    let body = format!(
        "<h1>Projects</h1>{}{list}{}",
        search_form("", None),
        limit_note(projects.truncated, PROJECT_LIST_LIMIT, "projects")
    );
    page("Projects", signed_in_as, &body)
}

pub(super) fn render_project(
    signed_in_as: &str,
    project: &str,
    sessions: &Limited<BrowseSession>,
    memories: &Limited<BrowseMemory>,
) -> String {
    let session_items: String = sessions
        .items
        .iter()
        .map(|session| {
            let ended = session
                .ended_at
                .as_deref()
                .map_or_else(String::new, |ended| format!(" to {}", escape_html(ended)));
            let summary = session
                .summary
                .as_deref()
                .map_or_else(String::new, |summary| {
                    format!("<br>{}", escape_html(&excerpt(summary)))
                });
            format!(
                "<li><strong>{}</strong> <small>{}{ended} in {}</small>{summary}</li>",
                escape_html(&session.id),
                escape_html(&session.started_at),
                escape_html(&session.directory),
            )
        })
        .collect();
    let memory_items = memory_list(&memories.items);
    let body = format!(
        "<h1>{}</h1>{}<h2>Sessions</h2>{}{}<h2>Memories</h2>{}{}",
        escape_html(project),
        search_form("", Some(project)),
        if sessions.items.is_empty() {
            "<p>No sessions.</p>".to_owned()
        } else {
            format!("<ul>{session_items}</ul>")
        },
        limit_note(sessions.truncated, SESSION_LIST_LIMIT, "sessions"),
        memory_items,
        limit_note(memories.truncated, MEMORY_LIST_LIMIT, "memories"),
    );
    page(project, signed_in_as, &body)
}

fn memory_list(memories: &[BrowseMemory]) -> String {
    if memories.is_empty() {
        return "<p>No memories.</p>".to_owned();
    }
    let items: String = memories
        .iter()
        .map(|memory| {
            format!(
                "<li>{} <small>{} - {} - {}</small><br>{}</li>",
                memory_link(memory),
                escape_html(&memory.kind),
                escape_html(&memory.project),
                escape_html(&memory.updated_at),
                escape_html(&excerpt(&memory.content)),
            )
        })
        .collect();
    format!("<ul>{items}</ul>")
}

pub(super) fn render_memory(signed_in_as: &str, memory: &BrowseMemory) -> String {
    let topic = memory
        .topic_key
        .as_deref()
        .map_or_else(String::new, |topic| {
            format!(" - topic {}", escape_html(topic))
        });
    let body = format!(
        "<h1>{}</h1><p><small>{} in {} - session {} - {}{topic}</small></p><pre>{}</pre>",
        escape_html(nonempty_or(&memory.title, "(untitled)")),
        escape_html(&memory.kind),
        project_link(&memory.project),
        escape_html(&memory.session_id),
        escape_html(&memory.updated_at),
        escape_html(&memory.content),
    );
    page(&memory.title, signed_in_as, &body)
}

pub(super) fn render_search(
    signed_in_as: &str,
    term: &str,
    project: Option<&str>,
    results: Option<&Limited<BrowseMemory>>,
) -> String {
    let heading = project.map_or_else(
        || "Search".to_owned(),
        |project| format!("Search in {}", escape_html(project)),
    );
    let found = match results {
        None => String::new(),
        Some(results) if results.items.is_empty() => {
            format!(
                "<p>No memory matches &quot;{}&quot;.</p>",
                escape_html(term)
            )
        }
        Some(results) => format!(
            "{}{}",
            memory_list(&results.items),
            limit_note(results.truncated, MEMORY_SEARCH_LIMIT, "matches")
        ),
    };
    let body = format!("<h1>{heading}</h1>{}{found}", search_form(term, project));
    page("Search", signed_in_as, &body)
}
