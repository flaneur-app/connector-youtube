// SPDX-License-Identifier: AGPL-3.0-or-later

//! Stream resolution and related items.
//!
//! YouTube now serves the adaptive formats of its web and TV clients as SABR only (no URL).
//! Resolution therefore asks RustyPipe's default client order first, and when that fails on the
//! player script or gives no direct stream, falls back to the iOS client, which still returns
//! direct `googlevideo.com` URLs without a PO token, then to the Android VR client, which is the
//! one that gives the HLS manifest of a live.

use async_trait::async_trait;
use flaneur_connector::{Error, Player, Result};
use flaneur_model::{Cursor, Item, ItemRef, Page, Playable};
use rustypipe::client::{ClientType, RustyPipeQuery};
use rustypipe::model::{VideoItem, VideoPlayer};
use serde_json::{Value, json};

use crate::connector::{YouTube, video_id};
use crate::tracking::Tracking;
use crate::{cursor, error, map};

impl YouTube {
    /// The player answer of the first client that gives something playable.
    async fn player_answer(&self, q: &RustyPipeQuery, id: &str) -> Result<VideoPlayer> {
        let first = match q.player(id).await {
            Ok(p) if map::is_usable(&p) => return Ok(p),
            Ok(p) => {
                tracing::debug!(
                    "{:?} player of {id} has no direct stream; trying other clients",
                    p.client_type
                );
                Some(p)
            }
            Err(e) if error::worth_another_client(&e) => {
                tracing::debug!(
                    "default player of {id} failed ({}); trying other clients",
                    self.redactor().apply(&e.to_string())
                );
                None
            }
            Err(e) => return Err(self.err(e)),
        };

        let live = first.as_ref().is_some_and(|p| p.details.is_live);
        let fallbacks: [ClientType; 2] = if live {
            [ClientType::AndroidVr, ClientType::Ios]
        } else {
            [ClientType::Ios, ClientType::AndroidVr]
        };
        let tried = first.as_ref().map(|p| p.client_type);
        let mut last_error = None;
        for client in fallbacks {
            if Some(client) == tried {
                continue;
            }
            match q.player_from_client(id, client).await {
                Ok(p) if map::is_usable(&p) => {
                    tracing::debug!("playing {id} with the {client:?} client");
                    return Ok(p);
                }
                Ok(_) => {}
                Err(e) => last_error = Some(e),
            }
        }
        match last_error {
            Some(e) if first.is_none() => Err(self.err(e)),
            _ => Err(Error::Unavailable(
                "no client gave a direct stream (SABR only)".into(),
            )),
        }
    }

    /// Playback tracking URLs, for the history of the signed-in user; `None` when anonymous.
    ///
    /// The authenticated web player gives URLs tied to the session. When it gives none, the iOS
    /// player's (anonymous) URLs are used, still sent with the session cookie.
    pub(crate) async fn tracking(&self, id: &str) -> Option<Tracking> {
        let authed = self.authed().ok()?;
        let body = json!({ "videoId": id, "contentCheckOk": true, "racyCheckOk": true });
        for (q, client) in [
            (authed, ClientType::Desktop),
            (self.query(), ClientType::Ios),
        ] {
            match q.raw(client, "player", &body).await {
                Ok(text) => {
                    let tracking = serde_json::from_str::<Value>(&text)
                        .ok()
                        .and_then(|v| Tracking::from_player(&v, id));
                    if tracking.is_some() {
                        return tracking;
                    }
                }
                Err(e) => tracing::debug!(
                    "{client:?} player for tracking: {}",
                    self.redactor().apply(&e.to_string())
                ),
            }
        }
        None
    }
}

#[async_trait]
impl Player for YouTube {
    async fn resolve(&self, item: &ItemRef) -> Result<Playable> {
        let id = video_id(item)?;
        let q = self.query();
        let (details, player, tracking) = tokio::join!(
            q.video_details(id),
            self.player_answer(&q, id),
            self.tracking(id)
        );
        let player = player?;
        let details = details
            .map_err(|e| {
                tracing::debug!("details of {id}: {}", self.redactor().apply(&e.to_string()));
            })
            .ok();
        Ok(map::playable(
            details.as_ref(),
            &player,
            tracking.and_then(|t| t.encode()),
        ))
    }

    async fn related(&self, item: &ItemRef, cursor: Option<&Cursor>) -> Result<Page<Item>> {
        let id = video_id(item)?;
        if let Some(c) = cursor {
            return self
                .next_page::<VideoItem, _>(c, |v| Some(map::video_item(v)))
                .await;
        }
        let details = self
            .query()
            .video_details(id)
            .await
            .map_err(|e| self.err(e))?;
        Ok(cursor::page(details.recommended, true, |v| {
            Some(map::video_item(v))
        }))
    }
}
