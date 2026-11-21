// SPDX-License-Identifier: AGPL-3.0-or-later

//! Playback reporting through the playback tracking URLs of the player answer.
//!
//! The web player tells YouTube what it plays with two GET requests on URLs the `/player` answer
//! gives in `playbackTracking`: `videostatsPlaybackUrl` once at start (adds the video to the watch
//! history) and `videostatsWatchtimeUrl` while playing (the watched segments, hence the resume
//! position). RustyPipe drops `playbackTracking` from its model, so the connector reads it from a
//! raw, authenticated player request and keeps it, with the playback nonce of this playback, in
//! the opaque [`Playable::tracking`](flaneur_model::Playable::tracking) token.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

use flaneur_connector::{PlaybackEvent, PlaybackReport};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

/// What a playback needs to report itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Tracking {
    pub(crate) video_id: String,
    pub(crate) playback: String,
    pub(crate) watchtime: Option<String>,
    /// Client playback nonce, the same for every ping of one playback.
    pub(crate) cpn: String,
    /// Duration in seconds.
    pub(crate) length: u32,
}

impl Tracking {
    /// Reads `playbackTracking` from a raw `/player` answer.
    pub(crate) fn from_player(response: &Value, video_id: &str) -> Option<Self> {
        let url = |key: &str| {
            response
                .pointer(&format!("/playbackTracking/{key}/baseUrl"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        Some(Self {
            video_id: video_id.to_owned(),
            playback: url("videostatsPlaybackUrl")?,
            watchtime: url("videostatsWatchtimeUrl"),
            cpn: cpn(),
            length: response
                .pointer("/videoDetails/lengthSeconds")
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        })
    }

    pub(crate) fn encode(&self) -> Option<String> {
        serde_json::to_string(self).ok()
    }

    pub(crate) fn decode(token: &str) -> Option<Self> {
        serde_json::from_str(token).ok()
    }

    /// The URL to GET for this report; `None` when the answer had no URL for it.
    pub(crate) fn ping(&self, report: &PlaybackReport) -> Option<Url> {
        let position = if self.length > 0 {
            report.position.min(self.length)
        } else {
            report.position
        };
        let (base, state) = match report.event {
            PlaybackEvent::Started => (&self.playback, None),
            PlaybackEvent::Progress => (self.watchtime.as_ref()?, Some("playing")),
            PlaybackEvent::Stopped | PlaybackEvent::Ended => {
                (self.watchtime.as_ref()?, Some("paused"))
            }
        };
        let mut url = Url::parse(base).ok()?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("ver", "2")
                .append_pair("cpn", &self.cpn)
                .append_pair("cmt", &position.to_string())
                .append_pair("len", &self.length.to_string());
            if let Some(state) = state {
                let start = position.saturating_sub(report.watched_since_last);
                q.append_pair("st", &start.to_string())
                    .append_pair("et", &position.to_string())
                    .append_pair("state", state);
            }
        }
        Some(url)
    }
}

/// A client playback nonce: 16 characters of YouTube's base64url alphabet.
fn cpn() -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let state = RandomState::new();
    (0..16u8)
        .map(|i| {
            let mut h = state.build_hasher();
            h.write_u8(i);
            // Masked to 0..64: the cast cannot truncate.
            #[allow(clippy::cast_possible_truncation)]
            let index = (h.finish() & 63) as usize;
            char::from(ALPHABET[index])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flaneur_model::{ItemRef, ServiceId};
    use serde_json::json;

    fn player() -> Value {
        json!({
            "videoDetails": { "videoId": "dQw4w9WgXcQ", "lengthSeconds": "213" },
            "playbackTracking": {
                "videostatsPlaybackUrl": {
                    "baseUrl": "https://s.youtube.com/api/stats/playback?cl=1&docid=dQw4w9WgXcQ&ei=abc"
                },
                "videostatsWatchtimeUrl": {
                    "baseUrl": "https://s.youtube.com/api/stats/watchtime?cl=1&docid=dQw4w9WgXcQ&ei=abc"
                }
            }
        })
    }

    fn report(event: PlaybackEvent, position: u32, watched: u32) -> PlaybackReport {
        PlaybackReport {
            item: ItemRef::new(ServiceId::new("youtube"), "dQw4w9WgXcQ"),
            event,
            position,
            watched_since_last: watched,
            tracking: None,
        }
    }

    fn params(url: &Url) -> Vec<(String, String)> {
        url.query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect()
    }

    #[test]
    fn reads_tracking_urls_and_round_trips() {
        let t = Tracking::from_player(&player(), "dQw4w9WgXcQ").unwrap();
        assert_eq!(t.length, 213);
        assert_eq!(t.cpn.len(), 16);
        assert_eq!(Tracking::decode(&t.encode().unwrap()), Some(t));
        assert_eq!(Tracking::from_player(&json!({}), "x"), None);
    }

    #[test]
    fn start_pings_the_playback_url() {
        let t = Tracking::from_player(&player(), "dQw4w9WgXcQ").unwrap();
        let url = t.ping(&report(PlaybackEvent::Started, 0, 0)).unwrap();
        assert_eq!(url.path(), "/api/stats/playback");
        let p = params(&url);
        assert!(p.contains(&("docid".into(), "dQw4w9WgXcQ".into())));
        assert!(p.contains(&("ver".into(), "2".into())));
        assert!(p.contains(&("cpn".into(), t.cpn.clone())));
        assert!(p.contains(&("cmt".into(), "0".into())));
        assert!(p.contains(&("len".into(), "213".into())));
        assert!(!p.iter().any(|(k, _)| k == "st"));
    }

    #[test]
    fn progress_reports_the_watched_segment() {
        let t = Tracking::from_player(&player(), "dQw4w9WgXcQ").unwrap();
        let url = t.ping(&report(PlaybackEvent::Progress, 90, 30)).unwrap();
        assert_eq!(url.path(), "/api/stats/watchtime");
        let p = params(&url);
        for (k, v) in [
            ("cmt", "90"),
            ("st", "60"),
            ("et", "90"),
            ("state", "playing"),
        ] {
            assert!(p.contains(&(k.into(), v.into())), "{k}={v} in {p:?}");
        }
        let stop = t.ping(&report(PlaybackEvent::Stopped, 95, 5)).unwrap();
        assert!(params(&stop).contains(&("state".into(), "paused".into())));
    }

    #[test]
    fn no_watchtime_url_no_progress_ping() {
        let mut v = player();
        v["playbackTracking"]
            .as_object_mut()
            .unwrap()
            .remove("videostatsWatchtimeUrl");
        let t = Tracking::from_player(&v, "dQw4w9WgXcQ").unwrap();
        assert!(t.ping(&report(PlaybackEvent::Progress, 10, 10)).is_none());
        assert!(t.ping(&report(PlaybackEvent::Started, 0, 0)).is_some());
    }
}
