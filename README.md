# Flâneur connector: YouTube

The YouTube connector of [Flâneur](https://github.com/flaneur-app), built on [RustyPipe](https://codeberg.org/ThetaDev/rustypipe) (InnerTube). It implements the connector traits of [`flaneur-app/core`](https://github.com/flaneur-app/core).

> Flâneur is an independent, open-source project. It is not affiliated with, endorsed by or
> sponsored by Google, YouTube, Odysee, LBRY or any other service it can connect to. All
> trademarks belong to their owners.

## What works

Crate `flaneur-connector-youtube`, service id `youtube`. Every facet of `flaneur_connector` is
implemented; `capabilities()` says which operation needs a session.

| Operation | Support | Status |
| --- | --- | --- |
| Search, suggestions, filters (kind, upload date) | anonymous | verified live |
| Stream resolution (direct video + audio URLs, HLS for lives) | anonymous | verified live |
| Related items | anonymous | verified live |
| Channel header, Videos / Shorts / Lives tabs, playlists | anonymous | verified live |
| Comments (top, newest, replies) | anonymous | verified live |
| Public playlists | anonymous | verified live |
| Shared links (`watch`, `youtu.be`, `shorts`, `live`, `embed`, `@handle`, `channel`, `c`, `user`, `playlist`) | offline | unit-tested |
| History, playlists + Watch later + Liked, subscriptions, subscription feed, home | signed in | implemented, **not verified** (no account) |
| Like / dislike, subscribe, Watch later and playlist edits, top-level comments | signed in | implemented, **untested** |
| Playback reporting (watch history, resume position) | signed in | implemented, **untested** |
| Trending | — | unsupported: YouTube removed the page in 2025 |
| Shorts feed | — | unsupported for now (channel Shorts tabs work) |
| Replying to a comment | — | unsupported: needs `createReplyParams`, which RustyPipe does not keep |

"Verified live" means checked against youtube.com, anonymously, by `cargo nextest run
--run-ignored only` and `cargo run --example youtube`. The writes are the requests YouTube's web
client sends (see `src/actions.rs`), sent with RustyPipe's cookie authentication; their bodies are
unit-tested, their effect on an account is not.

Absent stays absent: what YouTube does not give (views, dates or durations missing from some
lists) is `None`, never `0`. Dates YouTube only gives as "3 days ago" carry
`Precision::Relative`.

## Streams, and the SABR risk

YouTube is moving the adaptive formats of its web and TV clients to **SABR** (server-driven
streaming, without a URL per format). The connector asks RustyPipe's default clients first; when
they fail on the player script or give no direct stream, it falls back to the **iOS** client,
which still returns direct `googlevideo.com` URLs up to 2160p (AV1, VP9, H.264; Opus and AAC)
without a PO token, then to the **Android VR** client, which gives the HLS manifest of lives.
Formats without a URL and DRM-protected formats are left out.

This depends on clients YouTube may retire without notice. Age-restricted, members-only and
Premium playback need an authenticated web or TV player, which today only offers SABR: they
come back `Unavailable`. A SABR client (RustyPipe's `abr-streaming` work) is the long-term fix.

Playback reporting uses the `playbackTracking` URLs of an authenticated `/player` answer (RustyPipe
drops them from its model, so the connector reads them from a raw request and keeps them in
`Playable::tracking`): `Started` pings `videostatsPlaybackUrl`, progress and stop ping
`videostatsWatchtimeUrl` with the watched segment.

## Signing in with cookies.txt

> **Warning.** A `cookies.txt` for youtube.com is your **whole Google session** (Gmail, Drive,
> payments…) for as long as it stays valid. Keep it on your own machine, never paste it into a
> chat, an issue or a bug report, and never commit it.

1. Open a **fresh private window** and sign in on <https://www.youtube.com>.
2. Export the cookies of `youtube.com` in **Netscape** format with an exporter that works locally,
   e.g. "Get cookies.txt LOCALLY"
   ([Firefox](https://addons.mozilla.org/firefox/addon/get-cookies-txt-locally/),
   [Chromium](https://chromewebstore.google.com/detail/get-cookiestxt-locally/cclelndahbckbenkjhflpdbgdldlbecc)).
3. **Close the private window right away**: YouTube rotates the cookies of an open session and
   the exported copy would stop working.
4. Hand the file to Flâneur (`Credentials::Cookies`), then delete it.

What the connector does with it:

- The session secret it returns is the `youtube.com` lines of the file only; cookies of other
  Google domains are dropped. The app keeps that secret in the OS keyring.
- The connector itself writes nothing to disk. RustyPipe would save the cookie in
  `rustypipe_cache.json`: it gets a cache storage that strips the cookie and any OAuth token
  before anything is kept, in memory or in `Config::cache_file`. RustyPipe's error reports (raw
  answers, which hold account data once signed in) are turned off.
- Cookie values and `SAPISIDHASH` headers are redacted from every error message.
- `sign_out()` drops the session from memory; it does not sign the browser session out. To
  revoke it, sign that session out in your Google Account (Security → Your devices).

## Dependencies

RustyPipe is pinned to revision `597db17` of its unreleased `feat/deobf-extractor` branch. The
latest release, 0.11.4 (April 2025), no longer parses YouTube's player script, public playlists,
the new list layouts (views, dates and durations), channel Shorts tabs or subscriber counts; that
branch fixes all of them. Back to crates.io as soon as a release carries these fixes.

The YouTube Data API v3 is not used: it has no history, Watch Later or feeds, and its quota
(10,000 units a day per Google Cloud project, shared by all its users; 50 units per write) rules
out a shared key. At most it could later be an opt-in "bring your own project" path for likes,
comments and subscriptions.

## Your account stays on your machine

Signing in uses your own browser cookies. They are stored in your operating system's keyring and
sent only to the service they belong to, from your own IP address. No Flâneur server ever sees
them, because there is no Flâneur server.

## License note

This crate is AGPL-3.0-or-later. It links [RustyPipe](https://codeberg.org/ThetaDev/rustypipe),
which is GPL-3.0-only; section 13 of both licenses allows the combination, and a binary that
includes this connector must be distributed under the terms of both.

## Development

See [AGENTS.md](AGENTS.md) for the conventions and the commands. The shared dev container lives in
[`flaneur-app/.dev`](https://github.com/flaneur-app/.dev).

```bash
cargo run --example youtube                       # anonymous tour against live YouTube
FLANEUR_YT_COOKIES=~/secrets/yt.txt cargo run --example account -- history
cargo nextest run --run-ignored only              # live tests
```

## License

[AGPL-3.0-or-later](LICENSE).
