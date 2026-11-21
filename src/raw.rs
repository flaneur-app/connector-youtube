// SPDX-License-Identifier: AGPL-3.0-or-later

//! Parsers for the raw InnerTube answers RustyPipe has no model for: the home feed
//! (`/browse FEwhat_to_watch`) and the account menu.
//!
//! Home pages after the first are continuations, which RustyPipe parses itself; only the first
//! page goes through here. It understands the three item renderers YouTube uses in feeds:
//! `lockupViewModel` (current layout), `videoRenderer` (older layout, still in some answers)
//! and `shortsLockupViewModel` (Shorts shelves).

use flaneur_connector::Account;
use flaneur_model::{ChannelSummary, Item, ItemKind, Thumbnail, Timestamp};
use serde_json::Value;

use crate::map::{bare_item, channel_ref, thumbnail};
use crate::text::{parse_ago, parse_count, parse_duration};
use crate::urls::is_channel_id;

/// The first page of the home feed.
#[derive(Debug, Default)]
pub(crate) struct HomePage {
    pub(crate) items: Vec<Item>,
    /// Continuation token of the next page.
    pub(crate) ctoken: Option<String>,
}

/// Text of a `{simpleText}`, `{runs: [{text}]}` or `{content}` object.
fn text(v: &Value) -> Option<String> {
    if let Some(s) = v.get("simpleText").or_else(|| v.get("content")) {
        return s.as_str().map(str::to_owned);
    }
    let runs = v.get("runs")?.as_array()?;
    Some(
        runs.iter()
            .filter_map(|r| r.get("text").and_then(Value::as_str))
            .collect(),
    )
}

fn str_at<'a>(v: &'a Value, pointer: &str) -> Option<&'a str> {
    v.pointer(pointer).and_then(Value::as_str)
}

/// `[{url, width, height}]` thumbnail lists, under `thumbnails` or `sources`.
fn images(v: Option<&Value>) -> Vec<Thumbnail> {
    let Some(list) = v
        .and_then(|v| v.get("thumbnails").or_else(|| v.get("sources")))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let dim = |t: &Value, key: &str| {
        t.get(key)
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0)
    };
    list.iter()
        .filter_map(|t| thumbnail(t.get("url")?.as_str()?, dim(t, "width"), dim(t, "height")))
        .collect()
}

/// First `browseId` of a channel found anywhere under `v`.
fn channel_id(v: &Value) -> Option<String> {
    match v {
        Value::Object(o) => o
            .get("browseId")
            .and_then(Value::as_str)
            .filter(|id| is_channel_id(id))
            .map(str::to_owned)
            .or_else(|| o.values().find_map(channel_id)),
        Value::Array(a) => a.iter().find_map(channel_id),
        _ => None,
    }
}

/// Parses the first page of the home feed.
pub(crate) fn home(response: &Value, now: Timestamp) -> HomePage {
    let mut page = HomePage::default();
    walk(response, now, &mut page);
    page
}

fn walk(v: &Value, now: Timestamp, page: &mut HomePage) {
    match v {
        Value::Object(o) => {
            if let Some(l) = o.get("lockupViewModel") {
                page.items.extend(lockup(l, now));
            } else if let Some(r) = o.get("videoRenderer") {
                page.items.extend(video_renderer(r, now));
            } else if let Some(s) = o.get("shortsLockupViewModel") {
                page.items.extend(shorts_lockup(s));
            } else if let Some(token) = o
                .get("continuationCommand")
                .and_then(|c| c.get("token"))
                .and_then(Value::as_str)
            {
                page.ctoken.get_or_insert_with(|| token.to_owned());
            } else {
                o.values().for_each(|x| walk(x, now, page));
            }
        }
        Value::Array(a) => a.iter().for_each(|x| walk(x, now, page)),
        _ => {}
    }
}

/// Applies one metadata text of a card ("1.2M views", "3 days ago", a channel name).
fn metadata_part(item: &mut Item, part: &str, now: Timestamp, channel_name: &mut Option<String>) {
    let lower = part.to_ascii_lowercase();
    if (lower.contains("view") || lower.contains("watching"))
        && let Some(count) = parse_count(part)
    {
        item.views = item.views.or(Some(count));
        if lower.contains("watching") {
            item.kind = ItemKind::Live;
        }
    } else if let Some(published) = parse_ago(part, now) {
        item.published.get_or_insert(published);
    } else if lower.starts_with("scheduled for") || lower.starts_with("premieres") {
        item.kind = ItemKind::Upcoming;
    } else if channel_name.is_none() {
        *channel_name = Some(part.to_owned());
    }
}

