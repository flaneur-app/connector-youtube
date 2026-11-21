// SPDX-License-Identifier: AGPL-3.0-or-later

//! Search, channel pages, feeds and comments.

use async_trait::async_trait;
use flaneur_connector::{
    ChannelTab, Channels, CommentSort, Comments, Error, FeedKind, Feeds, Result, Search,
    SearchFilters, SearchResult,
};
use flaneur_model::{
    Channel, ChannelRef, Comment, CommentAuthor, Cursor, Item, ItemKind, ItemRef, Page, Playlist,
    Precision, Published, Timestamp,
};
use rustypipe::client::ClientType;
use rustypipe::model::paginator::Paginator;
use rustypipe::model::{PlaylistItem, UrlTarget, VideoItem, YouTubeItem};
use rustypipe::param::ChannelVideoTab;
use rustypipe::param::search_filter::{Feature, ItemType, Length, SearchFilter, UploadDate};
use serde_json::{Value, json};

use crate::connector::{YouTube, video_id};
use crate::{actions, cursor, map, raw, text, urls};

/// The coarsest YouTube upload-date filter that still includes everything newer than `age`
/// seconds; `None` when even "this year" would cut items off.
pub(crate) fn upload_date(age: Timestamp) -> Option<UploadDate> {
    const DAY: Timestamp = 86_400;
    match age {
        a if a <= 3_600 => Some(UploadDate::Hour),
        a if a <= DAY => Some(UploadDate::Day),
        a if a <= 7 * DAY => Some(UploadDate::Week),
        a if a <= 31 * DAY => Some(UploadDate::Month),
        a if a <= 365 * DAY => Some(UploadDate::Year),
        _ => None,
    }
}

fn search_filter(filters: &SearchFilters, now: Timestamp) -> SearchFilter {
    let mut f = SearchFilter::new();
    if let Some(kind) = filters.kind {
        f = f.item_type(ItemType::Video);
        match kind {
            ItemKind::Live => f = f.feature(Feature::IsLive),
            ItemKind::Short => f = f.length(Length::Short),
            ItemKind::Video | ItemKind::Premiere | ItemKind::Upcoming => {}
        }
    }
    if let Some(after) = filters.published_after {
        f = f.date_opt(upload_date(now - after));
    }
    f
}

/// Keeps what the filters ask for; YouTube's own filters are coarser (a "video" filter still
/// returns Shorts and lives).
fn search_result(i: YouTubeItem, filters: &SearchFilters) -> Option<SearchResult> {
    match i {
        YouTubeItem::Video(v) => {
            let item = map::video_item(v);
            let kind_ok = filters.kind.is_none_or(|k| match k {
                ItemKind::Premiere | ItemKind::Upcoming => {
                    matches!(item.kind, ItemKind::Premiere | ItemKind::Upcoming)
                }
                k => item.kind == k,
            });
            let date_ok = match (filters.published_after, item.published) {
                (Some(after), Some(p)) => p.at >= after,
                _ => true,
            };
            (kind_ok && date_ok).then(|| SearchResult::Item(Box::new(item)))
        }
        YouTubeItem::Channel(c) => filters
            .kind
            .is_none()
            .then(|| SearchResult::Channel(map::channel_item(c))),
        YouTubeItem::Playlist(p) => filters
            .kind
            .is_none()
            .then(|| SearchResult::Playlist(map::playlist_item(p))),
    }
}

#[async_trait]
impl Search for YouTube {
    async fn search(
        &self,
        query: &str,
        filters: &SearchFilters,
        cursor: Option<&Cursor>,
    ) -> Result<Page<SearchResult>> {
        if let Some(c) = cursor {
            return self
                .next_page::<YouTubeItem, _>(c, |i| search_result(i, filters))
                .await;
        }
        let filter = search_filter(filters, text::now());
        let result = self
            .query()
            .search_filter::<YouTubeItem, _>(query, &filter)
            .await
            .map_err(|e| self.err(e))?;
        Ok(cursor::page(result.items, true, |i| {
            search_result(i, filters)
        }))
    }

    async fn suggestions(&self, prefix: &str) -> Result<Vec<String>> {
        if prefix.trim().is_empty() {
            return Ok(Vec::new());
        }
        self.query()
            .search_suggestion(prefix)
            .await
            .map_err(|e| self.err(e))
    }
}

