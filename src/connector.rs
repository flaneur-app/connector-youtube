// SPDX-License-Identifier: AGPL-3.0-or-later

//! The connector value: RustyPipe client, session, capabilities.

use std::path::PathBuf;
use std::sync::{Arc, PoisonError, RwLock};

use async_trait::async_trait;
use flaneur_connector::{
    Account, AuthMethod, Capabilities, Channels, Comments, Connector, Credentials, Error, Feeds,
    Library, Player, Reactions, Result, Search, ServiceInfo, Session, Subscriptions, Support,
    Target,
};
use flaneur_model::{Cursor, ItemRef, Page};
use rustypipe::client::{ClientType, RustyPipe, RustyPipeQuery};
use rustypipe::error::Error as RpError;
use rustypipe::model::paginator::Paginator;
use rustypipe::model::traits::FromYtItem;
use rustypipe::param::Language;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use url::Url;

use crate::actions::{self, Action};
use crate::session::{Redactor, cookie_header, youtube_cookie_file};
use crate::storage::{CacheStore, SharedCache};
use crate::{cursor, error, map, raw, urls};

/// How the connector is set up.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// File in which RustyPipe's cache (InnerTube client versions, deobfuscation code of the
    /// player script) is kept between runs, so start-up does not fetch them again. It never holds
    /// a cookie or a token. `None` keeps the cache in memory only.
    pub cache_file: Option<PathBuf>,
    /// Lets RustyPipe use `rustypipe-botguard` (PO tokens for the web clients) when it is
    /// installed. Off by default: today the web clients only offer SABR formats without URLs,
    /// and the iOS client, which needs no PO token, is the one that gives playable streams.
    pub botguard: bool,
}

/// The signed-in session, in memory only.
pub(crate) struct Active {
    /// `Cookie:` header for requests RustyPipe does not send (playback reports).
    pub(crate) cookie_header: String,
    pub(crate) redactor: Redactor,
}

/// The YouTube connector.
///
/// Every request goes from this machine to YouTube; the session cookie lives in memory and in the
/// [`Session`] handed to the app, nowhere else.
pub struct YouTube {
    info: ServiceInfo,
    cache: Arc<CacheStore>,
    botguard: bool,
    client: RwLock<RustyPipe>,
    session: RwLock<Option<Arc<Active>>>,
    pub(crate) http: reqwest::Client,
}

impl std::fmt::Debug for YouTube {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("YouTube")
            .field("signed_in", &self.is_signed_in())
            .finish_non_exhaustive()
    }
}

fn build_client(cache: &Arc<CacheStore>, botguard: bool) -> Result<RustyPipe> {
    let mut builder = RustyPipe::builder()
        .storage(Box::new(SharedCache(Arc::clone(cache))))
        // RustyPipe's error reports hold raw answers, which include account data once signed in.
        .no_reporter()
        // Dates and counts of listings are parsed from English text.
        .lang(Language::En);
    if !botguard {
        builder = builder.no_botguard();
    }
    builder
        .build()
        .map_err(|e| error::map(e, &Redactor::default()))
}