fn lockup(l: &Value, now: Timestamp) -> Option<Item> {
    if str_at(l, "/contentType") != Some("LOCKUP_CONTENT_TYPE_VIDEO") {
        return None;
    }
    let id = str_at(l, "/contentId")?;
    let meta = l.pointer("/metadata/lockupMetadataViewModel")?;
    let title = meta.get("title").and_then(text).unwrap_or_default();
    let mut item = bare_item(id, ItemKind::Video, title);
    item.thumbnails = images(l.pointer("/contentImage/thumbnailViewModel/image"));

    let badges = l
        .pointer("/contentImage/thumbnailViewModel/overlays")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|o| o.pointer("/thumbnailBottomOverlayViewModel/badges"))
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|b| str_at(b, "/thumbnailBadgeViewModel/text"));
    for badge in badges {
        match badge {
            "LIVE" => item.kind = ItemKind::Live,
            "UPCOMING" => item.kind = ItemKind::Upcoming,
            "SHORTS" => item.kind = ItemKind::Short,
            other => item.duration = item.duration.or_else(|| parse_duration(other)),
        }
    }

    let mut channel_name = None;
    let parts = meta
        .pointer("/metadata/contentMetadataViewModel/metadataRows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| row.get("metadataParts").and_then(Value::as_array))
        .flatten()
        .filter_map(|p| p.get("text").and_then(text));
    for part in parts {
        metadata_part(&mut item, &part, now, &mut channel_name);
    }
    if let (Some(name), Some(cid)) = (channel_name, meta.get("image").and_then(channel_id)) {
        item.channel = Some(ChannelSummary {
            r#ref: channel_ref(&cid),
            name,
            avatars: images(
                meta.pointer("/image/decoratedAvatarViewModel/avatar/avatarViewModel/image"),
            ),
            verified: None,
        });
    }
    Some(finish(item))
}

fn video_renderer(r: &Value, now: Timestamp) -> Option<Item> {
    let id = str_at(r, "/videoId")?;
    let title = r.get("title").and_then(text).unwrap_or_default();
    let mut item = bare_item(id, ItemKind::Video, title);
    item.thumbnails = images(r.get("thumbnail"));
    item.duration = r
        .get("lengthText")
        .and_then(text)
        .and_then(|t| parse_duration(&t));
    item.views = r
        .get("viewCountText")
        .and_then(text)
        .and_then(|t| parse_count(&t));
    item.published = r
        .get("publishedTimeText")
        .and_then(text)
        .and_then(|t| parse_ago(&t, now));
    let live_badge = r
        .get("badges")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|b| str_at(b, "/metadataBadgeRenderer/style") == Some("BADGE_STYLE_TYPE_LIVE_NOW"));
    if live_badge {
        item.kind = ItemKind::Live;
    }
    if let Some(start) = str_at(r, "/upcomingEventData/startTime").and_then(|s| s.parse().ok()) {
        item.kind = ItemKind::Upcoming;
        item.scheduled_start = Some(start);
    }
    let owner = r.pointer("/ownerText/runs/0");
    if let (Some(name), Some(cid)) = (
        owner.and_then(|o| o.get("text")).and_then(Value::as_str),
        owner.and_then(channel_id),
    ) {
        item.channel = Some(ChannelSummary {
            r#ref: channel_ref(&cid),
            name: name.to_owned(),
            avatars: Vec::new(),
            verified: None,
        });
    }
    Some(finish(item))
}

fn shorts_lockup(s: &Value) -> Option<Item> {
    let id = str_at(s, "/onTap/innertubeCommand/reelWatchEndpoint/videoId")
        .or_else(|| str_at(s, "/entityId")?.strip_prefix("shorts-shelf-item-"))?;
    let title = s
        .pointer("/overlayMetadata/primaryText")
        .and_then(text)
        .unwrap_or_default();
    let mut item = bare_item(id, ItemKind::Short, title);
    item.thumbnails = images(s.pointer("/thumbnailViewModel/image"));
    item.views = s
        .pointer("/overlayMetadata/secondaryText")
        .and_then(text)
        .and_then(|t| parse_count(&t));
    Some(item)
}