impl YouTube {
    /// The `UC…` id of a channel reference; handles (`@name`) and legacy paths (`c/name`,
    /// `user/name`) from [`parse_url`](flaneur_connector::Connector::parse_url) are resolved.
    pub(crate) async fn channel_id(&self, channel: &ChannelRef) -> Result<String> {
        if channel.service.0 != map::SERVICE {
            return Err(Error::NotFound);
        }
        let id = channel.id.as_str();
        if urls::is_channel_id(id) {
            return Ok(id.to_owned());
        }
        if !(id.starts_with('@') || id.starts_with("c/") || id.starts_with("user/")) {
            return Err(Error::NotFound);
        }
        match self
            .query()
            .resolve_url(format!("https://www.youtube.com/{id}"), false)
            .await
            .map_err(|e| self.err(e))?
        {
            UrlTarget::Channel { id } => Ok(id),
            _ => Err(Error::NotFound),
        }
    }
}

fn video_tab(tab: ChannelTab) -> ChannelVideoTab {
    match tab {
        ChannelTab::Videos => ChannelVideoTab::Videos,
        ChannelTab::Shorts => ChannelVideoTab::Shorts,
        ChannelTab::Lives => ChannelVideoTab::Live,
    }
}

#[async_trait]
impl Channels for YouTube {
    async fn channel(&self, channel: &ChannelRef) -> Result<Channel> {
        let id = self.channel_id(channel).await?;
        let page = self
            .query()
            .channel_videos(&id)
            .await
            .map_err(|e| self.err(e))?;
        Ok(map::channel_header(&page))
    }

    async fn items(
        &self,
        channel: &ChannelRef,
        tab: ChannelTab,
        cursor: Option<&Cursor>,
    ) -> Result<Page<Item>> {
        let tab = video_tab(tab);
        if let Some(c) = cursor {
            return self
                .next_page::<VideoItem, _>(c, |v| Some(map::video_item(v)))
                .await;
        }
        let id = self.channel_id(channel).await?;
        let page = self
            .query()
            .channel_videos_tab(&id, tab)
            .await
            .map_err(|e| self.err(e))?;
        Ok(cursor::page(page.content, true, |v| {
            Some(map::video_item(v))
        }))
    }

    async fn playlists(
        &self,
        channel: &ChannelRef,
        cursor: Option<&Cursor>,
    ) -> Result<Page<Playlist>> {
        if let Some(c) = cursor {
            return self
                .next_page::<PlaylistItem, _>(c, |p| Some(map::playlist_item(p)))
                .await;
        }
        let id = self.channel_id(channel).await?;
        let page = self
            .query()
            .channel_playlists(&id)
            .await
            .map_err(|e| self.err(e))?;
        Ok(cursor::page(page.content, true, |p| {
            Some(map::playlist_item(p))
        }))
    }
}

fn only_videos(i: YouTubeItem) -> Option<Item> {
    match i {
        YouTubeItem::Video(v) => Some(map::video_item(v)),
        YouTubeItem::Channel(_) | YouTubeItem::Playlist(_) => None,
    }
}

#[async_trait]
impl Feeds for YouTube {
    async fn feed(&self, kind: FeedKind, cursor: Option<&Cursor>) -> Result<Page<Item>> {
        match kind {
            FeedKind::Home => {
                let q = self.authed()?;
                if let Some(c) = cursor {
                    return self.next_page::<YouTubeItem, _>(c, only_videos).await;
                }
                let text = q
                    .raw(
                        ClientType::Desktop,
                        "browse",
                        &json!({ "browseId": "FEwhat_to_watch" }),
                    )
                    .await
                    .map_err(|e| self.err(e))?;
                let response: Value = serde_json::from_str(&text)
                    .map_err(|_| Error::Parse("home feed answered no JSON".into()))?;
                let home = raw::home(&response, text::now());
                Ok(Page {
                    items: home.items,
                    next: home.ctoken.map(|t| cursor::from_ctoken(t, true)),
                    total: None,
                })
            }
            FeedKind::Subscriptions => {
                let q = self.authed()?;
                if let Some(c) = cursor {
                    return self
                        .next_page::<VideoItem, _>(c, |v| Some(map::video_item(v)))
                        .await;
                }
                let feed = q.subscription_feed().await.map_err(|e| self.err(e))?;
                Ok(cursor::page(feed, true, |v| Some(map::video_item(v))))
            }
            FeedKind::Trending | FeedKind::Shorts => Err(Error::Unsupported),
        }
    }
}

