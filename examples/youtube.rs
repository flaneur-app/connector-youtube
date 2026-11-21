// SPDX-License-Identifier: AGPL-3.0-or-later

//! Anonymous tour of the connector against live YouTube: search, stream resolution and selection,
//! comments, related items, a channel and a live.
//!
//! ```bash
//! cargo run --example youtube                      # defaults: "rust programming", dQw4w9WgXcQ
//! cargo run --example youtube -- "query" <video id or URL>
//! ```
//!
//! Stream URLs are printed by domain only: they embed the IP address that resolved them, and the
//! CDN node name hints at the network.

use std::error::Error;

use flaneur_connector::{ChannelTab, CommentSort, Connector, SearchFilters, SearchResult, Target};
use flaneur_connector_youtube::{Config, YouTube};
use flaneur_model::{ItemKind, ItemRef, Playable, ServiceId, Stream, VideoCodec};
use flaneur_vm::{Selection, SelectionPrefs, select_streams};
use url::Url;

/// `googlevideo.com` for `rr2---sn-abcd.googlevideo.com`.
fn domain(url: &Url) -> String {
    let host = url.host_str().unwrap_or("?");
    let labels: Vec<&str> = host.rsplitn(3, '.').collect();
    match labels.as_slice() {
        [tld, name, ..] => format!("{name}.{tld}"),
        _ => host.to_owned(),
    }
}

type Res<T = ()> = Result<T, Box<dyn Error>>;

fn describe(s: &Stream) -> String {
    let format = match (&s.video_codec, &s.audio) {
        (Some(v), _) => format!(
            "{v:?} {}p{}",
            s.height.unwrap_or(0),
            s.fps.map_or(String::new(), |f| format!(" {f}fps"))
        ),
        (None, Some(a)) => format!(
            "{} lang={:?} original={:?}",
            a.codec.as_deref().unwrap_or("?"),
            a.language,
            a.original
        ),
        (None, None) => String::new(),
    };
    format!(
        "{:?}/{:?} {format} {} kbit/s {} via {}",
        s.kind,
        s.container,
        s.bitrate.map_or(0, |b| b / 1000),
        s.size
            .map_or("size ?".into(), |b| format!("{} MB", b / 1_000_000)),
        domain(&s.url),
    )
}

/// Ranged GET past the first megabyte (YouTube serves that part even without a PO token).
async fn check(url: &Url, size: Option<u64>) -> String {
    let start = match size {
        Some(s) if s > 1_200_000 => 1_100_000,
        _ => 0,
    };
    let range = format!("bytes={start}-{}", start + 65_535);
    match reqwest::Client::new()
        .get(url.clone())
        .header("Range", range)
        .send()
        .await
    {
        Ok(r) => format!("HTTP {} at byte {start}", r.status()),
        Err(e) => format!("failed: {}", e.without_url()),
    }
}

async fn print_selection(playable: &Playable, label: &str, prefs: &SelectionPrefs) {
    match select_streams(playable, prefs) {
        Some(Selection::Split { video, audio }) => {
            println!("  {label}:");
            println!(
                "    video {} -> {}",
                describe(video),
                check(&video.url, video.size).await
            );
            println!(
                "    audio {} -> {}",
                describe(audio),
                check(&audio.url, audio.size).await
            );
        }
        Some(Selection::Single(s)) => {
            println!(
                "  {label}: single {} -> {}",
                describe(s),
                check(&s.url, s.size).await
            );
        }
        Some(Selection::Hls(url)) => {
            println!("  {label}: HLS via {}", domain(url));
        }
        None => println!("  {label}: nothing playable"),
    }
}

