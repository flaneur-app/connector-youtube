// SPDX-License-Identifier: AGPL-3.0-or-later

//! Signed-in operations against your own account, to check them by hand.
//!
//! ```bash
//! export FLANEUR_YT_COOKIES=~/secrets/youtube-cookies.txt   # Netscape cookies.txt, chmod 600
//! cargo run --example account -- whoami
//! cargo run --example account -- history | subscriptions | feed | home | playlists | watch-later
//! # These change your account for real:
//! cargo run --example account -- like <video> | dislike <video> | unrate <video>
//! cargo run --example account -- subscribe <UC… or @handle> | unsubscribe <…>
//! cargo run --example account -- wl-add <video> | wl-remove <video>
//! cargo run --example account -- comment <video> "text"          # PUBLIC
//! cargo run --example account -- reply <video> "text"            # PUBLIC, to the top comment
//! cargo run --example account -- watched <video> <seconds>       # history + resume position
//! ```
//!
//! Nothing is written to disk: the cookies are read from the file, kept in memory, and sent to
//! youtube.com only.

use std::error::Error;

use flaneur_connector::{Connector, Credentials, FeedKind, PlaybackEvent, PlaybackReport};
use flaneur_connector_youtube::{Config, YouTube};
use flaneur_model::{ChannelRef, Item, ItemRef, Reaction, ServiceId};

type Res<T = ()> = Result<T, Box<dyn Error>>;

fn print_items(items: &[Item]) {
    for i in items.iter().take(10) {
        println!(
            "  [{:?}] {} ({}) | {} | views={:?} published={:?}",
            i.kind,
            i.title,
            i.r#ref.id,
            i.channel.as_ref().map_or("?", |c| c.name.as_str()),
            i.views,
            i.published.map(|p| p.precision)
        );
    }
}

fn arg(args: &[String], n: usize, what: &str) -> Res<String> {
    args.get(n)
        .cloned()
        .ok_or_else(|| format!("missing {what}").into())
}

// One match arm per command: long, but flat.
#[allow(clippy::too_many_lines)]
#[tokio::main]
async fn main() -> Res {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map_or("whoami", String::as_str);
    let path = std::env::var("FLANEUR_YT_COOKIES").unwrap_or_else(|_| "cookies.txt".into());
    let cookies = std::fs::read_to_string(&path)
        .map_err(|e| format!("reading {path}: {e} (set FLANEUR_YT_COOKIES)"))?;

    let yt = YouTube::new(Config::default())?;
    let session = yt.sign_in(Credentials::Cookies(cookies)).await?;
    println!(
        "signed in as {:?}; capabilities.signed_in = {}",
        session.account.as_ref().map(|a| (&a.name, &a.handle)),
        yt.capabilities().signed_in
    );
    let yt_ref = |id: String| ItemRef::new(ServiceId::new("youtube"), id);
    let library = yt.library().ok_or("no library")?;
    let subscriptions = yt.subscriptions().ok_or("no subscriptions")?;

    match command {
        "whoami" => {}
        "history" => {
            let page = library.history(None).await?;
            println!(
                "history: {} items, more: {}",
                page.items.len(),
                page.next.is_some()
            );
            print_items(&page.items);
        }
        "subscriptions" => {
            let page = subscriptions.list(None).await?;
            println!(
                "subscriptions: {} channels, total {:?}",
                page.items.len(),
                page.total
            );
            for c in page.items.iter().take(10) {
                println!("  {} ({})", c.name, c.r#ref.id);
            }
        }
        "feed" | "home" => {
            let kind = if command == "feed" {
                FeedKind::Subscriptions
            } else {
                FeedKind::Home
            };
            let page = yt.feeds().ok_or("no feeds")?.feed(kind, None).await?;
            println!(
                "{kind:?}: {} items, more: {}",
                page.items.len(),
                page.next.is_some()
            );
            print_items(&page.items);
            if let Some(next) = &page.next {
                let more = yt.feeds().ok_or("no feeds")?.feed(kind, Some(next)).await?;
                println!("page 2: {} items", more.items.len());
            }
        }
        "playlists" => {
            let page = library.playlists(None).await?;
            for p in &page.items {
                println!("  {} ({}) {:?} items", p.title, p.r#ref.id, p.item_count);
            }
        }
        "watch-later" => {
            let wl = library.watch_later().ok_or("no watch later")?;
            let page = library.playlist_items(&wl, None).await?;
            println!(
                "watch later: {} items, total {:?}",
                page.items.len(),
                page.total
            );
            print_items(&page.items);
        }
        "like" | "dislike" | "unrate" => {
            let reaction = match command {
                "like" => Reaction::Like,
                "dislike" => Reaction::Dislike,
                _ => Reaction::None,
            };
            let video = yt_ref(arg(&args, 1, "video id")?);
            yt.reactions()
                .ok_or("no reactions")?
                .react(&video, reaction)
                .await?;
            println!("{command}: done; check https://www.youtube.com/playlist?list=LL");
        }
        "subscribe" | "unsubscribe" => {
            let channel = ChannelRef::new(ServiceId::new("youtube"), arg(&args, 1, "channel")?);
            subscriptions
                .set_subscribed(&channel, command == "subscribe")
                .await?;
            println!("{command}: done; check https://www.youtube.com/feed/channels");
        }
        "wl-add" | "wl-remove" => {
            let video = yt_ref(arg(&args, 1, "video id")?);
            let wl = library.watch_later().ok_or("no watch later")?;
            if command == "wl-add" {
                library.add_to_playlist(&wl, &video).await?;
            } else {
                library.remove_from_playlist(&wl, &video).await?;
            }
            println!("{command}: done; check https://www.youtube.com/playlist?list=WL");
        }
        "reply" => {
            let video = yt_ref(arg(&args, 1, "video id")?);
            let text = arg(&args, 2, "text")?;
            let comments = yt.comments().ok_or("no comments")?;
            let top = comments
                .comments(&video, flaneur_connector::CommentSort::Top, None)
                .await?;
            let parent = top.items.first().ok_or("no comment to reply to")?;
            let token = parent.reply_token.as_deref().ok_or("no reply token")?;
            let reply = comments.post(&video, &text, Some(token)).await?;
            println!(
                "replied to {:?} ({}): reply id {:?}; delete it on the video page",
                parent.author.name, parent.id, reply.id
            );
        }
        "comment" => {
            let video = yt_ref(arg(&args, 1, "video id")?);
            let text = arg(&args, 2, "text")?;
            let comment = yt
                .comments()
                .ok_or("no comments")?
                .post(&video, &text, None)
                .await?;
            println!(
                "posted comment id {:?}; delete it on the video page",
                comment.id
            );
        }
        "watched" => {
            let video = yt_ref(arg(&args, 1, "video id")?);
            let position: u32 = arg(&args, 2, "position in seconds")?.parse()?;
            let playable = yt.player().resolve(&video).await?;
            println!("tracking token present: {}", playable.tracking.is_some());
            for (event, at, watched) in [
                (PlaybackEvent::Started, 0, 0),
                (PlaybackEvent::Progress, position, position),
                (PlaybackEvent::Stopped, position, 0),
            ] {
                library
                    .report_playback(&PlaybackReport {
                        item: video.clone(),
                        event,
                        position: at,
                        watched_since_last: watched,
                        tracking: playable.tracking.clone(),
                    })
                    .await?;
                println!("  {event:?} at {at}s: sent");
            }
            println!(
                "check https://www.youtube.com/feed/history: entry, and progress bar at {position}s"
            );
        }
        other => return Err(format!("unknown command {other}").into()),
    }
    yt.sign_out().await;
    Ok(())
}
