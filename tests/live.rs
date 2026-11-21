// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tests against live YouTube, anonymous. Ignored by default (network, and YouTube changes):
//!
//! ```bash
//! cargo nextest run --run-ignored only
//! ```
//!
//! The non-ignored tests here need no network: they check what an anonymous connector refuses.

use flaneur_connector::{
    ChannelTab, CommentSort, Connector, Error, FeedKind, SearchFilters, SearchResult, Target,
};
use flaneur_connector_youtube::{Config, YouTube};
use flaneur_model::{ItemKind, ItemRef, PlaylistRef, Reaction, ServiceId, StreamKind};
use flaneur_vm::{Selection, SelectionPrefs, select_streams};
use url::Url;

fn yt() -> YouTube {
    YouTube::new(Config::default()).unwrap()
}

fn video(id: &str) -> ItemRef {
    ItemRef::new(ServiceId::new("youtube"), id)
}

#[tokio::test]
async fn anonymous_connector_refuses_account_operations() {
    let yt = yt();
    let rick = video("dQw4w9WgXcQ");
    assert!(matches!(
        yt.feeds().unwrap().feed(FeedKind::Home, None).await,
        Err(Error::SignInRequired)
    ));
    assert!(matches!(
        yt.feeds().unwrap().feed(FeedKind::Trending, None).await,
        Err(Error::Unsupported)
    ));
    assert!(matches!(
        yt.library().unwrap().history(None).await,
        Err(Error::SignInRequired)
    ));
    assert!(matches!(
        yt.reactions().unwrap().react(&rick, Reaction::Like).await,
        Err(Error::SignInRequired)
    ));
    assert!(matches!(
        yt.comments().unwrap().post(&rick, "hi", None).await,
        Err(Error::SignInRequired)
    ));
}

#[tokio::test]
#[ignore = "live YouTube"]
async fn search_and_suggestions() {
    let yt = yt();
    let search = yt.search().unwrap();
    let page = search
        .search("rust programming", &SearchFilters::default(), None)
        .await
        .unwrap();
    assert!(page.items.len() > 5);
    let Some(next) = &page.next else {
        panic!("a second page")
    };
    let more = search
        .search("rust programming", &SearchFilters::default(), Some(next))
        .await
        .unwrap();
    assert_ne!(more.items.len(), 0);
    assert_ne!(search.suggestions("rust prog").await.unwrap().len(), 0);
}

#[tokio::test]
#[ignore = "live YouTube"]
async fn resolves_playable_streams() {
    let yt = yt();
    let p = yt.player().resolve(&video("dQw4w9WgXcQ")).await.unwrap();
    assert_eq!(p.item.kind, ItemKind::Video);
    assert!(p.streams.iter().any(|s| s.kind == StreamKind::Video));
    assert!(p.streams.iter().any(|s| s.kind == StreamKind::Audio));
    assert!(p.expires_at.is_some());
    assert!(matches!(
        select_streams(&p, &SelectionPrefs::default()),
        Some(Selection::Split { .. })
    ));
}

#[tokio::test]
#[ignore = "live YouTube"]
async fn a_live_plays_through_hls() {
    let yt = yt();
    let lives = yt
        .search()
        .unwrap()
        .search(
            "news live",
            &SearchFilters {
                kind: Some(ItemKind::Live),
                ..SearchFilters::default()
            },
            None,
        )
        .await
        .unwrap();
    let Some(SearchResult::Item(live)) = lives.items.first() else {
        panic!("no live found");
    };
    assert_eq!(live.kind, ItemKind::Live);
    let p = yt.player().resolve(&live.r#ref).await.unwrap();
    assert_eq!(p.item.kind, ItemKind::Live);
    assert!(p.hls_manifest.is_some());
}

#[tokio::test]
#[ignore = "live YouTube"]
async fn comments_and_replies() {
    let yt = yt();
    let comments = yt.comments().unwrap();
    let rick = video("dQw4w9WgXcQ");
    let top = comments
        .comments(&rick, CommentSort::Top, None)
        .await
        .unwrap();
    assert_ne!(top.items.len(), 0);
    assert!(top.total.is_some());
    let token = top
        .items
        .iter()
        .find_map(|c| c.replies_token.clone())
        .unwrap();
    let replies = comments.replies(&rick, &token, None).await.unwrap();
    assert_ne!(replies.items.len(), 0);
    let next = top.next.unwrap();
    assert_ne!(
        comments
            .comments(&rick, CommentSort::Top, Some(&next))
            .await
            .unwrap()
            .items
            .len(),
        0
    );
    let newest = comments
        .comments(&rick, CommentSort::Newest, None)
        .await
        .unwrap();
    assert_ne!(newest.items.len(), 0);
}

#[tokio::test]
#[ignore = "live YouTube"]
async fn related_items() {
    let yt = yt();
    let page = yt
        .player()
        .related(&video("dQw4w9WgXcQ"), None)
        .await
        .unwrap();
    assert!(page.items.len() > 5);
    assert!(page.items.iter().any(|i| i.views.is_some()));
}

#[tokio::test]
#[ignore = "live YouTube"]
async fn channel_from_a_handle_url() {
    let yt = yt();
    let Some(Target::Channel(channel)) =
        yt.parse_url(&Url::parse("https://www.youtube.com/@LinusTechTips").unwrap())
    else {
        panic!("a channel");
    };
    let channels = yt.channels().unwrap();
    let header = channels.channel(&channel).await.unwrap();
    assert_eq!(header.summary.r#ref.id, "UCXuqSBlHAE6Xw-yeJA0Tunw");
    assert!(header.subscribers.unwrap() > 1_000_000);
    let videos = channels
        .items(&channel, ChannelTab::Videos, None)
        .await
        .unwrap();
    assert!(videos.items.iter().all(|i| i.kind != ItemKind::Short));
    assert!(videos.items.iter().any(|i| i.duration.is_some()));
    let more = channels
        .items(&channel, ChannelTab::Videos, videos.next.as_ref())
        .await
        .unwrap();
    assert_ne!(more.items.len(), 0);
    let shorts = channels
        .items(&channel, ChannelTab::Shorts, None)
        .await
        .unwrap();
    assert_ne!(shorts.items.len(), 0);
    assert!(shorts.items.iter().all(|i| i.kind == ItemKind::Short));
    let lives = channels
        .items(&channel, ChannelTab::Lives, None)
        .await
        .unwrap();
    assert_ne!(lives.items.len(), 0);
    assert_ne!(
        channels
            .playlists(&channel, None)
            .await
            .unwrap()
            .items
            .len(),
        0
    );
}

#[tokio::test]
#[ignore = "live YouTube"]
async fn public_playlist() {
    let yt = yt();
    let homelab = PlaylistRef::new(
        ServiceId::new("youtube"),
        "PL2_OBreMn7FrsiSW0VDZjdq0xqUKkZYHT",
    );
    let page = yt
        .library()
        .unwrap()
        .playlist_items(&homelab, None)
        .await
        .unwrap();
    assert_ne!(page.items.len(), 0);
    assert!(page.total.unwrap() > 50);
}
