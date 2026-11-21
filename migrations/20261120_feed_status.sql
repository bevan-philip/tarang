-- no-transaction

-- Replaces the `disabled` flag with a `status` that also says why a feed is
-- not being polled. Existing rows are backfilled from `disabled`,
-- `failure_count` and `last_error`; the error text is the only record of
-- why a feed was disabled. Active feeds lose any error left over from
-- before they were resumed.

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
                             CHECK (greader_hidden IN (0, 1)),
    etag             TEXT             DEFAULT NULL,
    last_modified    TEXT             DEFAULT NULL,
    status           TEXT    NOT NULL DEFAULT 'active'
                             CHECK (status IN ('active', 'backing_off', 'rate_limited',
                                               'paused', 'too_many_errors', 'gone', 'duplicate')),
    failure_count    INTEGER NOT NULL DEFAULT 0,
    last_error       TEXT             DEFAULT NULL,
    moved_from       TEXT             DEFAULT NULL,
    redirected_to    TEXT             DEFAULT NULL,
    ttl_secs         INTEGER          DEFAULT NULL,
    skip_hours       TEXT    NOT NULL DEFAULT '',
    skip_days        TEXT    NOT NULL DEFAULT '',
    CHECK (status <> 'active' OR (failure_count = 0 AND last_error IS NULL)),
    CHECK (status NOT IN ('backing_off', 'rate_limited')
           OR (failure_count > 0 AND last_error IS NOT NULL)),
    CHECK (status NOT IN ('too_many_errors', 'gone', 'duplicate') OR last_error IS NOT NULL)
) STRICT;

INSERT INTO feed_new (pk, name, url, display_url, category, metadata, refresh_interval,
                      last_refresh, next_poll_at, greader_hidden, etag, last_modified,
                      status, failure_count, last_error, moved_from, redirected_to,
                      ttl_secs, skip_hours, skip_days)
SELECT pk, name, url, display_url, category, metadata, refresh_interval,
       last_refresh, next_poll_at, greader_hidden, etag, last_modified,
       status,
       CASE WHEN status = 'active' THEN 0 ELSE failure_count END,
       CASE WHEN status = 'active' THEN NULL ELSE last_error END,
       moved_from, redirected_to, ttl_secs, skip_hours, skip_days
FROM (
    SELECT *,
           CASE
               WHEN disabled = 1 AND last_error LIKE 'HTTP 410%' THEN 'gone'
               WHEN disabled = 1 AND last_error LIKE '%which another feed already uses' THEN 'duplicate'
               WHEN disabled = 1 AND failure_count >= 10 AND last_error IS NOT NULL THEN 'too_many_errors'
               WHEN disabled = 1 THEN 'paused'
               WHEN failure_count > 0 AND last_error LIKE 'HTTP 429%' THEN 'rate_limited'
               WHEN failure_count > 0 AND last_error IS NOT NULL THEN 'backing_off'
               ELSE 'active'
           END AS status
    FROM feed
);

DROP TABLE feed;
ALTER TABLE feed_new RENAME TO feed;
CREATE INDEX idx_feed_next_poll_at ON feed (next_poll_at);
CREATE INDEX idx_feed_category     ON feed (category);

PRAGMA foreign_key_check;

COMMIT;

PRAGMA foreign_keys=ON;