impl YouTube {
    async fn comment_page(&self, cursor: &Cursor, video_id: &str) -> Result<Page<Comment>> {
        let paginator: Paginator<rustypipe::model::Comment> = cursor::paginator(cursor)?;
        match paginator
            .next(self.query())
            .await
            .map_err(|e| self.err(e))?
        {
            Some(p) => Ok(cursor::page(p, false, |c| Some(map::comment(c, video_id)))),
            None => Ok(Page::last(Vec::new())),
        }
    }
}

#[async_trait]
impl Comments for YouTube {
    async fn comments(
        &self,
        item: &ItemRef,
        sort: CommentSort,
        cursor: Option<&Cursor>,
    ) -> Result<Page<Comment>> {
        let id = video_id(item)?;
        if let Some(c) = cursor {
            return self.comment_page(c, id).await;
        }
        let q = self.query();
        let details = q.video_details(id).await.map_err(|e| self.err(e))?;
        let start = match sort {
            CommentSort::Top => details.top_comments,
            CommentSort::Newest => details.latest_comments,
        };
        let total = start.count;
        match start.next(&q).await.map_err(|e| self.err(e))? {
            Some(p) => {
                let mut page = cursor::page(p, false, |c| Some(map::comment(c, id)));
                page.total = total;
                Ok(page)
            }
            // Comments are turned off.
            None => Ok(Page {
                items: Vec::new(),
                next: None,
                total,
            }),
        }
    }

    async fn replies(
        &self,
        item: &ItemRef,
        replies_token: &str,
        cursor: Option<&Cursor>,
    ) -> Result<Page<Comment>> {
        let id = video_id(item)?;
        let start = Cursor(replies_token.to_owned());
        self.comment_page(cursor.unwrap_or(&start), id).await
    }

    /// A top-level comment, or, with the `reply_token` of a comment, a reply in its thread.
    async fn post(&self, item: &ItemRef, text: &str, reply_to: Option<&str>) -> Result<Comment> {
        let id = video_id(item)?;
        let action = match reply_to {
            Some(token) => actions::create_reply(token, text),
            None => actions::create_comment(id, text),
        };
        let response = self.send(&action).await?;
        let comment_id = actions::created_comment_id(&response).unwrap_or_default();
        let reply_token = match reply_to {
            Some(token) => Some(token.to_owned()),
            None => (!comment_id.is_empty()).then(|| actions::reply_params(id, &comment_id)),
        };
        Ok(Comment {
            id: comment_id,
            author: CommentAuthor {
                name: String::new(),
                channel: None,
                avatars: Vec::new(),
                is_publisher: false,
            },
            text: text.to_owned(),
            published: Some(Published {
                at: text::now(),
                precision: Precision::Exact,
            }),
            likes: Some(0),
            reply_count: Some(0),
            replies_token: None,
            reply_token,
            pinned: false,
            hearted: false,
            my_reaction: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_date_never_cuts_newer_items() {
        assert_eq!(upload_date(600), Some(UploadDate::Hour));
        assert_eq!(upload_date(2 * 86_400), Some(UploadDate::Week));
        assert_eq!(upload_date(400 * 86_400), None);
    }

    #[test]
    fn kind_filter_drops_other_kinds() {
        let items: Vec<YouTubeItem> = serde_json::from_str(
            &std::fs::read_to_string(format!(
                "{}/tests/fixtures/search_items.json",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap(),
        )
        .unwrap();
        let shorts = SearchFilters {
            kind: Some(ItemKind::Short),
            ..SearchFilters::default()
        };
        assert!(
            items
                .iter()
                .cloned()
                .filter_map(|i| search_result(i, &shorts))
                .all(|r| matches!(r, SearchResult::Item(i) if i.kind == ItemKind::Short))
        );
        let all = SearchFilters::default();
        let n = items
            .into_iter()
            .filter_map(|i| search_result(i, &all))
            .count();
        assert_eq!(n, 4);
    }
}
