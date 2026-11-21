// SPDX-License-Identifier: AGPL-3.0-or-later

//! RustyPipe's cache, kept free of secrets.
//!
//! RustyPipe caches client versions and the deobfuscation code of YouTube's player, and by default
//! writes them to `rustypipe_cache.json` **together with the session cookie and OAuth token**. A
//! connector never stores secrets (the app keeps them in the OS keyring), so the cache handed to
//! RustyPipe here strips `auth_cookie` and `oauth_token` before anything is kept, in memory or on
//! disk. The live cookie only ever exists in RustyPipe's memory and in the [`Session`] the app
//! stores.
//!
//! [`Session`]: flaneur_connector::Session

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use rustypipe::cache::CacheStorage;
use serde_json::Value;

/// Keys of RustyPipe's cache that hold credentials.
const SECRET_KEYS: [&str; 2] = ["auth_cookie", "oauth_token"];

/// The non-secret part of RustyPipe's cache, in memory and optionally in a file.
#[derive(Debug, Default)]
pub(crate) struct CacheStore {
    memory: Mutex<Option<String>>,
    file: Option<PathBuf>,
}

impl CacheStore {
    pub(crate) fn new(file: Option<PathBuf>) -> Arc<Self> {
        Arc::new(Self {
            memory: Mutex::new(None),
            file,
        })
    }
}

/// [`CacheStorage`] over a shared [`CacheStore`]: every RustyPipe client the connector builds
/// (one per session) reuses the same cache.
pub(crate) struct SharedCache(pub(crate) Arc<CacheStore>);

impl CacheStorage for SharedCache {
    fn write(&self, data: &str) {
        let Some(clean) = strip_secrets(data) else {
            return;
        };
        if let Some(path) = &self.0.file
            && let Err(e) = write_private(path, &clean)
        {
            tracing::warn!("could not write the cache file {}: {e}", path.display());
        }
        *self.0.memory.lock().unwrap_or_else(PoisonError::into_inner) = Some(clean);
    }

    fn read(&self) -> Option<String> {
        let memory = self
            .0
            .memory
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        memory
            .or_else(|| {
                let path = self.0.file.as_ref()?;
                std::fs::read_to_string(path).ok()
            })
            .and_then(|data| strip_secrets(&data))
    }
}

/// The cache JSON without its credentials; `None` when it is not the JSON object RustyPipe
/// writes (nothing unknown is ever kept).
pub(crate) fn strip_secrets(data: &str) -> Option<String> {
    let Ok(Value::Object(mut map)) = serde_json::from_str::<Value>(data) else {
        return None;
    };
    for key in SECRET_KEYS {
        map.remove(key);
    }
    serde_json::to_string(&Value::Object(map)).ok()
}

/// Writes a file readable by its owner only.
fn write_private(path: &std::path::Path, data: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(data.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CACHE: &str = r#"{"clients":{},"deobf":{"data":"x"},"auth_cookie":{"cookie":"SAPISID=very-secret"},"oauth_token":{"access_token":"tok"}}"#;

    #[test]
    fn credentials_never_reach_the_cache() {
        let store = CacheStore::new(None);
        let cache = SharedCache(store);
        cache.write(CACHE);
        let kept = cache.read().unwrap();
        assert!(!kept.contains("very-secret"));
        assert!(!kept.contains("tok"));
        assert!(kept.contains("deobf"));
    }

    #[test]
    fn credentials_never_reach_the_cache_file() {
        let dir = std::env::temp_dir().join(format!("flaneur-yt-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("cache.json");
        let cache = SharedCache(CacheStore::new(Some(file.clone())));
        cache.write(CACHE);
        let on_disk = std::fs::read_to_string(&file).unwrap();
        assert!(!on_disk.contains("very-secret"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_cache_file_written_by_someone_else_loses_its_cookie_on_read() {
        let dir = std::env::temp_dir().join(format!("flaneur-yt-cache-r-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("cache.json");
        std::fs::write(&file, CACHE).unwrap();
        let cache = SharedCache(CacheStore::new(Some(file)));
        assert!(!cache.read().unwrap().contains("very-secret"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unknown_content_is_dropped() {
        assert_eq!(strip_secrets("not json"), None);
        assert_eq!(strip_secrets("[1]"), None);
    }
}
