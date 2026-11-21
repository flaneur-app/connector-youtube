// SPDX-License-Identifier: AGPL-3.0-or-later

//! InnerTube writes RustyPipe has no API for: likes, subscriptions, playlist edits, comments.
//!
//! Each is one authenticated `POST /youtubei/v1/<endpoint>` sent through RustyPipe's `raw()`,
//! which adds the session cookie and the `SAPISIDHASH` authorization like the web client does.
//! The bodies are those of the web client: the `edit_playlist` one is the very command YouTube
//! embeds in its own pages (the "Watch later" button of every lockup), the others follow
//! YouTube.js. They are built here as plain values so their exact shape is tested without a
//! network.

use flaneur_connector::Error;
use flaneur_model::Reaction;
use serde_json::{Value, json};

/// One InnerTube write.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Action {
    /// Path under `/youtubei/v1/`.
    pub(crate) endpoint: &'static str,
    /// JSON body; RustyPipe adds the `context`.
    pub(crate) body: Value,
}

/// Like, dislike, or remove the rating.
pub(crate) fn rate(video_id: &str, reaction: Reaction) -> Action {
    let endpoint = match reaction {
        Reaction::Like => "like/like",
        Reaction::Dislike => "like/dislike",
        Reaction::None => "like/removelike",
    };
    Action {
        endpoint,
        body: json!({ "target": { "videoId": video_id } }),
    }
}

/// Subscribe to, or unsubscribe from, a channel. `params` are the ones the web client sends.
pub(crate) fn subscribe(channel_id: &str, subscribed: bool) -> Action {
    let (endpoint, params) = if subscribed {
        ("subscription/subscribe", "EgIIAhgA")
    } else {
        ("subscription/unsubscribe", "CgIIAhgA")
    };
    Action {
        endpoint,
        body: json!({ "channelIds": [channel_id], "params": params }),
    }
}

/// Add a video to, or remove it from, a playlist of the user (`WL` is Watch Later).
pub(crate) fn edit_playlist(playlist_id: &str, video_id: &str, add: bool) -> Action {
    let action = if add {
        json!({ "action": "ACTION_ADD_VIDEO", "addedVideoId": video_id })
    } else {
        json!({ "action": "ACTION_REMOVE_VIDEO_BY_VIDEO_ID", "removedVideoId": video_id })
    };
    Action {
        endpoint: "browse/edit_playlist",
        body: json!({ "playlistId": playlist_id, "actions": [action] }),
    }
}

/// Post a public top-level comment.
pub(crate) fn create_comment(video_id: &str, text: &str) -> Action {
    Action {
        endpoint: "comment/create_comment",
        body: json!({
            "createCommentParams": create_comment_params(video_id),
            "commentText": text,
        }),
    }
}

/// `createCommentParams`: the protobuf message `{2: video_id, 5: {1: 0}, 10: 7}`, in standard
/// base64, then percent-encoded, exactly as YouTube.js sends it.
pub(crate) fn create_comment_params(video_id: &str) -> String {
    let id = video_id.as_bytes();
    let mut pb = Vec::with_capacity(id.len() + 8);
    pb.push(0x12); // field 2, length-delimited
    // Video ids are 11 bytes: one length byte is always enough.
    pb.push(u8::try_from(id.len()).unwrap_or(u8::MAX));
    pb.extend_from_slice(id);
    pb.extend_from_slice(&[0x2a, 0x02, 0x08, 0x00]); // field 5: { field 1: 0 }
    pb.extend_from_slice(&[0x50, 0x07]); // field 10: 7
    data_encoding::BASE64
        .encode(&pb)
        .replace('+', "%2B")
        .replace('/', "%2F")
        .replace('=', "%3D")
}

/// InnerTube answers a refused write with HTTP 200 and an `error` object.
pub(crate) fn check_response(response: &Value) -> Result<(), Error> {
    let Some(error) = response.get("error") else {
        return Ok(());
    };
    let code = error.get("code").and_then(Value::as_u64).unwrap_or(0);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("refused")
        .to_owned();
    Err(match code {
        401 | 403 => Error::SignInRequired,
        404 => Error::NotFound,
        429 => Error::RateLimited { retry_after: None },
        _ => Error::Service(format!("YouTube refused the change: {message}")),
    })
}