// A linear tour, one section after the other.
#[allow(clippy::too_many_lines)]
#[tokio::main]
async fn main() -> Res {
    let mut args = std::env::args().skip(1);
    let query = args.next().unwrap_or_else(|| "rust programming".into());
    let video = args.next().unwrap_or_else(|| "dQw4w9WgXcQ".into());

    let yt = YouTube::new(Config::default())?;
    let caps = yt.capabilities();
    println!(
        "== {} (signed in: {}) search={:?} channels={:?} comments={:?} home={:?} trending={:?}",
        yt.info().name,
        caps.signed_in,
        caps.search,
        caps.channels,
        caps.comments_read,
        caps.home,
        caps.trending
    );

    let search = yt.search().ok_or("no search")?;
    println!("\n== search {query:?}");
    let suggestions = search.suggestions(&query[..query.len().min(6)]).await?;
    println!(
        "suggestions: {:?}",
        &suggestions[..suggestions.len().min(5)]
    );
    let page = search
        .search(&query, &SearchFilters::default(), None)
        .await?;
    println!(
        "{} results on the first page, more: {}",
        page.items.len(),
        page.next.is_some()
    );
    for r in page.items.iter().take(6) {
        match r {
            SearchResult::Item(i) => println!(
                "  [{:?}] {} | {} | views={:?} | dur={:?} | published={:?}",
                i.kind,
                i.title,
                i.channel.as_ref().map_or("?", |c| c.name.as_str()),
                i.views,
                i.duration,
                i.published.map(|p| p.precision)
            ),
            SearchResult::Channel(c) => println!("  [channel] {} ({})", c.name, c.r#ref.id),
            SearchResult::Playlist(p) => {
                println!("  [playlist] {} ({:?} items)", p.title, p.item_count);
            }
        }
    }

    let item = match yt.parse_url(&video) {
        Some(Target::Item(r)) => r,
        _ => ItemRef::new(ServiceId::new("youtube"), video),
    };
    println!("\n== resolve {}", item.id);
    let playable = yt.player().resolve(&item).await?;
    let i = &playable.item;
    println!(
        "{} [{:?}] by {} | {:?}s | views={:?} likes={:?} comments={:?} | published={:?}",
        i.title,
        i.kind,
        i.channel.as_ref().map_or("?", |c| c.name.as_str()),
        i.duration,
        i.views,
        i.reactions.likes,
        i.reactions.comments,
        i.published
    );
    let count = |k| playable.streams.iter().filter(|s| s.kind == k).count();
    println!(
        "{} streams: {} video, {} audio, {} muxed | HLS: {} | expires in {:?} s",
        playable.streams.len(),
        count(flaneur_model::StreamKind::Video),
        count(flaneur_model::StreamKind::Audio),
        count(flaneur_model::StreamKind::Muxed),
        playable.hls_manifest.is_some(),
        playable.expires_at.map(|t| t - i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs())
        )
        .unwrap_or(0))
    );
    print_selection(
        &playable,
        "default prefs (1080p, AV1 > VP9 > H.264)",
        &SelectionPrefs::default(),
    )
    .await;
    print_selection(
        &playable,
        "GPU decodes H.264 only, 720p",
        &SelectionPrefs {
            max_height: 720,
            hardware_codecs: vec![VideoCodec::H264],
            ..SelectionPrefs::default()
        },
    )
    .await;
    print_selection(
        &playable,
        "audio mode",
        &SelectionPrefs {
            audio_only: true,
            ..SelectionPrefs::default()
        },
    )
    .await;

    let comments = yt.comments().ok_or("no comments")?;
    println!("\n== comments (top)");
    let page = comments.comments(&item, CommentSort::Top, None).await?;
    println!("total: {:?}", page.total);
    for c in page.items.iter().take(3) {
        println!(
            "  {} [{:?} | likes={:?} replies={:?}{}] {}",
            c.author.name,
            c.published.map(|p| p.precision),
            c.likes,
            c.reply_count,
            if c.pinned { " pinned" } else { "" },
            c.text
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(80)
                .collect::<String>()
        );
    }
    if let Some(token) = page.items.iter().find_map(|c| c.replies_token.clone()) {
        let replies = comments.replies(&item, &token, None).await?;
        println!("  first replies page: {} replies", replies.items.len());
    }
    let newest = comments.comments(&item, CommentSort::Newest, None).await?;
    println!("newest first: {} on the first page", newest.items.len());

    println!("\n== related");
    let related = yt.player().related(&item, None).await?;
    for r in related.items.iter().take(3) {
        println!(
            "  [{:?}] {} | views={:?} dur={:?} published={:?}",
            r.kind,
            r.title,
            r.views,
            r.duration,
            r.published.map(|p| p.precision)
        );
    }

    println!("\n== channel @LinusTechTips (from a URL)");
    let Some(Target::Channel(channel)) = yt.parse_url("https://www.youtube.com/@LinusTechTips")
    else {
        return Err("not a channel".into());
    };
    let channels = yt.channels().ok_or("no channels")?;
    let header = channels.channel(&channel).await?;
    println!(
        "{} ({}) {:?} subscribers={:?}",
        header.summary.name, header.summary.r#ref.id, header.handle, header.subscribers
    );
    for tab in [ChannelTab::Videos, ChannelTab::Shorts, ChannelTab::Lives] {
        let page = channels.items(&channel, tab, None).await?;
        let first = page.items.first();
        println!(
            "  {tab:?}: {} items, first: {:?} [{:?}] views={:?} dur={:?} published={:?}",
            page.items.len(),
            first.map(|i| i.title.as_str()),
            first.map(|i| i.kind),
            first.and_then(|i| i.views),
            first.and_then(|i| i.duration),
            first.and_then(|i| i.published).map(|p| p.precision)
        );
    }
    let playlists = channels.playlists(&channel, None).await?;
    println!("  playlists: {}", playlists.items.len());

    println!("\n== a live, found by search");
    let lives = search
        .search(
            "news live",
            &SearchFilters {
                kind: Some(ItemKind::Live),
                ..SearchFilters::default()
            },
            None,
        )
        .await?;
    if let Some(SearchResult::Item(live)) = lives.items.first() {
        let p = yt.player().resolve(&live.r#ref).await?;
        println!(
            "{} [{:?}] views={:?}",
            p.item.title, p.item.kind, p.item.views
        );
        print_selection(&p, "live", &SelectionPrefs::default()).await;
    } else {
        println!("no live found");
    }
    Ok(())
}
