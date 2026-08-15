-- no-transaction

PRAGMA foreign_keys=OFF;

BEGIN TRANSACTION;

-- feed
CREATE TABLE feed_new (
    pk               INTEGER PRIMARY KEY AUTOINCREMENT,
    name             TEXT    NOT NULL,
    url              TEXT    NOT NULL UNIQUE,
    category         INTEGER          DEFAULT NULL REFERENCES category (pk) ON DELETE SET NULL,
    metadata         TEXT    NOT NULL DEFAULT '{}'
                             CHECK (json_valid(metadata)),
    refresh_interval INTEGER NOT NULL DEFAULT 3600
                             CHECK (refresh_interval > 0),
    last_refresh     INTEGER          DEFAULT NULL,
    next_poll_at     INTEGER          DEFAULT NULL
) STRICT;

INSERT INTO feed_new (pk, name, url, category, metadata, refresh_interval, last_refresh, next_poll_at)
SELECT
    feed.pk,
    feed.name,
    feed.url,
    (SELECT MIN(category) FROM feed_category WHERE feed_category.feed = feed.pk),
    feed.metadata,
    feed.refresh_interval,
    feed.last_refresh,
    feed.next_poll_at
FROM feed;

DROP TABLE feed_category;
DROP TABLE feed;
ALTER TABLE feed_new RENAME TO feed;
CREATE INDEX idx_feed_next_poll_at ON feed (next_poll_at);
CREATE INDEX idx_feed_category     ON feed (category);

PRAGMA foreign_key_check;

COMMIT;

PRAGMA foreign_keys=ON;
