# AGENTS.md — Flâneur `connector-youtube`

This repository inherits the [canonical workspace rules](https://github.com/flaneur-app/.dev/blob/main/AGENTS.md). Only project-specific details are documented here.

## Scope

The YouTube connector of [Flâneur](https://github.com/flaneur-app), built on [RustyPipe](https://codeberg.org/ThetaDev/rustypipe) (InnerTube). It implements the connector traits of [`flaneur-app/core`](https://github.com/flaneur-app/core).

**Status:** first version. Anonymous features are verified against live YouTube; signed-in reads
and all writes are implemented but unverified (see the README table). Kept in its own repository
so the YouTube-specific code stays isolated from the rest of Flâneur.

## Structure

```text
src/
├── lib.rs         crate docs, public API: YouTube, Config, SERVICE
├── connector.rs   YouTube (RustyPipe client + session), Connector impl, capabilities, sign-in
├── player.rs      Player: client fallback (default → iOS → Android VR), related, tracking fetch
├── browse.rs      Search, Channels, Feeds, Comments
├── account.rs     Library, Subscriptions, Reactions (session needed)
├── actions.rs     InnerTube write bodies (like, subscribe, edit_playlist, comments, replies)
├── tracking.rs    playback tracking token and ping URLs (history, resume position)
├── map.rs         RustyPipe model → flaneur_model (items, streams, comments, channels)
├── raw.rs         parsers for raw answers: home feed first page, account menu
├── cursor.rs      RustyPipe paginators ↔ opaque Cursor
├── urls.rs        parse_url, id validation
├── text.rs        English counts, durations, "3 days ago"
├── session.rs     cookies.txt filtering, Cookie header, secret redaction
├── storage.rs     RustyPipe cache storage that never keeps the cookie
└── error.rs       RustyPipe errors → flaneur_connector::Error
examples/
├── youtube.rs     anonymous live tour (search, resolve + select_streams, comments, channel, live)
└── account.rs     signed-in checks and writes with FLANEUR_YT_COOKIES (changes the account)
tests/
├── live.rs        live anonymous tests (#[ignore]) + offline session gates
└── fixtures/      trimmed live answers (stream URLs and visitor data scrubbed)
```

RustyPipe is pinned by `rev` to its `feat/deobf-extractor` branch (see `Cargo.toml` and the
README); `deny.toml` allows that git source and clarifies its deprecated `GPL-3.0` license id.
Before moving the pin, run the live tests and both examples.

## License

**AGPL-3.0-or-later**. Every source file starts with an SPDX header:

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
```

Dependencies must have AGPL-compatible licenses; `cargo deny check` enforces the list in `deny.toml`.
`GPL-3.0-only` and `GPL-3.0-or-later` are also allowed here: RustyPipe is GPL-3.0-only.

## Naming and legal positioning

- Never use "YouTube" or "tube" in the name of the app, a crate meant for end users, or the org.
  Service names appear only where they describe what a connector talks to.
- No ad blocking and no downloading as advertised features.
- Cookies and tokens stay on the user's machine (OS keyring). No code path sends them anywhere but
  to the service they belong to.
- Every README keeps the non-affiliation notice.

## Secrets in code

- Never log, print or put in an error a cookie, a `SAPISIDHASH` or a token: map RustyPipe errors
  with `YouTube::err` (redacts) and pass other messages through `Redactor::apply`.
- Never give RustyPipe a storage or a reporter that writes the cookie: keep `SharedCache` and
  `no_reporter()`.
- Fixtures come from anonymous answers only; scrub stream URLs (they embed the IP) and visitor
  data before committing them.

## Commit Scopes

Defined in `scopes.json`:

| Scope | Covers |
| --- | --- |
| `connector` | Connector implementation: feeds, search, channels, library, comments |
| `auth` | Cookie and OAuth session handling |
| `player` | Stream resolution and format mapping |
| `build`, `test`, `docs`, `deps`, `ci`, `ai` | As usual |

## Commands

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace --no-tests=pass
cargo deny check

# Against live YouTube (network; not in CI)
cargo nextest run --run-ignored only
cargo run --example youtube
FLANEUR_YT_COOKIES=~/secrets/yt.txt cargo run --example account -- whoami
```
