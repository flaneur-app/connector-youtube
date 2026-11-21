// SPDX-License-Identifier: AGPL-3.0-or-later

//! RustyPipe errors, in terms the UI can act on, with every secret redacted.

use flaneur_connector::Error;
use rustypipe::error::{AuthError, Error as RpError, ExtractionError, UnavailabilityReason};

use crate::session::Redactor;

/// Maps a RustyPipe error; messages go through `redactor` first.
pub(crate) fn map(e: RpError, redactor: &Redactor) -> Error {
    let msg = |s: &str| redactor.apply(s);
    match e {
        RpError::Extraction(e) => match e {
            ExtractionError::Unavailable { reason, msg: why } => match reason {
                UnavailabilityReason::IpBan
                | UnavailabilityReason::VpnBan
                | UnavailabilityReason::Captcha
                | UnavailabilityReason::TryAgain => Error::RateLimited { retry_after: None },
                UnavailabilityReason::Deleted => Error::NotFound,
                _ => Error::Unavailable(msg(&format!("{reason}: {why}"))),
            },
            ExtractionError::NotFound { .. } => Error::NotFound,
            ExtractionError::BadRequest(m) => Error::Service(msg(&format!("bad request: {m}"))),
            ExtractionError::Botguard(m) => Error::Service(msg(&format!("PO token: {m}"))),
            e @ (ExtractionError::InvalidData(_)
            | ExtractionError::Deobfuscation(_)
            | ExtractionError::WrongResult(_)
            | ExtractionError::DeserializationWarnings) => Error::Parse(msg(&e.to_string())),
            e @ ExtractionError::Redirect(_) => Error::Service(msg(&e.to_string())),
        },
        RpError::Http(m) => Error::Network(msg(&m)),
        RpError::HttpStatus(status, m) => status_error(status, &msg(&m)),
        RpError::Auth(AuthError::NoLogin) => Error::SignInRequired,
        RpError::Auth(e) => Error::Service(msg(&e.to_string())),
        RpError::Other(m) => Error::Service(msg(&m)),
    }
}

/// Maps an HTTP status of a YouTube answer.
pub(crate) fn status_error(status: u16, message: &str) -> Error {
    match status {
        401 => Error::SignInRequired,
        404 => Error::NotFound,
        429 => Error::RateLimited { retry_after: None },
        _ => Error::Service(format!("HTTP {status}: {message}")),
    }
}

/// Whether the next InnerTube client may succeed where this one failed: the player script
/// could not be parsed, the answer did not match, or the client is not allowed for this video.
pub(crate) fn worth_another_client(e: &RpError) -> bool {
    match e {
        RpError::Extraction(e) => match e {
            ExtractionError::Unavailable { reason, .. } => {
                *reason == UnavailabilityReason::UnsupportedClient
            }
            ExtractionError::NotFound { .. } | ExtractionError::Redirect(_) => false,
            ExtractionError::BadRequest(_)
            | ExtractionError::InvalidData(_)
            | ExtractionError::Deobfuscation(_)
            | ExtractionError::Botguard(_)
            | ExtractionError::WrongResult(_)
            | ExtractionError::DeserializationWarnings => true,
        },
        RpError::Other(_) => true,
        RpError::Http(_) | RpError::HttpStatus(..) | RpError::Auth(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bot_checks_are_rate_limits() {
        let e = RpError::Extraction(ExtractionError::Unavailable {
            reason: UnavailabilityReason::Captcha,
            msg: "Sign in to confirm you're not a bot".into(),
        });
        assert!(matches!(
            map(e, &Redactor::default()),
            Error::RateLimited { .. }
        ));
    }

    #[test]
    fn missing_login_asks_for_sign_in() {
        assert!(matches!(
            map(RpError::Auth(AuthError::NoLogin), &Redactor::default()),
            Error::SignInRequired
        ));
    }

    #[test]
    fn messages_are_redacted() {
        let e = RpError::Http("request failed, Cookie: SAPISID=abcdefgh123".into());
        let Error::Network(m) = map(e, &Redactor::default()) else {
            panic!("network error expected");
        };
        assert!(!m.contains("abcdefgh123"));
    }

    #[test]
    fn deobfuscation_errors_switch_client() {
        let e = RpError::Extraction(ExtractionError::InvalidData("x".into()));
        assert!(worth_another_client(&e));
        let e = RpError::Extraction(ExtractionError::Unavailable {
            reason: UnavailabilityReason::Private,
            msg: String::new(),
        });
        assert!(!worth_another_client(&e));
    }
}