/// The id of a comment YouTube just created, wherever the answer puts it.
pub(crate) fn created_comment_id(response: &Value) -> Option<String> {
    match response {
        Value::Object(o) => o
            .get("commentId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| o.values().find_map(created_comment_id)),
        Value::Array(a) => a.iter().find_map(created_comment_id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rating_bodies() {
        let like = rate("dQw4w9WgXcQ", Reaction::Like);
        assert_eq!(like.endpoint, "like/like");
        assert_eq!(like.body, json!({ "target": { "videoId": "dQw4w9WgXcQ" } }));
        assert_eq!(
            rate("dQw4w9WgXcQ", Reaction::Dislike).endpoint,
            "like/dislike"
        );
        assert_eq!(
            rate("dQw4w9WgXcQ", Reaction::None).endpoint,
            "like/removelike"
        );
    }

    #[test]
    fn subscription_bodies() {
        let sub = subscribe("UCXuqSBlHAE6Xw-yeJA0Tunw", true);
        assert_eq!(sub.endpoint, "subscription/subscribe");
        assert_eq!(
            sub.body,
            json!({ "channelIds": ["UCXuqSBlHAE6Xw-yeJA0Tunw"], "params": "EgIIAhgA" })
        );
        let unsub = subscribe("UCXuqSBlHAE6Xw-yeJA0Tunw", false);
        assert_eq!(unsub.endpoint, "subscription/unsubscribe");
        assert_eq!(unsub.body["params"], "CgIIAhgA");
    }

    #[test]
    fn playlist_bodies_match_youtube_own_commands() {
        // The "Watch later" button of a lockup in a live channel page (2026-11-21) carries
        // `playlistEditEndpoint: {playlistId: "WL", actions: [{addedVideoId, action}]}`.
        let add = edit_playlist("WL", "f0KK_4AaqLQ", true);
        assert_eq!(add.endpoint, "browse/edit_playlist");
        assert_eq!(
            add.body,
            json!({
                "playlistId": "WL",
                "actions": [{ "action": "ACTION_ADD_VIDEO", "addedVideoId": "f0KK_4AaqLQ" }]
            })
        );
        let remove = edit_playlist("WL", "f0KK_4AaqLQ", false);
        assert_eq!(
            remove.body["actions"][0],
            json!({ "action": "ACTION_REMOVE_VIDEO_BY_VIDEO_ID", "removedVideoId": "f0KK_4AaqLQ" })
        );
    }

    #[test]
    fn comment_params_are_youtube_js_protobuf() {
        // 12 0b "dQw4w9WgXcQ" 2a 02 08 00 50 07, base64 then percent-encoded.
        assert_eq!(
            create_comment_params("dQw4w9WgXcQ"),
            "EgtkUXc0dzlXZ1hjUSoCCABQBw%3D%3D"
        );
        let c = create_comment("dQw4w9WgXcQ", "Nice");
        assert_eq!(c.endpoint, "comment/create_comment");
        assert_eq!(c.body["commentText"], "Nice");
        assert_eq!(
            c.body["createCommentParams"],
            "EgtkUXc0dzlXZ1hjUSoCCABQBw%3D%3D"
        );
    }

    #[test]
    fn refused_writes_are_errors() {
        assert!(check_response(&json!({ "actions": [] })).is_ok());
        assert!(matches!(
            check_response(&json!({ "error": { "code": 401, "message": "x" } })),
            Err(Error::SignInRequired)
        ));
        assert!(matches!(
            check_response(&json!({ "error": { "code": 400, "message": "Invalid" } })),
            Err(Error::Service(_))
        ));
    }

    #[test]
    fn finds_the_new_comment_id() {
        let v = json!({ "actions": [{ "createCommentAction": { "contents": {
            "commentThreadRenderer": { "comment": { "commentRenderer": { "commentId": "Ugx1" } } }
        } } }] });
        assert_eq!(created_comment_id(&v).as_deref(), Some("Ugx1"));
    }
}
