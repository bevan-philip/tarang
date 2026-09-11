-- The RSS/Atom <link> element: a "pretty" URL for humans to open a feed's
-- site, as distinct from the feed's own (often ugly) XML URL. Mandatory per
-- the RSS spec, so NOT NULL here too; existing rows backfill to '' since we
-- have no site link on file for them.
ALTER TABLE feed ADD COLUMN display_url TEXT NOT NULL DEFAULT '';
