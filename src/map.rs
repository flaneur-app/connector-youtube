// SPDX-License-Identifier: AGPL-3.0-or-later

//! RustyPipe's model mapped into Flâneur's.
//!
//! Absent stays absent: RustyPipe's `None` and the zero placeholders YouTube's new list layouts
//! leave (no views, no date, no duration on channel lists) stay `None`, never `0`.

use flaneur_model::{
    AudioTrack, Channel, ChannelRef, ChannelSummary, Comment, CommentAuthor, Container,
    ContentFlags, Item, ItemKind, ItemRef, Playable, Playlist, PlaylistRef, Precision, Published,
    ReactionCounts, ServiceId, Stream, StreamKind, Thumbnail, Timestamp, VideoCodec,
};
use rustypipe::model as rp;
use rustypipe::model::richtext::ToPlaintext;
use url::Url;

use crate::cursor;
use crate::text::is_relative;

/// Id of the connector, stored in Flâneur's database.
pub const SERVICE: &str = "youtube";

/// Longest duration YouTube accepts for a Short.
const SHORT_MAX_SECONDS: u32 = 180;

pub(crate) fn service() -> ServiceId {
    ServiceId::new(SERVICE)
}

fn nonzero<T: Default + PartialEq>(v: T) -> Option<T> {
    (v != T::default()).then_some(v)
}

pub(crate) fn thumbnail(url: &str, width: u32, height: u32) -> Option<Thumbnail> {
    let url = if url.starts_with("//") {
        Url::parse(&format!("https:{url}"))
    } else {
        Url::parse(url)
    };
    Some(Thumbnail {
        url: url.ok()?,
        width: nonzero(width),
        height: nonzero(height),
    })
}

pub(crate) fn thumbnails(thumbnails: &[rp::Thumbnail]) -> Vec<Thumbnail> {
    thumbnails
        .iter()
        .filter_map(|t| thumbnail(&t.url, t.width, t.height))
        .collect()
}

/// A badge is a positive statement; its absence in a listing is no proof of the contrary.
fn verified(v: rp::Verification) -> Option<bool> {
    v.verified().then_some(true)
}

pub(crate) fn video_url(id: &str, kind: ItemKind) -> Option<Url> {
    match kind {
        ItemKind::Short => Url::parse(&format!("https://www.youtube.com/shorts/{id}")),
        _ => Url::parse(&format!("https://www.youtube.com/watch?v={id}")),
    }
    .ok()
}

pub(crate) fn channel_url(id: &str) -> Option<Url> {
    Url::parse(&format!("https://www.youtube.com/channel/{id}")).ok()
}

pub(crate) fn channel_ref(id: &str) -> ChannelRef {
    ChannelRef::new(service(), id)
}

pub(crate) fn item_ref(id: &str) -> ItemRef {
    ItemRef::new(service(), id)
}

pub(crate) fn playlist_ref(id: &str) -> PlaylistRef {
    PlaylistRef::new(service(), id)
}

pub(crate) fn channel_tag(c: &rp::ChannelTag) -> Option<ChannelSummary> {
    (!c.id.is_empty()).then(|| ChannelSummary {
        r#ref: channel_ref(&c.id),
        name: c.name.clone(),
        avatars: thumbnails(&c.avatar),
        verified: verified(c.verification),
    })
}

pub(crate) fn channel_item(c: rp::ChannelItem) -> ChannelSummary {
    ChannelSummary {
        r#ref: channel_ref(&c.id),
        name: c.name,
        avatars: thumbnails(&c.avatar),
        verified: verified(c.verification),
    }
}

pub(crate) fn kind(is_live: bool, is_upcoming: bool, is_short: bool) -> ItemKind {
    if is_upcoming {
        ItemKind::Upcoming
    } else if is_live {
        ItemKind::Live
    } else if is_short {
        ItemKind::Short
    } else {
        ItemKind::Video
    }
}

