// SPDX-License-Identifier: AGPL-3.0-or-later

//! What needs the session: history, playlists, subscriptions, likes.
//!
//! Reads go through RustyPipe's `userdata` API. Writes are the InnerTube calls of
//! [`actions`](crate::actions) and the playback tracking pings of
//! [`tracking`](crate::tracking).

use async_trait::async_trait;
use flaneur_connector::{Error, Library, PlaybackReport, Reactions, Result, Subscriptions};
use flaneur_model::{
    ChannelRef, ChannelSummary, Cursor, Item, ItemRef, Page, Playlist, PlaylistRef, Reaction,
};
use rustypipe::client::ClientType;
use rustypipe::model::paginator::Paginator;
use rustypipe::model::{ChannelItem, HistoryItem, PlaylistItem, VideoItem};

use crate::connector::{YouTube, video_id};
use crate::tracking::Tracking;
use crate::{actions, cursor, error, map};

/// Id of the Watch Later playlist.
const WATCH_LATER: &str = "WL";
/// Id of the Liked videos playlist.
const LIKED: &str = "LL";

fn own_playlist(id: &str, title: &str) -> Playlist {
    Playlist {
        r#ref: map::playlist_ref(id),
        title: title.to_owned(),
        channel: None,
        item_count: None,
        thumbnails: Vec::new(),
    }
}

fn playlist_id(playlist: &PlaylistRef) -> Result<&str> {
    if playlist.service.0 == map::SERVICE && !playlist.id.is_empty() {
        Ok(&playlist.id)
    } else {
        Err(Error::NotFound)
    }
}

#[async_trait]
impl Library for YouTube {
    async fn history(&self, cursor: Option<&Cursor>) -> Result<Page<Item>> {
        let q = self.authed()?;
        let item = |h: HistoryItem<VideoItem>| Some(map::video_item(h.item));
        if let Some(c) = cursor {
            let paginator: Paginator<HistoryItem<VideoItem>> = cursor::paginator(c)?;
            return match paginator.next(&q).await.map_err(|e| self.err(e))? {
                Some(p) => Ok(cursor::page(p, false, item)),
                None => Ok(Page::last(Vec::new())),
            };
        }
        let history = q.history().await.map_err(|e| self.err(e))?;
        Ok(cursor::page(history, true, item))
    }

    /// Pings the playback tracking URLs of the item with the session cookie: `Started` adds the
    /// item to the watch history, the other events report the watched segment.
    async fn report_playback(&self, report: &PlaybackReport) -> Result<()> {
        let active = self.active()?;
        let id = video_id(&report.item)?;
        let tracking = match report
            .tracking
            .as_deref()
            .and_then(Tracking::decode)
            .filter(|t| t.video_id == id)
        {
            Some(t) => t,
            None => self.tracking(id).await.ok_or_else(|| {
                Error::Unavailable("YouTube gave no playback tracking for this item".into())
            })?,
        };
        let Some(url) = tracking.ping(report) else {
            return Ok(());
        };
        let user_agent = self.query().user_agent(ClientType::Desktop).into_owned();
        let response = self
            .http
            .get(url)
            .header(reqwest::header::COOKIE, &active.cookie_header)
            .header(reqwest::header::USER_AGENT, user_agent)
            .header(
                reqwest::header::REFERER,
                format!("https://www.youtube.com/watch?v={id}"),
            )
            .send()
            .await
            .map_err(|e| Error::Network(active.redactor.apply(&e.without_url().to_string())))?;
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(error::status_error(
                status.as_u16(),
                status
                    .canonical_reason()
                    .unwrap_or("playback report refused"),
            ))
        }
    }

    /// Watch later and Liked videos first, then the playlists of the library.
    async fn playlists(&self, cursor: Option<&Cursor>) -> Result<Page<Playlist>> {
        let q = self.authed()?;
        if let Some(c) = cursor {
            return self
                .next_page::<PlaylistItem, _>(c, |p| Some(map::playlist_item(p)))
                .await;
        }
        let saved = q.saved_playlists().await.map_err(|e| self.err(e))?;
        let mut page = cursor::page(saved, true, |p| Some(map::playlist_item(p)));
        page.items.splice(
            0..0,
            [
                own_playlist(WATCH_LATER, "Watch later"),
                own_playlist(LIKED, "Liked videos"),
            ],
        );
        page.total = page.total.map(|n| n + 2);
        Ok(page)
    }

    fn watch_later(&self) -> Option<PlaylistRef> {
        Some(map::playlist_ref(WATCH_LATER))
    }

    async fn playlist_items(
        &self,
        playlist: &PlaylistRef,
        cursor: Option<&Cursor>,
    ) -> Result<Page<Item>> {
        let id = playlist_id(playlist)?;
        if let Some(c) = cursor {
            return self
                .next_page::<VideoItem, _>(c, |v| Some(map::video_item(v)))
                .await;
        }
        let result = match id {
            WATCH_LATER => self.authed()?.watch_later().await,
            LIKED => self.authed()?.liked_videos().await,
            // Public playlists work anonymously; private ones need the session.
            id if self.is_signed_in() => self.authed()?.playlist(id).await,
            id => self.query().playlist(id).await,
        };
        let list = result.map_err(|e| self.err(e))?;
        let total = list.video_count;
        let mut page = cursor::page(list.videos, true, |v| Some(map::video_item(v)));
        page.total = Some(total);
        Ok(page)
    }

    async fn add_to_playlist(&self, playlist: &PlaylistRef, item: &ItemRef) -> Result<()> {
        let action = actions::edit_playlist(playlist_id(playlist)?, video_id(item)?, true);
        self.send(&action).await.map(|_| ())
    }

    async fn remove_from_playlist(&self, playlist: &PlaylistRef, item: &ItemRef) -> Result<()> {
        let action = actions::edit_playlist(playlist_id(playlist)?, video_id(item)?, false);
        self.send(&action).await.map(|_| ())
    }
}

#[async_trait]
impl Subscriptions for YouTube {
    async fn list(&self, cursor: Option<&Cursor>) -> Result<Page<ChannelSummary>> {
        let q = self.authed()?;
        let channel = |c: ChannelItem| Some(map::channel_item(c));
        if let Some(c) = cursor {
            return self.next_page::<ChannelItem, _>(c, channel).await;
        }
        let subscriptions = q.subscriptions().await.map_err(|e| self.err(e))?;
        Ok(cursor::page(subscriptions, true, channel))
    }

    async fn set_subscribed(&self, channel: &ChannelRef, subscribed: bool) -> Result<()> {
        self.active()?;
        let id = self.channel_id(channel).await?;
        self.send(&actions::subscribe(&id, subscribed))
            .await
            .map(|_| ())
    }
}

#[async_trait]
impl Reactions for YouTube {
    async fn react(&self, item: &ItemRef, reaction: Reaction) -> Result<()> {
        let action = actions::rate(video_id(item)?, reaction);
        self.send(&action).await.map(|_| ())
    }
}
