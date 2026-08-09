//! The parts that exist because a browser is on the other end.
//!
//! Cookies, the headers a reverse proxy rewrites, and the escaping that keeps a
//! project name out of the dashboard's markup.

use super::*;

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

/// Reports whether the `Host` header names this machine.
///
/// A caller that lies here only weakens the cookie it receives itself; a
/// victim's browser always sends the real host.
fn request_host_is_loopback(headers: &HeaderMap) -> bool {
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let host = host.trim();
    // Strip the port, taking care of the bracketed IPv6 form.
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
