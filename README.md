# tarang
A headless RSS reader. 

## Philosophy and core design priciples.
- This is a minimalist RSS reader, that is designed for API usage only. The intended way of usage is to plug a UI on top of it ([example](https://github.com/bevan-philip/tarangcat)).
- Arbitrary metadata on each entry: express the feeds in whatever manner you want. 
- SQLite as the backing database.
- Portable.

## Why you shouldn't use it
- It is designed for the author, not for anyone else.
- There is no authentication layer, as I run it inside a [tailnet](https://tailscale.com/docs/concepts/tailnet).
- There are many alternatives for a FOSS RSS server, like [miniflux](https://github.com/miniflux/v2) or [FreshRSS](https://www.freshrss.org/).

## Features
- Minimal REST API for building stateless RSS applications.
- GReader API shim for stateful/mobile applications, like NetNewsWire.
- OPML import/export.
- Filters (ask your agent to figure out what filter you want to make).

## Running stuff
### sqlx
```bash
cargo install sqlx-cli --no-default-features --features rustls,sqlite
cargo sqlx database create
cargo sqlx migrate run
cargo sqlx prepare --workspace -- --all-targets
```
