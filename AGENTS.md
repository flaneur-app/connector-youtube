# AGENTS.md — Flâneur `connector-youtube`

This repository inherits the [canonical workspace rules](https://github.com/flaneur-app/.dev/blob/main/AGENTS.md). Only project-specific details are documented here.

## Scope

The YouTube connector of [Flâneur](https://github.com/flaneur-app), built on [RustyPipe](https://codeberg.org/ThetaDev/rustypipe) (InnerTube). It implements the connector traits of [`flaneur-app/core`](https://github.com/flaneur-app/core).

**Status:** bootstrapping. Kept in its own repository so the YouTube-specific code stays isolated from the rest of Flâneur.

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
```
