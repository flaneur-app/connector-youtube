// SPDX-License-Identifier: AGPL-3.0-or-later

//! Continuation tokens of RustyPipe paginators, as opaque [`Cursor`]s.
//!
//! A cursor is the JSON of everything RustyPipe needs to fetch the next page: the continuation
//! token, the endpoint it belongs to, the visitor data of the first page and whether the listing
//! needs the session. It never holds a secret.

use flaneur_connector::Error;
use flaneur_model::{Cursor, Page};
use rustypipe::model::paginator::{ContinuationEndpoint, Paginator};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Token {
    ctoken: String,
    endpoint: ContinuationEndpoint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    visitor_data: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    authenticated: bool,
}

/// The cursor of the page after `paginator`, `None` at the end.
pub(crate) fn next<T>(paginator: &Paginator<T>) -> Option<Cursor> {
    let token = Token {
        ctoken: paginator.ctoken.clone()?,
        endpoint: paginator.endpoint,
        visitor_data: paginator.visitor_data.clone(),
        authenticated: paginator.authenticated,
    };
    serde_json::to_string(&token).ok().map(Cursor)
}

/// A cursor for a continuation token found outside a RustyPipe paginator (raw responses).
pub(crate) fn from_ctoken(ctoken: String, authenticated: bool) -> Cursor {
    let token = Token {
        ctoken,
        endpoint: ContinuationEndpoint::Browse,
        visitor_data: None,
        authenticated,
    };
    Cursor(serde_json::to_string(&token).unwrap_or_default())
}

/// An empty paginator positioned at `cursor`, whose `next()` fetches the page it points to.
pub(crate) fn paginator<T: DeserializeOwned>(cursor: &Cursor) -> Result<Paginator<T>, Error> {
    let token: Token = serde_json::from_str(&cursor.0)
        .map_err(|_| Error::Parse("not a cursor of this connector".into()))?;
    let value = serde_json::json!({
        "count": null,
        "items": [],
        "ctoken": token.ctoken,
        "visitor_data": token.visitor_data,
        "endpoint": token.endpoint,
        "authenticated": token.authenticated,
    });
    serde_json::from_value(value).map_err(|e| Error::Parse(format!("cursor: {e}")))
}

/// A [`Page`] from a paginator. `first` says it is the first page, whose count is the total
/// YouTube announced; later pages only know their own length, which is no total.
pub(crate) fn page<T, U>(
    paginator: Paginator<T>,
    first: bool,
    map: impl FnMut(T) -> Option<U>,
) -> Page<U> {
    let next = next(&paginator);
    let total = if first { paginator.count } else { None };
    Page {
        items: paginator.items.into_iter().filter_map(map).collect(),
        next,
        total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustypipe::model::VideoItem;

    #[test]
    fn round_trips_a_continuation() {
        let p: Paginator<VideoItem> = serde_json::from_value(serde_json::json!({
            "count": 42, "items": [], "ctoken": "4qmFsgI", "visitor_data": "Cgt2",
            "endpoint": "search", "authenticated": true
        }))
        .unwrap();
        let cursor = next(&p).unwrap();
        let back: Paginator<VideoItem> = paginator(&cursor).unwrap();
        assert_eq!(back.ctoken.as_deref(), Some("4qmFsgI"));
        assert_eq!(back.endpoint, ContinuationEndpoint::Search);
        assert_eq!(back.visitor_data.as_deref(), Some("Cgt2"));
        assert!(back.authenticated);
    }

    #[test]
    fn last_page_has_no_cursor() {
        let p: Paginator<VideoItem> = Paginator::default();
        assert_eq!(next(&p), None);
    }

    #[test]
    fn rejects_foreign_cursors() {
        assert!(paginator::<VideoItem>(&Cursor("page=2".into())).is_err());
    }
}
