-- Optional per-feed scoping for filters. No rows for a filter means it's
-- global (applies to every feed) - the default for all existing filters, so
-- this migration needs no backfill.

CREATE TABLE filter_feed (
    filter INTEGER NOT NULL REFERENCES filter (pk) ON DELETE CASCADE,
    feed   INTEGER NOT NULL REFERENCES feed (pk) ON DELETE CASCADE,
    PRIMARY KEY (filter, feed)
) STRICT;

CREATE INDEX idx_filter_feed_feed ON filter_feed (feed);
