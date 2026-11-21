// SPDX-License-Identifier: AGPL-3.0-or-later

//! Recognition of shared links, without any request.
//!
//! Channel links by handle (`/@name`), custom name (`/c/name`) or legacy user name (`/user/name`)
//! cannot be turned into a channel id offline: their [`ChannelRef`] keeps that path (`@name`,
//! `c/name`, `user/name`), and the connector resolves it the first time it loads the channel.

use flaneur_connector::Target;
use url::Url;

use crate::map::{channel_ref, item_ref, playlist_ref};

const HOSTS: [&str; 6] = [
    "youtube.com",
    "www.youtube.com",
    "m.youtube.com",
    "music.youtube.com",
    "youtube-nocookie.com",
    "www.youtube-nocookie.com",
];

/// An 11-character video id.
pub(crate) fn is_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A `UC…` channel id.
pub(crate) fn is_channel_id(id: &str) -> bool {
    id.len() == 24
        && id.starts_with("UC")
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn is_playlist_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '%'))
}

fn video(id: &str) -> Option<Target> {
    is_video_id(id).then(|| Target::Item(item_ref(id)))
}

/// What a pasted link points to; a missing scheme (`youtu.be/…`, `www.youtube.com/…`) is
/// taken as `https`.
pub(crate) fn parse_text(text: &str) -> Option<Target> {
    let text = text.trim();
    let url = match Url::parse(text) {
        Ok(url) => url,
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            Url::parse(&format!("https://{text}")).ok()?
        }
        Err(_) => return None,
    };
    parse(&url)
}

/// What a YouTube URL points to.
pub(crate) fn parse(url: &Url) -> Option<Target> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let mut segments = url.path_segments()?.filter(|s| !s.is_empty());
    let first = segments.next();
    let second = segments.next();

    if host == "youtu.be" {
        return video(first?);
    }
    if !HOSTS.contains(&host.as_str()) {
        return None;
    }

    let query = |key: &str| {
        url.query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
    };
    match (first?, second) {
        ("watch", _) => match query("v") {
            Some(v) => video(&v),
            None => query("list")
                .filter(|l| is_playlist_id(l))
                .map(|l| Target::Playlist(playlist_ref(&l))),
        },
        ("shorts" | "live" | "embed" | "v" | "e", Some(id)) => video(id),
        ("playlist", _) => query("list")
            .filter(|l| is_playlist_id(l))
            .map(|l| Target::Playlist(playlist_ref(&l))),
        ("channel", Some(id)) => is_channel_id(id).then(|| Target::Channel(channel_ref(id))),
        (kind @ ("c" | "user"), Some(name)) => {
            is_name(name).then(|| Target::Channel(channel_ref(&format!("{kind}/{name}"))))
        }
        (handle, _) if handle.starts_with('@') && is_name(&handle[1..]) => {
            Some(Target::Channel(channel_ref(handle)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(url: &str) -> Option<Target> {
        parse(&Url::parse(url).unwrap())
    }

    fn item(url: &str) -> Option<String> {
        match target(url)? {
            Target::Item(r) => Some(r.id),
            _ => None,
        }
    }

    fn channel(url: &str) -> Option<String> {
        match target(url)? {
            Target::Channel(r) => Some(r.id),
            _ => None,
        }
    }

    #[test]
    fn video_links() {
        let id = Some("dQw4w9WgXcQ".to_owned());
        assert_eq!(item("https://www.youtube.com/watch?v=dQw4w9WgXcQ"), id);
        assert_eq!(
            item("https://m.youtube.com/watch?feature=share&v=dQw4w9WgXcQ&t=42s"),
            id
        );
        assert_eq!(item("https://youtu.be/dQw4w9WgXcQ?si=abc"), id);
        assert_eq!(item("https://www.youtube.com/shorts/dQw4w9WgXcQ"), id);
        assert_eq!(
            item("https://youtube.com/live/dQw4w9WgXcQ?feature=share"),
            id
        );
        assert_eq!(
            item("https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ"),
            id
        );
        assert_eq!(
            item(
                "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL2_OBreMn7FrsiSW0VDZjdq0xqUKkZYHT"
            ),
            id
        );
        assert_eq!(item("https://www.youtube.com/watch?v=short"), None);
    }

    #[test]
    fn channel_links() {
        assert_eq!(
            channel("https://www.youtube.com/channel/UCXuqSBlHAE6Xw-yeJA0Tunw/videos"),
            Some("UCXuqSBlHAE6Xw-yeJA0Tunw".into())
        );
        assert_eq!(
            channel("https://www.youtube.com/@LinusTechTips"),
            Some("@LinusTechTips".into())
        );
        assert_eq!(
            channel("https://www.youtube.com/@LinusTechTips/shorts"),
            Some("@LinusTechTips".into())
        );
        assert_eq!(
            channel("https://www.youtube.com/c/LinusTechTips"),
            Some("c/LinusTechTips".into())
        );
        assert_eq!(
            channel("https://www.youtube.com/user/LinusTechTips"),
            Some("user/LinusTechTips".into())
        );
    }

    #[test]
    fn playlist_links() {
        let Some(Target::Playlist(p)) =
            target("https://www.youtube.com/playlist?list=PL2_OBreMn7FrsiSW0VDZjdq0xqUKkZYHT")
        else {
            panic!("playlist expected");
        };
        assert_eq!(p.id, "PL2_OBreMn7FrsiSW0VDZjdq0xqUKkZYHT");
        assert_eq!(p.service.0, "youtube");
    }

    #[test]
    fn pasted_text() {
        let id = |t| match parse_text(t) {
            Some(Target::Item(r)) => Some(r.id),
            _ => None,
        };
        assert_eq!(
            id("  https://youtu.be/dQw4w9WgXcQ \n"),
            Some("dQw4w9WgXcQ".into())
        );
        assert_eq!(id("youtu.be/dQw4w9WgXcQ"), Some("dQw4w9WgXcQ".into()));
        assert_eq!(
            id("www.youtube.com/watch?v=dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".into())
        );
        assert_eq!(parse_text("not a link"), None);
        assert_eq!(parse_text("lbry://@channel"), None);
    }

    #[test]
    fn other_links() {
        assert_eq!(target("https://example.org/watch?v=dQw4w9WgXcQ"), None);
        assert_eq!(target("https://www.youtube.com/feed/history"), None);
        assert_eq!(target("https://www.youtube.com/"), None);
        assert_eq!(target("ftp://youtube.com/watch?v=dQw4w9WgXcQ"), None);
    }
}
