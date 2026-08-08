# tarang
A headless RSS reader. 

## Philosophy and core design priciples.
- This is a minimalist RSS reader, that is designed for API usage only. The intended way of usage is to plug a UI on top of it.
- Arbitrary metadata on each entry: express the feeds in whatever manner you want. 
- SQLite as the backing database.
- Portable.

## Running stuff
### sqlx
```bash
cargo install sqlx-cli --no-default-features --features rustls,sqlite
cargo sqlx database create
cargo sqlx migrate run
cargo sqlx prepare
```