/// A publication date. RustyPipe estimates dates from "3 days ago" texts: those are
/// [`Precision::Relative`]; dates printed as such ("Oct 25, 2009") are [`Precision::Day`].
pub(crate) fn published(
    date: Option<time::OffsetDateTime>,
    text: Option<&str>,
) -> Option<Published> {
    let at = date?.unix_timestamp();
    let precision = if text.is_some_and(is_relative) {
        Precision::Relative
    } else {
        Precision::Day
    };
    Some(Published { at, precision })
}

/// An item with only what every listing has; the rest is filled by the caller.
pub(crate) fn bare_item(id: &str, kind: ItemKind, title: String) -> Item {
    Item {
        r#ref: item_ref(id),
        kind,
        title,
        description: None,
        duration: None,
        published: None,
        scheduled_start: None,
        channel: None,
        thumbnails: Vec::new(),
        views: None,
        reactions: ReactionCounts::default(),
        flags: ContentFlags::default(),
        web_url: video_url(id, kind),
        watch: None,
    }
}

pub(crate) fn video_item(v: rp::VideoItem) -> Item {
    let kind = kind(v.is_live, v.is_upcoming, v.is_short);
    let mut item = bare_item(&v.id, kind, v.name);
    item.duration = v.duration.and_then(nonzero);
    item.published = published(v.publish_date, v.publish_date_txt.as_deref());
    item.channel = v.channel.as_ref().and_then(channel_tag);
    item.thumbnails = thumbnails(&v.thumbnail);
    item.views = v.view_count;
    if kind == ItemKind::Upcoming {
        // Upcoming items carry their start time as the publication date.
        item.scheduled_start = item.published.map(|p| p.at);
        item.published = None;
    }
    item
}

pub(crate) fn playlist_item(p: rp::PlaylistItem) -> Playlist {
    Playlist {
        r#ref: playlist_ref(&p.id),
        title: p.name,
        channel: p.channel.as_ref().and_then(channel_tag),
        item_count: p.video_count,
        thumbnails: thumbnails(&p.thumbnail),
    }
}

pub(crate) fn comment(c: rp::Comment) -> Comment {
    let author = match &c.author {
        Some(a) => CommentAuthor {
            name: a.name.clone(),
            channel: (!a.id.is_empty()).then(|| channel_ref(&a.id)),
            avatars: thumbnails(&a.avatar),
            is_publisher: c.by_owner,
        },
        None => CommentAuthor {
            name: String::new(),
            channel: None,
            avatars: Vec::new(),
            is_publisher: c.by_owner,
        },
    };
    let replies_token = (c.reply_count > 0)
        .then(|| cursor::next(&c.replies))
        .flatten()
        .map(|cursor| cursor.0);
    Comment {
        published: published(c.publish_date, Some(&c.publish_date_txt)),
        id: c.id,
        author,
        text: c.text.to_plaintext(),
        likes: c.like_count.map(u64::from),
        reply_count: Some(u64::from(c.reply_count)),
        replies_token,
        pinned: c.pinned,
        hearted: c.hearted,
        my_reaction: None,
    }
}

/// The header of a channel page.
pub(crate) fn channel_header<T>(c: &rp::Channel<T>) -> Channel {
    // RustyPipe 0.11.4 read the video count as the subscriber count on YouTube's current channel
    // header (8000 subscribers for a channel of 8000 videos). The pinned revision reads it right;
    // a count equal to the video count is still dropped rather than shown wrong.
    let subscribers = match (c.subscriber_count, c.video_count) {
        (Some(s), Some(v)) if s == v => None,
        (s, _) => s,
    };
    Channel {
        summary: ChannelSummary {
            r#ref: channel_ref(&c.id),
            name: c.name.clone(),
            avatars: thumbnails(&c.avatar),
            verified: verified(c.verification),
        },
        handle: c.handle.clone(),
        description: nonzero(c.description.clone()),
        subscribers,
        banners: thumbnails(&c.banner),
        subscribed: None,
        web_url: channel_url(&c.id),
    }
}

