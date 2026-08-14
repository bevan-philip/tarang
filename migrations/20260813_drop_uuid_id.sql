-- no-transaction

PRAGMA foreign_keys=OFF;

BEGIN TRANSACTION;

-- feed
CREATE TABLE feed_new (
    pk               INTEGER PRIMARY KEY AUTOINCREMENT,
    name             TEXT    NOT NULL,
    url              TEXT    NOT NULL UNIQUE,
    metadata         TEXT    NOT NULL DEFAULT '{}'
                             CHECK (json_valid(metadata)),
    refresh_interval INTEGER NOT NULL DEFAULT 3600
                             CHECK (refresh_interval > 0),
    last_refresh     INTEGER          DEFAULT NULL,
    next_poll_at     INTEGER          DEFAULT NULL
) STRICT;

INSERT INTO feed_new (pk, name, url, metadata, refresh_interval, last_refresh, next_poll_at)
SELECT pk, name, url, metadata, refresh_interval, last_refresh, next_poll_at FROM feed;

DROP TABLE feed;
ALTER TABLE feed_new RENAME TO feed;
CREATE INDEX idx_feed_next_poll_at ON feed (next_poll_at);

-- category
CREATE TABLE category_new (
    pk   INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT    NOT NULL UNIQUE
) STRICT;

INSERT INTO category_new (pk, name) SELECT pk, name FROM category;

DROP TABLE category;
ALTER TABLE category_new RENAME TO category;

-- article
CREATE TABLE article_new (
    pk           INTEGER PRIMARY KEY AUTOINCREMENT,
    feed         INTEGER NOT NULL REFERENCES feed (pk) ON DELETE CASCADE,
    url          TEXT    NOT NULL UNIQUE,
    content      TEXT    NOT NULL DEFAULT '',
    published_at INTEGER          DEFAULT NULL,
    retrieved_at INTEGER NOT NULL DEFAULT (unixepoch()),
    title        TEXT             DEFAULT NULL,
    summary      TEXT             DEFAULT NULL,
    guid         TEXT    NOT NULL DEFAULT ''
) STRICT;

INSERT INTO article_new (pk, feed, url, content, published_at, retrieved_at, title, summary, guid)
SELECT pk, feed, url, content, published_at, retrieved_at, title, summary, guid FROM article;

DROP TABLE article;
ALTER TABLE article_new RENAME TO article;
CREATE INDEX idx_article_feed_published ON article (feed, published_at DESC);
CREATE INDEX idx_article_published      ON article (published_at DESC);
CREATE UNIQUE INDEX idx_article_guid    ON article (guid);

-- label
CREATE TABLE label_new (
    pk   INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT    NOT NULL UNIQUE
) STRICT;

INSERT INTO label_new (pk, name) SELECT pk, name FROM label;

DROP TABLE label;
ALTER TABLE label_new RENAME TO label;

PRAGMA foreign_key_check;

COMMIT;

PRAGMA foreign_keys=ON;