/// Fixes what depends on the final kind.
fn finish(mut item: Item) -> Item {
    if item.kind == ItemKind::Live {
        item.duration = None;
    }
    item.web_url = crate::map::video_url(&item.r#ref.id, item.kind);
    item
}

/// The signed-in account, from `/account/account_menu`.
pub(crate) fn account(response: &Value) -> Option<Account> {
    let header = find(response, "activeAccountHeaderRenderer")?;
    Some(Account {
        name: header.get("accountName").and_then(text)?,
        handle: header.get("channelHandle").and_then(text),
        avatars: images(header.get("accountPhoto")),
    })
}

fn find<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(o) => o.get(key).or_else(|| o.values().find_map(|x| find(x, key))),
        Value::Array(a) => a.iter().find_map(|x| find(x, key)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flaneur_model::Precision;
    use serde_json::json;

    const NOW: Timestamp = 1_795_222_000;

    fn fixture() -> Value {
        let path = format!("{}/tests/fixtures/home.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn parses_the_three_renderers_of_a_feed() {
        let page = home(&fixture(), NOW);
        assert_eq!(page.items.len(), 3, "the playlist lockup is skipped");
        assert!(page.ctoken.as_deref().unwrap().starts_with("4qmFsg"));

        let lockup = &page.items[0];
        assert_eq!(lockup.r#ref.id, "f0KK_4AaqLQ");
        assert_eq!(lockup.kind, ItemKind::Video);
        assert_eq!(lockup.duration, Some(1008));
        assert_eq!(lockup.views, Some(1_200_000));
        let published = lockup.published.unwrap();
        assert_eq!(published.precision, Precision::Relative);
        assert_eq!(published.at, NOW - 9 * 3600);
        let channel = lockup.channel.as_ref().unwrap();
        assert_eq!(channel.name, "Linus Tech Tips");
        assert_eq!(channel.r#ref.id, "UCXuqSBlHAE6Xw-yeJA0Tunw");
        assert_eq!(lockup.thumbnails.len(), 2);

        let video = &page.items[1];
        assert_eq!(video.r#ref.id, "5C_HPTJg5ek");
        assert_eq!(video.title, "Rust in 100 Seconds");
        assert_eq!(video.duration, Some(149));
        assert_eq!(video.views, Some(2_688_709));
        assert_eq!(video.channel.as_ref().unwrap().name, "Fireship");
        assert_eq!(
            video.channel.as_ref().unwrap().r#ref.id,
            "UCsBjURrPoezykLs9EqgamOA"
        );

        let short = &page.items[2];
        assert_eq!(short.kind, ItemKind::Short);
        assert_eq!(short.r#ref.id, "fLzFtwtMEnY");
        assert_eq!(short.views, Some(27_000));
        assert_eq!(short.duration, None);
        assert!(
            short
                .web_url
                .as_ref()
                .unwrap()
                .as_str()
                .contains("/shorts/")
        );
    }

    #[test]
    fn an_empty_home_is_empty() {
        let nudge = json!({ "contents": { "twoColumnBrowseResultsRenderer": { "tabs": [{
            "tabRenderer": { "content": { "richGridRenderer": { "contents": [{
                "feedNudgeRenderer": { "title": { "simpleText": "Try searching to get started" } }
            }] } } }
        }] } } });
        let page = home(&nudge, NOW);
        assert_eq!(page.items.len(), 0);
        assert!(page.ctoken.is_none());
    }

    #[test]
    fn reads_the_account_header() {
        // Shape of `/account/account_menu` as YouTube.js reads it; not checked against a live
        // answer (no account available).
        let v = json!({ "actions": [{ "openPopupAction": { "popup": { "multiPageMenuRenderer": {
            "header": { "activeAccountHeaderRenderer": {
                "accountName": { "simpleText": "Jane Doe" },
                "channelHandle": { "simpleText": "@janedoe" },
                "accountPhoto": { "thumbnails": [{ "url": "https://yt3.ggpht.com/a=s88", "width": 88, "height": 88 }] }
            } }
        } } } }] });
        let account = account(&v).unwrap();
        assert_eq!(account.name, "Jane Doe");
        assert_eq!(account.handle.as_deref(), Some("@janedoe"));
        assert_eq!(account.avatars.len(), 1);
        assert_eq!(super::account(&json!({})), None);
    }
}