/// Codecs of a MIME type: `video/mp4; codecs="avc1.640028, mp4a.40.2"` → `["avc1.640028",
/// "mp4a.40.2"]`.
pub(crate) fn mime_codecs(mime: &str) -> Vec<String> {
    let Some((_, codecs)) = mime.split_once("codecs=") else {
        return Vec::new();
    };
    codecs
        .trim_matches(|c: char| c == '"' || c.is_whitespace())
        .split(',')
        .map(|c| c.trim().trim_matches('"').to_owned())
        .filter(|c| !c.is_empty())
        .collect()
}

fn mime_type(mime: &str) -> String {
    mime.split(';').next().unwrap_or(mime).trim().to_owned()
}

fn video_codec(codec: rp::VideoCodec, codecs: &[String]) -> VideoCodec {
    match codec {
        rp::VideoCodec::Avc1 => VideoCodec::H264,
        rp::VideoCodec::Vp9 => VideoCodec::Vp9,
        rp::VideoCodec::Av01 => VideoCodec::Av1,
        _ => VideoCodec::Other(codecs.first().cloned().unwrap_or_default()),
    }
}

fn video_stream(s: &rp::VideoStream, kind: StreamKind) -> Option<Stream> {
    // DRM-protected tracks cannot be played without a license; SABR-only formats have no URL.
    if s.drm_track_type.is_some() {
        return None;
    }
    let url = Url::parse(s.url.as_deref()?).ok()?;
    let codecs = mime_codecs(&s.mime);
    let container = match s.format {
        rp::VideoFormat::Mp4 => Container::Mp4,
        rp::VideoFormat::Webm => Container::WebM,
        _ => Container::Other(mime_type(&s.mime)),
    };
    let audio = (kind == StreamKind::Muxed).then(|| AudioTrack {
        codec: codecs.get(1).cloned(),
        language: None,
        label: None,
        original: None,
        sample_rate: None,
        channels: None,
    });
    Some(Stream {
        url,
        kind,
        container,
        bitrate: nonzero(s.average_bitrate).or(nonzero(s.bitrate)),
        size: s.size.and_then(nonzero),
        width: nonzero(s.width),
        height: nonzero(s.height),
        fps: nonzero(u16::from(s.fps)),
        video_codec: Some(video_codec(s.codec, &codecs)),
        hdr: Some(s.hdr),
        audio,
    })
}

fn audio_stream(s: &rp::AudioStream) -> Option<Stream> {
    if s.drm_track_type.is_some() {
        return None;
    }
    let url = Url::parse(s.url.as_deref()?).ok()?;
    let container = match s.format {
        rp::AudioFormat::M4a => Container::Mp4,
        rp::AudioFormat::Webm => Container::WebM,
        _ => Container::Other(mime_type(&s.mime)),
    };
    let original = match &s.track {
        // No track information: the video has a single audio track, the original one.
        None => Some(true),
        Some(t) => match t.track_type {
            Some(rp::AudioTrackType::Original) => Some(true),
            Some(_) => Some(false),
            None => None,
        },
    };
    Some(Stream {
        url,
        kind: StreamKind::Audio,
        container,
        bitrate: nonzero(s.average_bitrate).or(nonzero(s.bitrate)),
        size: nonzero(s.size),
        width: None,
        height: None,
        fps: None,
        video_codec: None,
        hdr: None,
        audio: Some(AudioTrack {
            codec: mime_codecs(&s.mime).into_iter().next(),
            language: s.track.as_ref().and_then(|t| t.lang.clone()),
            label: s.track.as_ref().map(|t| t.lang_name.clone()),
            original,
            sample_rate: None,
            channels: s.channels,
        }),
    })
}

/// Every stream of the player that can be fetched directly.
pub(crate) fn streams(p: &rp::VideoPlayer) -> Vec<Stream> {
    let muxed = p
        .video_streams
        .iter()
        .filter_map(|s| video_stream(s, StreamKind::Muxed));
    let video = p
        .video_only_streams
        .iter()
        .filter_map(|s| video_stream(s, StreamKind::Video));
    let audio = p.audio_streams.iter().filter_map(audio_stream);
    muxed.chain(video).chain(audio).collect()
}

