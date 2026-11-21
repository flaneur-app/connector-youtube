// SPDX-License-Identifier: AGPL-3.0-or-later

//! Cookies of a signed-in session, and redaction of everything secret they contain.

use std::fmt;

/// Cookie names whose values must never appear in a message, even when the session that owns
/// them is not the one active (an error from a sign-in attempt, for instance).
const SECRET_COOKIES: [&str; 13] = [
    "SAPISID",
    "APISID",
    "HSID",
    "SSID",
    "SID",
    "SIDCC",
    "LOGIN_INFO",
    "__Secure-1PSID",
    "__Secure-3PSID",
    "__Secure-1PAPISID",
    "__Secure-3PAPISID",
    "__Secure-1PSIDTS",
    "__Secure-3PSIDTS",
];

/// One cookie of a Netscape `cookies.txt` line.
struct CookieLine<'a> {
    line: &'a str,
    name: &'a str,
    value: &'a str,
}

/// Cookies of `youtube.com` (and its subdomains) in a Netscape `cookies.txt`.
fn youtube_cookies(cookies_txt: &str) -> impl Iterator<Item = CookieLine<'_>> {
    cookies_txt.lines().filter_map(|line| {
        let fields_line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        if fields_line.starts_with('#') || fields_line.trim().is_empty() {
            return None;
        }
        let fields: Vec<&str> = fields_line
            .trim_end_matches(['\r', '\n'])
            .split('\t')
            .collect();
        let domain = fields.first()?.trim_start_matches('.');
        if fields.len() != 7 || !(domain == "youtube.com" || domain.ends_with(".youtube.com")) {
            return None;
        }
        Some(CookieLine {
            line,
            name: fields[5],
            value: fields[6],
        })
    })
}

/// The session secret: the `youtube.com` lines of the user's `cookies.txt` only. Cookies of other
/// Google domains in the same export are dropped, so the keyring holds no more than needed.
pub(crate) fn youtube_cookie_file(cookies_txt: &str) -> String {
    let mut out = String::from("# Netscape HTTP Cookie File\n");
    for cookie in youtube_cookies(cookies_txt) {
        out.push_str(cookie.line.trim_end_matches(['\r', '\n']));
        out.push('\n');
    }
    out
}

/// `Cookie:` header value for `youtube.com`.
pub(crate) fn cookie_header(cookies_txt: &str) -> String {
    youtube_cookies(cookies_txt)
        .map(|c| format!("{}={}", c.name, c.value))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Removes secrets from text that may end up in an error, a log or the UI.
#[derive(Clone, Default)]
pub(crate) struct Redactor {
    values: Vec<String>,
}

impl fmt::Debug for Redactor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Redactor({} values)", self.values.len())
    }
}

impl Redactor {
    /// Redacts the cookie values of this `cookies.txt`, on top of the known cookie names.
    pub(crate) fn for_cookies(cookies_txt: &str) -> Self {
        let mut values: Vec<String> = youtube_cookies(cookies_txt)
            // Short values ("1", "YES") are not secrets and would mangle every message.
            .filter(|c| c.value.len() >= 8)
            .map(|c| c.value.to_owned())
            .collect();
        // Longest first, so a value that contains another is replaced whole.
        values.sort_by_key(|v| std::cmp::Reverse(v.len()));
        values.dedup();
        Self { values }
    }

    /// `text` with every cookie value, `NAME=value` pair of a secret cookie and `SAPISIDHASH`
    /// replaced by `<redacted>`.
    pub(crate) fn apply(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for value in &self.values {
            if out.contains(value.as_str()) {
                out = out.replace(value.as_str(), "<redacted>");
            }
        }
        for name in SECRET_COOKIES
            .iter()
            .map(|n| format!("{n}="))
            .chain(["SAPISIDHASH ".to_owned(), "SAPISID1PHASH ".to_owned()])
        {
            out = redact_after(&out, &name);
        }
        out
    }
}

/// Replaces the token following each occurrence of `marker` (up to `;`, whitespace or a quote).
fn redact_after(text: &str, marker: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find(marker) {
        // `SID=` also matches the end of `SAPISID=`: both are secret, the result is the same.
        let (before, after) = rest.split_at(pos + marker.len());
        out.push_str(before);
        let end = after
            .find(|c: char| c == ';' || c == '"' || c == '\'' || c.is_whitespace())
            .unwrap_or(after.len());
        if end > 0 && !after[..end].starts_with("<redacted>") {
            out.push_str("<redacted>");
        } else {
            out.push_str(&after[..end]);
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const COOKIES: &str = "# Netscape HTTP Cookie File\n\
        .youtube.com\tTRUE\t/\tTRUE\t1800000000\tSAPISID\tsapisid-value-123\n\
        #HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t1800000000\t__Secure-3PSID\tpsid-value-4567890\n\
        .youtube.com\tTRUE\t/\tFALSE\t1800000000\tPREF\tf6=40000000\n\
        .google.com\tTRUE\t/\tTRUE\t1800000000\tSID\tgoogle-sid-value\n";

    #[test]
    fn keeps_only_youtube_cookies() {
        let file = youtube_cookie_file(COOKIES);
        assert!(file.contains("sapisid-value-123"));
        assert!(file.contains("#HttpOnly_.youtube.com"));
        assert!(!file.contains("google-sid-value"));
        assert_eq!(
            cookie_header(COOKIES),
            "SAPISID=sapisid-value-123; __Secure-3PSID=psid-value-4567890; PREF=f6=40000000"
        );
    }

    #[test]
    fn redacts_cookie_values_and_auth_headers() {
        let r = Redactor::for_cookies(COOKIES);
        let msg = "failed: Cookie: SAPISID=sapisid-value-123; PREF=f6=40000000 \
                   Authorization: SAPISIDHASH 1700000000_abcdef psid-value-4567890";
        let out = r.apply(msg);
        assert!(!out.contains("sapisid-value-123"), "{out}");
        assert!(!out.contains("psid-value-4567890"), "{out}");
        assert!(!out.contains("1700000000_abcdef"), "{out}");
        // Every value of the session goes, secret or not; the rest of the message stays.
        assert!(
            out.starts_with("failed: Cookie: SAPISID=<redacted>"),
            "{out}"
        );
        assert!(
            out.contains("Authorization: SAPISIDHASH <redacted>"),
            "{out}"
        );
    }

    #[test]
    fn redacts_known_cookie_names_without_a_session() {
        let out = Redactor::default().apply("Cookie: LOGIN_INFO=AFmmF2swRQ; SID=abc");
        assert_eq!(out, "Cookie: LOGIN_INFO=<redacted>; SID=<redacted>");
    }

    #[test]
    fn debug_shows_no_value() {
        let r = Redactor::for_cookies(COOKIES);
        assert!(!format!("{r:?}").contains("sapisid"));
    }
}