impl YouTube {
    /// A connector with no session.
    pub fn new(config: Config) -> Result<Self> {
        let cache = CacheStore::new(config.cache_file);
        let client = build_client(&cache, config.botguard)?;
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| Error::Network(e.to_string()))?;
        Ok(Self {
            info: ServiceInfo {
                id: map::service(),
                name: "YouTube".into(),
                auth_methods: vec![AuthMethod::Cookies],
            },
            cache,
            botguard: config.botguard,
            client: RwLock::new(client),
            session: RwLock::new(None),
            http,
        })
    }

    pub(crate) fn client(&self) -> RustyPipe {
        self.client
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// An anonymous query, or one that uses the session for what needs it.
    pub(crate) fn query(&self) -> RustyPipeQuery {
        self.client().query()
    }

    /// A query that always sends the session cookie.
    pub(crate) fn authed(&self) -> Result<RustyPipeQuery> {
        self.active()?;
        Ok(self.query().authenticated())
    }

    pub(crate) fn is_signed_in(&self) -> bool {
        self.session
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    /// The session, or [`Error::SignInRequired`].
    pub(crate) fn active(&self) -> Result<Arc<Active>> {
        self.session
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or(Error::SignInRequired)
    }

    pub(crate) fn redactor(&self) -> Redactor {
        self.active()
            .map_or_else(|_| Redactor::default(), |a| a.redactor.clone())
    }

    /// Maps a RustyPipe error, redacting the session's secrets.
    pub(crate) fn err(&self, e: RpError) -> Error {
        error::map(e, &self.redactor())
    }

    /// The next page of a listing, from its cursor.
    pub(crate) async fn next_page<T, U>(
        &self,
        cursor: &Cursor,
        map: impl FnMut(T) -> Option<U>,
    ) -> Result<Page<U>>
    where
        T: FromYtItem + DeserializeOwned,
    {
        let paginator: Paginator<T> = cursor::paginator(cursor)?;
        match paginator
            .next(self.query())
            .await
            .map_err(|e| self.err(e))?
        {
            Some(p) => Ok(cursor::page(p, false, map)),
            None => Ok(Page::last(Vec::new())),
        }
    }

    /// Sends one InnerTube write as the signed-in user.
    pub(crate) async fn send(&self, action: &Action) -> Result<Value> {
        let q = self.authed()?;
        tracing::debug!("POST /youtubei/v1/{}", action.endpoint);
        let text = q
            .raw(ClientType::Desktop, action.endpoint, &action.body)
            .await
            .map_err(|e| self.err(e))?;
        let response: Value = serde_json::from_str(&text)
            .map_err(|_| Error::Parse(format!("/{} answered no JSON", action.endpoint)))?;
        actions::check_response(&response)?;
        Ok(response)
    }

    /// Checks the cookies with YouTube and makes them the session. Returns the secret to keep
    /// (the `youtube.com` cookies only) and the account, when YouTube tells it.
    async fn activate(&self, cookies_txt: &str) -> Result<(String, Option<Account>)> {
        let header = cookie_header(cookies_txt);
        if header.is_empty() {
            return Err(Error::Service(
                "this cookies.txt holds no youtube.com cookie".into(),
            ));
        }
        let secret = youtube_cookie_file(cookies_txt);
        let redactor = Redactor::for_cookies(cookies_txt);
        let client = self.client();
        client
            .user_auth_set_cookie_txt(&secret)
            .await
            .map_err(|e| error::map(e, &redactor))?;
        let account = match client
            .query()
            .authenticated()
            .raw(ClientType::Desktop, "account/account_menu", &json!({}))
            .await
        {
            Ok(text) => serde_json::from_str::<Value>(&text)
                .ok()
                .as_ref()
                .and_then(raw::account),
            Err(e) => {
                tracing::debug!("account menu: {}", redactor.apply(&e.to_string()));
                None
            }
        };
        *self.session.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(Active {
            cookie_header: header,
            redactor,
        }));
        Ok((secret, account))
    }
}

/// The YouTube id of an item, or [`Error::NotFound`] for an item of another service.
pub(crate) fn video_id(item: &ItemRef) -> Result<&str> {
    if item.service.0 == map::SERVICE && urls::is_video_id(&item.id) {
        Ok(&item.id)
    } else {
        Err(Error::NotFound)
    }
}

#[async_trait]
impl Connector for YouTube {
    fn info(&self) -> &ServiceInfo {
        &self.info
    }

    fn capabilities(&self) -> Capabilities {
        use Support::{Anonymous, SignedIn, Unsupported};
        Capabilities {
            signed_in: self.is_signed_in(),
            // YouTube's anonymous home is empty ("try searching to get started").
            home: SignedIn,
            subscription_feed: SignedIn,
            // YouTube removed the Trending page in 2025.
            trending: Unsupported,
            // No Shorts feed (`/reel/reel_watch_sequence`) yet; channel Shorts tabs work.
            shorts: Unsupported,
            search: Anonymous,
            channels: Anonymous,
            related: Anonymous,
            history_read: SignedIn,
            history_write: SignedIn,
            playlists_read: SignedIn,
            playlists_write: SignedIn,
            subscriptions_read: SignedIn,
            subscriptions_write: SignedIn,
            reactions_write: SignedIn,
            comments_read: Anonymous,
            comments_write: SignedIn,
        }
    }