/// A player answer Flâneur can play: separate video and audio with URLs, or an HLS manifest.
/// A lone muxed format does not count: the clients that only give that one (TV, Android VR)
/// sign it for themselves and it answers 403.
pub(crate) fn is_usable(p: &rp::VideoPlayer) -> bool {
    let has_url = |url: &Option<String>| url.is_some();
    p.hls_manifest_url.is_some()
        || (p
            .video_only_streams
            .iter()
            .any(|s| has_url(&s.url) && s.drm_track_type.is_none())
            && p.audio_streams
                .iter()
                .any(|s| has_url(&s.url) && s.drm_track_type.is_none()))
}

/// The `expire` parameter of a `googlevideo.com` URL, as a query parameter or a path segment
/// (manifests use `/expire/<ts>/`).
pub(crate) fn expire_param(url: &Url) -> Option<Timestamp> {
    if let Some((_, v)) = url.query_pairs().find(|(k, _)| k == "expire") {
        return v.parse().ok();
    }
    let mut segments = url.path_segments()?;
    segments.find(|s| *s == "expire")?;
    segments.next()?.parse().ok()
}

/// Portrait video no longer than a Short can be: the player answer has no "Short" flag.
fn looks_like_short(p: &rp::VideoPlayer) -> bool {
    let duration = p.details.duration;
    duration > 0
        && duration <= SHORT_MAX_SECONDS
        && p.video_only_streams
            .iter()
            .chain(&p.video_streams)
            .max_by_key(|s| s.height)
            .is_some_and(|s| s.height > s.width)
}

