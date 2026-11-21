# Flâneur connector: YouTube

The YouTube connector of [Flâneur](https://github.com/flaneur-app), built on [RustyPipe](https://codeberg.org/ThetaDev/rustypipe) (InnerTube). It implements the connector traits of [`flaneur-app/core`](https://github.com/flaneur-app/core).

> Flâneur is an independent, open-source project. It is not affiliated with, endorsed by or
> sponsored by Google, YouTube, Odysee, LBRY or any other service it can connect to. All
> trademarks belong to their owners.

## Your account stays on your machine

Signing in uses your own browser cookies or OAuth tokens. They are stored in your operating
system's keyring and sent only to the service they belong to, from your own IP address. No
Flâneur server ever sees them, because there is no Flâneur server.

## License note

This crate is AGPL-3.0-or-later. It links [RustyPipe](https://codeberg.org/ThetaDev/rustypipe),
which is GPL-3.0-only; section 13 of both licenses allows the combination, and a binary that
includes this connector must be distributed under the terms of both.

## Development

See [AGENTS.md](AGENTS.md) for the conventions and the commands. The shared dev container lives in
[`flaneur-app/.dev`](https://github.com/flaneur-app/.dev).

## License

[AGPL-3.0-or-later](LICENSE).
