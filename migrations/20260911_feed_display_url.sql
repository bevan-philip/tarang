-- no-transaction

-- The RSS/Atom <link> element: a "pretty" URL for humans to open a feed's
-- site, as distinct from the feed's own (often ugly) XML URL. Mandatory per
-- the RSS spec, so NOT NULL here too; existing rows backfill to '' since we
-- have no site link on file for them. There's exactly one production
-- database and those rows get backfilled by hand afterwards, so this is a
-- table rebuild (matching 20260813_drop_uuid_id.sql /
-- 20260815_single_category_per_feed.sql) rather than a permanent schema
-- DEFAULT.

PRAGMA foreign_keys=OFF;

BEGIN TRANSACTION;

CREATE TABLE feed_new (
    pk               INTEGER PRIMARY KEY AUTOINCREMENT,
    name             TEXT    NOT NULL,
    url              TEXT    NOT NULL UNIQUE,
    display_url      TEXT    NOT NULL,
    category         INTEGER          DEFAULT NULL REFERENCES category (pk) ON DELETE SET NULL,
    metadata         TEXT    NOT NULL DEFAULT '{}'
                             CHECK (json_valid(metadata)),
    refresh_interval INTEGER NOT NULL DEFAULT 3600
                             CHECK (refresh_interval > 0),
    last_refresh     INTEGER          DEFAULT NULL,
    next_poll_at     INTEGER          DEFAULT NULL,
    greader_hidden   INTEGER NOT NULL DEFAULT 0
                             CHECK (greader_hidden IN (0, 1))
) STRICT;

INSERT INTO feed_new (pk, name, url, display_url, category, metadata, refresh_interval, last_refresh, next_poll_at, greader_hidden)
SELECT pk, name, url, '', category, metadata, refresh_interval, last_refresh, next_poll_at, greader_hidden FROM feed;

DROP TABLE feed;
ALTER TABLE feed_new RENAME TO feed;
CREATE INDEX idx_feed_next_poll_at ON feed (next_poll_at);
CREATE INDEX idx_feed_category     ON feed (category);

PRAGMA foreign_key_check;

COMMIT;

PRAGMA foreign_keys=ON;