/// The watch page: details from `/next` when they came, streams from `/player`.
pub(crate) fn playable(
    details: Option<&rp::VideoDetails>,
    player: &rp::VideoPlayer,
    tracking: Option<String>,
) -> Playable {
    let d = &player.details;
    let kind = if d.is_live {
        // A premiere plays like a live, but its video was never streamed live.
        if d.is_live_content {
            ItemKind::Live
        } else {
            ItemKind::Premiere
        }
    } else if looks_like_short(player) {
        ItemKind::Short
    } else {
        ItemKind::Video
    };

    let title = details
        .map(|v| v.name.clone())
        .or_else(|| d.name.clone())
        .unwrap_or_default();
    let mut item = bare_item(&d.id, kind, title);
    item.description = details
        .map(|v| v.description.to_plaintext())
        .or_else(|| d.description.clone())
        .and_then(nonzero);
    item.duration = nonzero(d.duration);
    item.thumbnails = thumbnails(&d.thumbnail);
    item.views = details.map(|v| v.view_count).or(d.view_count);
    if let Some(v) = details {
        item.published = published(v.publish_date, v.publish_date_txt.as_deref());
        item.channel = channel_tag(&v.channel);
        item.reactions = ReactionCounts {
            likes: v.like_count.map(u64::from),
            dislikes: None,
            comments: v.top_comments.count,
        };
    }
    if item.channel.is_none() && !d.channel_id.is_empty() {
        item.channel = Some(ChannelSummary {
            r#ref: channel_ref(&d.channel_id),
            name: d.channel_name.clone().unwrap_or_default(),
            avatars: Vec::new(),
            verified: None,
        });
    }

    let streams = streams(player);
    let hls_manifest = player
        .hls_manifest_url
        .as_deref()
        .and_then(|u| Url::parse(u).ok());
    let dash_manifest = player
        .dash_manifest_url
        .as_deref()
        .and_then(|u| Url::parse(u).ok());
    let expires_at = streams
        .iter()
        .map(|s| &s.url)
        .chain(&hls_manifest)
        .find_map(expire_param)
        .or_else(|| Some(player.valid_until.unix_timestamp()));

    Playable {
        item,
        streams,
        hls_manifest,
        dash_manifest,
        expires_at,
        start_at: None,
        tracking,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn fixture<T: serde::de::DeserializeOwned>(name: &str) -> T {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn maps_ios_player_streams() {
        let player: rp::VideoPlayer = fixture("player_ios.json");
        let streams = streams(&player);
        // 4 video formats with URLs, 1 SABR-only format without, 2 audio formats.
        assert_eq!(streams.len(), 6);
        let av1 = &streams[0];
        assert_eq!(av1.kind, StreamKind::Video);
        assert_eq!(av1.video_codec, Some(VideoCodec::Av1));
        assert_eq!(av1.container, Container::Mp4);
        assert_eq!(av1.height, Some(2160));
        assert!(av1.fps.is_some() && av1.bitrate.is_some() && av1.size.is_some());
        assert_eq!(streams[1].video_codec, Some(VideoCodec::Vp9));
        assert_eq!(streams[1].container, Container::WebM);
        assert_eq!(streams[2].video_codec, Some(VideoCodec::H264));
        let opus = &streams[4];
        assert_eq!(opus.kind, StreamKind::Audio);
        let track = opus.audio.as_ref().unwrap();
        assert_eq!(track.codec.as_deref(), Some("opus"));
        assert_eq!(track.original, Some(true));
        assert_eq!(
            streams[5].audio.as_ref().unwrap().codec.as_deref(),
            Some("mp4a.40.2")
        );
        assert!(is_usable(&player));
    }

    #[test]
    fn playable_carries_expiry_manifest_and_details() {
        let player: rp::VideoPlayer = fixture("player_ios.json");
        let details: rp::VideoDetails = fixture("video_details.json");
        let p = playable(Some(&details), &player, None);
        assert_eq!(p.expires_at, Some(1_795_244_199));
        assert!(p.hls_manifest.is_some());
        assert_eq!(p.item.kind, ItemKind::Video);
        assert_eq!(p.item.duration, Some(213));
        assert!(p.item.reactions.likes.is_some());
        let channel = p.item.channel.unwrap();
        assert_eq!(channel.r#ref.id, "UCuAXFkgsw1L7xaCfnd5JJOw");
        assert_eq!(p.item.published.unwrap().precision, Precision::Day);
    }

    #[test]
    fn playable_without_details_falls_back_to_the_player() {
        let player: rp::VideoPlayer = fixture("player_ios.json");
        let p = playable(None, &player, None);
        assert_ne!(p.item.title, "");
        assert_eq!(p.item.channel.unwrap().r#ref.id, "UCuAXFkgsw1L7xaCfnd5JJOw");
        assert_eq!(p.item.reactions.likes, None);
    }

    #[test]
    fn dubbed_tracks_are_not_original() {
        let mut v: Value = fixture("player_ios.json");
        v["audio_streams"][0]["track"] = serde_json::json!({
            "id": "de.3", "lang": "de", "lang_name": "German", "is_default": false,
            "track_type": "dubbed-auto"
        });
        let player: rp::VideoPlayer = serde_json::from_value(v).unwrap();
        let audio = streams(&player)
            .into_iter()
            .find(|s| s.kind == StreamKind::Audio)
            .unwrap()
            .audio
            .unwrap();
        assert_eq!(audio.language.as_deref(), Some("de"));
        assert_eq!(audio.label.as_deref(), Some("German"));
        assert_eq!(audio.original, Some(false));
    }

    #[test]
    fn sabr_only_players_are_not_usable() {
        let mut v: Value = fixture("player_ios.json");
        for s in v["video_only_streams"].as_array_mut().unwrap() {
            s["url"] = Value::Null;
        }
        v["hls_manifest_url"] = Value::Null;
        let player: rp::VideoPlayer = serde_json::from_value(v).unwrap();
        assert!(!is_usable(&player));
    }

    #[test]
    fn live_without_live_content_is_a_premiere() {
        let mut v: Value = fixture("player_ios.json");
        v["details"]["is_live"] = Value::Bool(true);
        let player: rp::VideoPlayer = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(playable(None, &player, None).item.kind, ItemKind::Premiere);
        v["details"]["is_live_content"] = Value::Bool(true);
        let player: rp::VideoPlayer = serde_json::from_value(v).unwrap();
        assert_eq!(playable(None, &player, None).item.kind, ItemKind::Live);
    }

    #[test]
    fn channel_shorts_keep_unknowns_unknown() {
        let ch: rp::Channel<rp::paginator::Paginator<rp::VideoItem>> =
            fixture("channel_shorts.json");
        let header = channel_header(&ch);
        assert_eq!(header.handle.as_deref(), Some("@LinusTechTips"));
        assert!(header.subscribers.unwrap() > 1_000_000);
        for v in ch.content.items {
            let item = video_item(v);
            assert_eq!(item.kind, ItemKind::Short);
            assert_eq!(item.duration, None);
            assert_eq!(item.published, None);
            assert!(item.views.is_some());
            assert!(item.web_url.unwrap().as_str().contains("/shorts/"));
        }
    }

    #[test]
    fn subscriber_count_equal_to_video_count_is_dropped() {
        let mut v: Value = fixture("channel_live.json");
        v["subscriber_count"] = v["video_count"].clone();
        let ch: rp::Channel<rp::paginator::Paginator<rp::VideoItem>> =
            serde_json::from_value(v).unwrap();
        assert_eq!(channel_header(&ch).subscribers, None);
    }

    #[test]
    fn streamed_lives_have_relative_dates() {
        let ch: rp::Channel<rp::paginator::Paginator<rp::VideoItem>> = fixture("channel_live.json");
        let item = video_item(ch.content.items[0].clone());
        assert_eq!(item.kind, ItemKind::Video);
        assert_eq!(item.published.unwrap().precision, Precision::Relative);
        assert!(item.duration.is_some());
    }

    #[test]
    fn search_items() {
        let items: Vec<rp::YouTubeItem> = fixture("search_items.json");
        let mut seen_video = false;
        for it in items {
            match it {
                rp::YouTubeItem::Video(v) => {
                    let item = video_item(v);
                    assert_eq!(item.published.unwrap().precision, Precision::Relative);
                    assert!(item.channel.is_some());
                    seen_video = true;
                }
                rp::YouTubeItem::Channel(c) => {
                    assert!(channel_item(c).r#ref.id.starts_with("UC"));
                }
                rp::YouTubeItem::Playlist(p) => {
                    let _ = playlist_item(p);
                }
            }
        }
        assert!(seen_video);
    }

    #[test]
    fn comments_with_replies_have_a_replies_token() {
        let page: rp::paginator::Paginator<rp::Comment> = fixture("comments.json");
        for c in page.items {
            let c = comment(c);
            assert!(c.reply_count.unwrap() > 0);
            assert!(c.replies_token.is_some());
            assert_eq!(c.published.unwrap().precision, Precision::Relative);
            assert!(c.author.channel.is_some());
        }
    }

    #[test]
    fn expiry_from_query_or_path() {
        let q = Url::parse("https://rr1.googlevideo.com/videoplayback?expire=1795243457&itag=1")
            .unwrap();
        assert_eq!(expire_param(&q), Some(1_795_243_457));
        let p = Url::parse(
            "https://manifest.googlevideo.com/api/manifest/hls_variant/expire/1795243457/id/x",
        )
        .unwrap();
        assert_eq!(expire_param(&p), Some(1_795_243_457));
        assert_eq!(
            expire_param(&Url::parse("https://example.org/").unwrap()),
            None
        );
    }

    #[test]
    fn mime_codec_lists() {
        assert_eq!(
            mime_codecs(r#"video/mp4; codecs="avc1.42001E, mp4a.40.2""#),
            ["avc1.42001E", "mp4a.40.2"]
        );
        assert_eq!(mime_codecs("video/mp4").len(), 0);
    }
}
