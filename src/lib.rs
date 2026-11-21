// SPDX-License-Identifier: AGPL-3.0-or-later

//! The YouTube connector of Flâneur, built on [RustyPipe](https://codeberg.org/ThetaDev/rustypipe)
//! (InnerTube, the API of YouTube's own clients).
//!
//! [`YouTube`] implements [`flaneur_connector::Connector`] and every facet: search, channels,
//! player, comments and feeds work anonymously; history, playlists, subscriptions, likes,
//! comment posting and playback reporting need a session from the user's `cookies.txt`
//! ([`AuthMethod::Cookies`](flaneur_connector::AuthMethod::Cookies)).
//!
//! Every request goes from the user's machine to YouTube. The session secret is the
//! `youtube.com` part of the user's `cookies.txt`: the connector hands it to the app (for the OS
//! keyring) and otherwise keeps it in memory only. RustyPipe's cache, which would store the
//! cookie in a JSON file, is given a storage that strips it (see [`Config::cache_file`]).