    async fn sign_in(&self, credentials: Credentials) -> Result<Session> {
        let Credentials::Cookies(cookies_txt) = credentials else {
            return Err(Error::Unsupported);
        };
        let (secret, account) = self.activate(&cookies_txt).await?;
        Ok(Session {
            service: map::service(),
            account,
            secret,
        })
    }

    async fn restore(&self, session: &Session) -> Result<()> {
        if session.service.0 != map::SERVICE {
            return Err(Error::SignInRequired);
        }
        self.activate(&session.secret).await.map(|_| ())
    }

    fn sign_out(&self) {
        // A fresh RustyPipe client holds no cookie; it shares the (secret-free) cache.
        match build_client(&self.cache, self.botguard) {
            Ok(client) => *self.client.write().unwrap_or_else(PoisonError::into_inner) = client,
            Err(e) => tracing::error!("could not rebuild the client on sign-out: {e}"),
        }
        *self.session.write().unwrap_or_else(PoisonError::into_inner) = None;
    }

    fn parse_url(&self, url: &Url) -> Option<Target> {
        urls::parse(url)
    }

    fn player(&self) -> &dyn Player {
        self
    }

    fn feeds(&self) -> Option<&dyn Feeds> {
        Some(self)
    }

    fn search(&self) -> Option<&dyn Search> {
        Some(self)
    }

    fn channels(&self) -> Option<&dyn Channels> {
        Some(self)
    }

    fn library(&self) -> Option<&dyn Library> {
        Some(self)
    }

    fn subscriptions(&self) -> Option<&dyn Subscriptions> {
        Some(self)
    }

    fn reactions(&self) -> Option<&dyn Reactions> {
        Some(self)
    }

    fn comments(&self) -> Option<&dyn Comments> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_follow_the_session() {
        let yt = YouTube::new(Config::default()).unwrap();
        let caps = yt.capabilities();
        assert!(!caps.signed_in);
        assert!(caps.search.available(false));
        assert!(caps.comments_read.available(false));
        assert!(!caps.history_read.available(caps.signed_in));
        assert!(!caps.trending.available(true));
        assert_eq!(yt.info().id.0, "youtube");
        assert_eq!(yt.info().auth_methods, [AuthMethod::Cookies]);
    }

    #[tokio::test]
    async fn writes_need_a_session() {
        let yt = YouTube::new(Config::default()).unwrap();
        let like = actions::rate("dQw4w9WgXcQ", flaneur_model::Reaction::Like);
        assert!(matches!(yt.send(&like).await, Err(Error::SignInRequired)));
    }

    #[tokio::test]
    async fn only_cookies_sign_in() {
        let yt = YouTube::new(Config::default()).unwrap();
        let password = Credentials::Password {
            login: "me".into(),
            password: "pw".into(),
        };
        assert!(matches!(
            yt.sign_in(password).await,
            Err(Error::Unsupported)
        ));
        let empty = Credentials::Cookies("# Netscape HTTP Cookie File\n".into());
        assert!(matches!(yt.sign_in(empty).await, Err(Error::Service(_))));
        assert!(!yt.is_signed_in());
    }

    #[test]
    fn foreign_items_are_not_found() {
        let other = ItemRef::new(flaneur_model::ServiceId::new("odysee"), "dQw4w9WgXcQ");
        assert!(matches!(video_id(&other), Err(Error::NotFound)));
        assert_eq!(
            video_id(&map::item_ref("dQw4w9WgXcQ")).unwrap(),
            "dQw4w9WgXcQ"
        );
    }
}
