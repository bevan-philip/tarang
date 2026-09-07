---
name: tarang
description: Understand and interact with a running Tarang instance — a headless, API-only RSS/Atom reader. Use this whenever the user mentions Tarang, asks to add/manage feeds, categories, articles, read/starred state, or filters against a self-hosted feed reader, wants to build a UI or automation on top of a Tarang server, or asks about Tarang's data model, its native API, or its Google-Reader-compatible ("greader") API. Also trigger this for tasks like "hide articles matching X", "mark everything as read", "export my feeds as OPML", or "why isn't this article showing up" against a Tarang backend — even if the user doesn't say the word "Tarang" but the context (config.toml with a `[server]`/`[database]` section, an `/api.json`/`/tarang/v1/...` URL, a `greader_hidden` field) makes clear that's what they mean.
---

# Tarang

Tarang is a headless RSS/Atom reader: a sync engine, a SQLite-backed data
store, and an HTTP API — nothing else. There is no bundled UI and no
built-in opinion about how you should present feeds to a human. The whole
point of the project is that *you* (an agent, a script, a UI you write) are
the client, and Tarang's job is just to fetch feeds on a schedule and expose
what it stored in a clean, fully self-describing API. It's single-tenant and
assumes a trusted network — there's no user/auth model to reason about.

Keep that framing in mind: don't reach for a browser or expect a web app to
click around in. Reach for HTTP requests against the API.

## Step 1: always discover the API from the live instance

**Never assume an endpoint's request/response shape from this document.**
Tarang's native API is generated at runtime from the actual route
definitions via `aide`, so it can't drift out of sync with reality —
but *this skill file can*. Treat the running instance as the only source
of truth for wire formats:

- `GET {base_url}/api.json` — the full OpenAPI 3 spec: every route, its
  parameters, and the JSON Schema for every request and response body.
  Fetch this first, before constructing any request, so you're working
  from the real current contract rather than a guess.
- `GET {base_url}/docs` — the same spec rendered as a browsable Swagger UI,
  useful if a human is driving alongside you.

`base_url` is wherever the operator has it bound (`config.toml`'s
`[server]` section sets host/port; there's no fixed default to assume).

This skill file exists to give you the *concepts* — the data model, why
things are shaped the way they are, and features that are easy to miss
(filters, the starred bypass, the greader shim) — not a copy of the
schema. Use `/api.json` for the exact contract and this file for the
mental model that makes that contract make sense.

## The data model

Everything hangs off two core entities:

- **Feed** — a subscribed URL, with a `name`, optional `category`
  (a feed belongs to at most one category/folder), a `refresh_interval`,
  and freeform `metadata` (Tarang deliberately doesn't impose a schema
  here — "express feeds in whatever manner you want" is a stated design
  goal, so use it for whatever your client needs to tag onto a feed).
- **Article** — one item pulled from a feed. Identity/dedup is by the
  feed's `guid`, not URL (URLs shift under tracking params and redirects;
  guids don't), so re-fetching a feed upserts rather than duplicates.

State that's *about* an article but not part of its content lives
separately:

- **Article state** (`is_read` / `is_starred`) — one row per article,
  created lazily on first mutation. Every article-listing endpoint
  returns these flags inline, so you don't need a second request to know
  what's read.
- **Filter** — a standing rule that hides matching articles from normal
  listings without deleting them. See below — this is one of the more
  interesting parts of the system.
- **Category** — just a name; a lightweight folder for feeds.

A feed's articles disappearing from a listing almost always means either
a filter matched them (see below) or they're simply older than the
listing's page/limit — check filters before assuming a bug.

## Two API surfaces — use the native one

Tarang mounts two independent routers over the same underlying data:

1. **`/tarang/v1/...`** — the native API. Self-documenting via `/api.json`
   as described above. This is the one you want for anything you're
   building yourself: adding feeds, reading articles, managing filters,
   OPML import/export, marking read/starred.
2. **`/greader/...`** — a Google Reader–protocol compatibility shim, for
   plugging in existing third-party GReader-client RSS apps (mobile
   readers, etc.) that only know how to speak that older protocol. It is
   *not* part of `/api.json` (it predates/sidesteps the `aide` router),
   uses its own stream-id encoding for feeds/categories/articles, and its
   auth is a formality — every client gets a fixed token and nothing is
   actually checked, consistent with Tarang's single-tenant, trusted-network
   design.

Only reach for `/greader/...` if the task is specifically about
interoperating with a GReader-protocol client. Otherwise default to the
native API — it's simpler, typed, and the one that stays in sync with
`/api.json`.

A feed can be marked `greader_hidden`, which removes it from the greader
surface only — it stays fully visible and manageable through the native
API. That's the mechanism for "I want this feed for my own
scripts/agents, not for my phone's RSS app."

## Filters — the feature worth knowing about

Filters let you suppress articles you don't want to see again, without
touching the underlying data. A filter is:

- **`field`**: `title`, `content`, or `both` — which part of the article
  to test (checked independently per field, not concatenated).
- **`match_type`**: `contains` (case-insensitive substring) or `regex`.
- **`pattern`**: the text or regex.
- **`feeds`** (optional): a list of feed ids to scope the filter to. Leave
  it empty/omitted and the filter is global — it applies everywhere. This
  per-feed scoping is a recent addition, so it's worth knowing it exists:
  you can have a rule like "hide anything mentioning 'sponsored'" scoped
  to just one noisy feed, without affecting the rest of your subscriptions.

Creating or updating a filter re-sweeps matching articles immediately and
returns which articles just got hidden, so you can verify a rule did what
you expected right away rather than guessing.

Disabling a filter (`enabled: false`) is instant and non-destructive — it
stops being enforced without losing its recorded matches, so re-enabling
it doesn't require a fresh sweep.

## Practical tips

- Prefer building up a request against the real `/api.json` schema rather
  than pattern-matching examples from memory — field names, optionality,
  and even which endpoints exist can change as Tarang evolves.
- Read/starred updates via `PATCH /tarang/v1/article/{id}` work even on
  filtered-out articles — filtering only affects *listing* endpoints, not
  direct access by id.
- If you're wiring up a new UI or automation from scratch, `GET
  /tarang/v1/summary` is normally the right first call — it returns
  categories, feeds, and a handful of recent article previews per feed in
  one shot, which is enough to render a starting screen before drilling
  into any single feed.
