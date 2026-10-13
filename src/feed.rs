use std::collections::HashMap;

use crate::database;
use crate::database::Db;
use crate::database::DbError;
use crate::database::ParsedArticle;
use crate::filter;
use crate::http;
use crate::poll::{self, Validators};

use feed_rs::{model::Entry, model::Feed as ParsedFeed, parser};

#[derive(Debug, thiserror::Error)]
pub enum FeedError {
    #[error("failed to fetch feed: {0}")]
    Fetch(#[from] reqwest::Error),
    #[error("failed to parse feed: {0}")]
    Parse(#[from] feed_rs::parser::ParseFeedError),
    #[error("feed has no link to its website")]
    MissingLink,
    #[error("invalid feed URL {url:?}: {reason}")]
    InvalidUrl { url: String, reason: &'static str },
    #[error("feed request failed: {}", poll::http_error_message(*status, body))]
    Http { status: u16, body: String },
    #[error(transparent)]
    Db(#[from] DbError),
}

impl FeedError {
    // Every error but a database one comes from the URL or what it served.
    pub fn is_client_error(&self) -> bool {
        !matches!(self, FeedError::Db(_))
    }
}

pub type FeedResult<T> = Result<T, FeedError>;

pub fn check_url(url: &str) -> FeedResult<()> {
    let invalid = |reason| {
        Err(FeedError::InvalidUrl {
            url: url.to_string(),
            reason,
        })
    };
    // The url crate always escapes these, so they only appear in pasted text.
    if url
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || "\"<>".contains(c))
    {
        return invalid("it contains a space, quote or other character not expected in a URL");
    }
    match reqwest::Url::parse(url) {
        Ok(parsed) if matches!(parsed.scheme(), "http" | "https") && parsed.host().is_some() => {
            Ok(())
        }
        Ok(_) => invalid("only http and https URLs with a host are supported"),
        Err(_) => invalid("it is not a valid URL"),
    }
}

#[derive(Default)]
pub struct FeedOptions {
    pub name: Option<String>,
    pub category: Option<i64>,
    pub metadata: Option<String>,
    pub refresh_interval: Option<i64>,
    pub greader_hidden: bool,
    /// A response for `url` that was already fetched (e.g. by discovery).
    pub response: Option<http::Response>,
}

pub async fn create_feed_with_articles(
    db: &Db,
    client: &reqwest::Client,
    url: &str,
    options: FeedOptions,
) -> FeedResult<database::Feed> {
    check_url(url)?;
    let fetched = fetch_feed(client, url, options.response).await?;
    let name = options
        .name
        .or(fetched.title)
        .unwrap_or_else(|| url.to_string());

    let feed = database::create_feed(
        db,
        &name,
        url,
        &fetched.link,
        options.category,
        options.metadata,
        options.refresh_interval,
        options.greader_hidden,
    )
    .await?;
    let filters = filter::load_compiled_filters(db).await?;
    database::create_articles(db, feed.pk, &fetched.articles, &filters).await?;
    database::record_poll(
        db,
        feed.pk,
        &poll::after_add(&feed, fetched.validators, crate::sync::unix_now()),
    )
    .await?;

    Ok(feed)
}

pub async fn bulk_feeds(
    client: &reqwest::Client,
    feeds: Vec<crate::opml::ImportedFeed>,
) -> FeedResult<HashMap<crate::opml::ImportedFeed, FetchedFeed>> {
    let results =
        futures::future::try_join_all(feeds.iter().map(|feed| fetch_feed(client, &feed.url, None)))
            .await?;
    Ok(feeds.into_iter().zip(results).collect())
}

pub async fn import_opml_feeds(
    db: &Db,
    http: &reqwest::Client,
    feeds: Vec<crate::opml::ImportedFeed>,
) -> FeedResult<()> {
    let bulk_fetch = bulk_feeds(http, feeds).await?;

    let mut category_map: std::collections::HashMap<String, i64> =
        database::list_categories(db, database::FeedScope::All)
            .await?
            .into_iter()
            .map(|c| (c.name, c.pk))
            .collect();

    for (imported, fetched) in bulk_fetch {
        let category_pk = match imported.category.as_deref() {
            None => None,
            Some(name) => match category_map.get(name) {
                Some(pk) => Some(*pk),
                None => {
                    let created = database::create_category(db, name).await?;
                    let pk = created.pk;
                    category_map.insert(created.name, pk);
                    Some(pk)
                }
            },
        };

        let display_url = imported.display_url.unwrap_or(fetched.link);

        let db_feed = database::create_feed(
            db,
            &imported.name,
            &imported.url,
            &display_url,
            category_pk,
            None,
            None,
            false,
        )
        .await?;
        let filters = filter::load_compiled_filters(db).await?;
        database::create_articles(db, db_feed.pk, &fetched.articles, &filters).await?;
        database::record_poll(
            db,
            db_feed.pk,
            &poll::after_add(&db_feed, fetched.validators, crate::sync::unix_now()),
        )
        .await?;
    }

    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum SkipReason {
    NoLink { entry_id: String },
    NoDate { url: String },
}

pub struct ParsedFeedResult {
    pub title: Option<String>,
    pub link: String,
    pub articles: Vec<ParsedArticle>,
    pub skipped: Vec<SkipReason>,
}

// feed-rs defaults an Atom <link>'s rel to "alternate" only when the XML
// omits the attribute (parser/atom/mod.rs), and RSS2 links always get
// rel: None - so this reliably picks the human-facing site link over a
// "self" link pointing back at the feed's own XML.
fn select_display_link(links: &[feed_rs::model::Link]) -> Option<String> {
    links
        .iter()
        .find(|l| matches!(l.rel.as_deref(), None | Some("alternate")))
        .or_else(|| links.first())
        .map(|l| l.href.clone())
}

// RSS 1.0/2.0 require a channel-level <link>, and Atom strongly recommends
// an alternate <link>; a feed with neither is treated as malformed rather
// than silently degraded.
pub fn parse_feed(bytes: &[u8], base_uri: &str) -> Result<ParsedFeedResult, FeedError> {
    let feed = parser::Builder::new()
        .base_uri(Some(base_uri))
        .id_generator(|links, _title, _uri| {
            links.first().map(|l| l.href.clone()).unwrap_or_default()
        })
        .build()
        .parse(bytes)?;
    let title = feed.title.as_ref().map(|t| t.content.clone());
    let link = select_display_link(&feed.links).ok_or(FeedError::MissingLink)?;
    let (articles, skipped) = process_feed(feed);
    Ok(ParsedFeedResult {
        title,
        link,
        articles,
        skipped,
    })
}

pub struct FetchedFeed {
    pub title: Option<String>,
    pub link: String,
    pub articles: Vec<ParsedArticle>,
    pub validators: Validators,
}

pub async fn fetch_feed(
    client: &reqwest::Client,
    url: &str,
    response: Option<http::Response>,
) -> FeedResult<FetchedFeed> {
    let response = match response {
        Some(response) => response,
        None => http::get(client, url, Default::default()).await?,
    };
    if response.status != 200 {
        return Err(FeedError::Http {
            status: response.status,
            body: poll::error_body(&response.body),
        });
    }
    let ParsedFeedResult {
        title,
        link,
        articles,
        skipped,
    } = parse_feed(&response.body, response.url.as_str())?;
    log_skipped(&skipped);
    Ok(FetchedFeed {
        title,
        link,
        articles,
        validators: Validators::from_headers(&response.headers),
    })
}

pub fn log_skipped(skipped: &[SkipReason]) {
    for reason in skipped {
        match reason {
            SkipReason::NoLink { entry_id } => {
                tracing::warn!(entry_id = %entry_id, "skipping entry: no link")
            }
            SkipReason::NoDate { url } => {
                tracing::warn!(url = %url, "skipping entry: no published or updated date")
            }
        }
    }
}

fn process_feed(feed: ParsedFeed) -> (Vec<ParsedArticle>, Vec<SkipReason>) {
    let mut articles = Vec::new();
    let mut skipped = Vec::new();

    for entry in feed.entries {
        match create_parsed_article(entry) {
            Ok(article) => articles.push(article),
            Err(reason) => skipped.push(reason),
        }
    }

    (articles, skipped)
}

fn create_parsed_article(entry: Entry) -> Result<ParsedArticle, SkipReason> {
    let Some(link) = entry.links.first() else {
        return Err(SkipReason::NoLink { entry_id: entry.id });
    };

    let Some(published_at) = entry.published.or(entry.updated) else {
        return Err(SkipReason::NoDate {
            url: link.href.clone(),
        });
    };

    let summary = entry.summary.as_ref().map(|s| s.content.clone());

    let content = entry
        .content
        .and_then(|c| c.body)
        .or_else(|| summary.clone())
        .unwrap_or_default();

    let title = entry.title.map(|t| t.content);

    Ok(ParsedArticle {
        url: link.href.clone(),
        guid: entry.id,
        title,
        content,
        summary,
        published_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE_URI: &str = "https://example.com/feed.xml";

    const RSS_HAPPY_PATH: &str = r#"<rss version="2.0"><channel>
        <title>RSS Feed</title>
        <link>https://example.com</link>
        <item>
            <title>Item One</title>
            <link>https://example.com/one</link>
            <pubDate>Mon, 01 Jan 2026 00:00:00 GMT</pubDate>
            <description>Item one body</description>
        </item>
    </channel></rss>"#;

    const ATOM_HAPPY_PATH: &str = r#"<feed xmlns="http://www.w3.org/2005/Atom">
        <title>Atom Feed</title>
        <link href="https://example.com"/>
        <entry>
            <id>urn:test:one</id>
            <title>Entry One</title>
            <link href="https://example.com/one"/>
            <updated>2026-01-01T00:00:00Z</updated>
            <content>Entry one body</content>
        </entry>
    </feed>"#;

    #[test]
    fn rss_happy_path_extracts_title_and_articles() {
        let result = parse_feed(RSS_HAPPY_PATH.as_bytes(), BASE_URI).unwrap();
        assert_eq!(result.title.as_deref(), Some("RSS Feed"));
        assert_eq!(result.articles.len(), 1);
        assert!(result.skipped.is_empty());
        let article = &result.articles[0];
        assert_eq!(article.guid, "https://example.com/one");
        assert_eq!(article.url, "https://example.com/one");
        assert_eq!(article.title.as_deref(), Some("Item One"));
    }

    #[test]
    fn atom_happy_path_extracts_title_and_articles() {
        let result = parse_feed(ATOM_HAPPY_PATH.as_bytes(), BASE_URI).unwrap();
        assert_eq!(result.title.as_deref(), Some("Atom Feed"));
        assert_eq!(result.articles.len(), 1);
        assert!(result.skipped.is_empty());
        let article = &result.articles[0];
        // Atom entries carry an explicit <id>, so feed-rs uses it directly
        // rather than falling back to the custom id_generator.
        assert_eq!(article.guid, "urn:test:one");
        assert_eq!(article.url, "https://example.com/one");
        assert_eq!(article.title.as_deref(), Some("Entry One"));
    }

    #[test]
    fn entry_with_no_link_is_skipped() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom">
            <title>Feed</title>
            <link href="https://example.com"/>
            <entry>
                <id>urn:test:no-link</id>
                <updated>2026-01-01T00:00:00Z</updated>
            </entry>
        </feed>"#;
        let result = parse_feed(xml.as_bytes(), BASE_URI).unwrap();
        assert!(result.articles.is_empty());
        assert_eq!(
            result.skipped,
            vec![SkipReason::NoLink {
                entry_id: "urn:test:no-link".to_string()
            }]
        );
    }

    #[test]
    fn entry_with_no_date_is_skipped() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom">
            <title>Feed</title>
            <link href="https://example.com"/>
            <entry>
                <id>urn:test:no-date</id>
                <link href="https://example.com/no-date"/>
            </entry>
        </feed>"#;
        let result = parse_feed(xml.as_bytes(), BASE_URI).unwrap();
        assert!(result.articles.is_empty());
        assert_eq!(
            result.skipped,
            vec![SkipReason::NoDate {
                url: "https://example.com/no-date".to_string()
            }]
        );
    }

    #[test]
    fn multiple_links_use_the_first() {
        // No <id> element, so the custom id_generator (first link's href)
        // kicks in - this is what proves it survived the extraction.
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom">
            <title>Feed</title>
            <link href="https://example.com"/>
            <entry>
                <link href="https://example.com/first"/>
                <link href="https://example.com/second"/>
                <updated>2026-01-01T00:00:00Z</updated>
            </entry>
        </feed>"#;
        let result = parse_feed(xml.as_bytes(), BASE_URI).unwrap();
        assert_eq!(result.articles.len(), 1);
        assert_eq!(result.articles[0].guid, "https://example.com/first");
        assert_eq!(result.articles[0].url, "https://example.com/first");
    }

    #[test]
    fn atom_feed_prefers_alternate_link_over_self() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom">
            <title>Feed</title>
            <link rel="self" href="https://example.com/feed.xml"/>
            <link rel="alternate" href="https://example.com/site"/>
        </feed>"#;
        let result = parse_feed(xml.as_bytes(), BASE_URI).unwrap();
        assert_eq!(result.link, "https://example.com/site");
    }

    #[test]
    fn feed_with_no_links_is_a_parse_error() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom">
            <title>Feed</title>
        </feed>"#;
        let result = parse_feed(xml.as_bytes(), BASE_URI);
        assert!(matches!(result, Err(FeedError::MissingLink)));
    }

    #[test]
    fn relative_link_is_resolved_against_base_uri() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom">
            <title>Feed</title>
            <link rel="alternate" href="../"/>
        </feed>"#;
        let result = parse_feed(xml.as_bytes(), "https://example.com/feed/atom.xml").unwrap();
        assert_eq!(result.link, "https://example.com/");
    }

    #[test]
    fn content_falls_back_to_summary_then_empty() {
        let with_summary = r#"<feed xmlns="http://www.w3.org/2005/Atom">
            <title>Feed</title>
            <link href="https://example.com"/>
            <entry>
                <id>urn:test:summary-only</id>
                <link href="https://example.com/summary-only"/>
                <updated>2026-01-01T00:00:00Z</updated>
                <summary>Just a summary</summary>
            </entry>
        </feed>"#;
        let result = parse_feed(with_summary.as_bytes(), BASE_URI).unwrap();
        assert_eq!(result.articles[0].content, "Just a summary");
        assert_eq!(
            result.articles[0].summary.as_deref(),
            Some("Just a summary")
        );

        let with_neither = r#"<feed xmlns="http://www.w3.org/2005/Atom">
            <title>Feed</title>
            <link href="https://example.com"/>
            <entry>
                <id>urn:test:neither</id>
                <link href="https://example.com/neither"/>
                <updated>2026-01-01T00:00:00Z</updated>
            </entry>
        </feed>"#;
        let result = parse_feed(with_neither.as_bytes(), BASE_URI).unwrap();
        assert_eq!(result.articles[0].content, "");
        assert_eq!(result.articles[0].summary, None);
    }

    #[test]
    fn invalid_bytes_return_parse_error() {
        let result = parse_feed(b"not a feed at all", BASE_URI);
        assert!(matches!(result, Err(FeedError::Parse(_))));
    }

    async fn test_db() -> Db {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        Db {
            read: pool.clone(),
            write: pool,
        }
    }

    fn rss_with_link(link: &str) -> String {
        format!(
            r#"<rss version="2.0"><channel>
                <title>Feed</title>
                <link>https://example.com</link>
                <item>
                    <title>Item</title>
                    <link>{link}</link>
                    <pubDate>Mon, 01 Jan 2026 00:00:00 GMT</pubDate>
                </item>
            </channel></rss>"#
        )
    }

    async fn spawn_test_server() -> (String, tokio::task::JoinHandle<()>) {
        use axum::{Router, routing::get};

        let app = Router::new()
            .route(
                "/one",
                get(|| async { rss_with_link("https://example.com/one-article") }),
            )
            .route(
                "/two",
                get(|| async { rss_with_link("https://example.com/two-article") }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, server)
    }

    #[tokio::test]
    async fn import_opml_feeds_imports_all_when_all_succeed() {
        let db = test_db().await;
        let (base, server) = spawn_test_server().await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let feeds = vec![
            crate::opml::ImportedFeed {
                name: "One".to_string(),
                url: format!("{base}/one"),
                display_url: None,
                category: Some("News".to_string()),
            },
            crate::opml::ImportedFeed {
                name: "Two".to_string(),
                url: format!("{base}/two"),
                display_url: None,
                category: Some("Tech".to_string()),
            },
        ];

        import_opml_feeds(&db, &client, feeds).await.unwrap();

        let categories = database::list_categories(&db, database::FeedScope::All)
            .await
            .unwrap();
        assert_eq!(categories.len(), 2);
        assert!(categories.iter().any(|c| c.name == "News"));
        assert!(categories.iter().any(|c| c.name == "Tech"));

        let db_feeds = database::list_feeds(&db, database::FeedScope::All)
            .await
            .unwrap();
        assert_eq!(db_feeds.len(), 2);

        server.abort();
    }

    #[tokio::test]
    async fn import_opml_feeds_dedupes_category_creation_within_batch() {
        let db = test_db().await;
        let (base, server) = spawn_test_server().await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let feeds = vec![
            crate::opml::ImportedFeed {
                name: "One".to_string(),
                url: format!("{base}/one"),
                display_url: None,
                category: Some("Same".to_string()),
            },
            crate::opml::ImportedFeed {
                name: "Two".to_string(),
                url: format!("{base}/two"),
                display_url: None,
                category: Some("Same".to_string()),
            },
        ];

        import_opml_feeds(&db, &client, feeds).await.unwrap();

        let categories = database::list_categories(&db, database::FeedScope::All)
            .await
            .unwrap();
        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].name, "Same");

        server.abort();
    }

    #[tokio::test]
    async fn import_opml_feeds_imports_nothing_if_any_feed_fails() {
        let db = test_db().await;
        let (base, server) = spawn_test_server().await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let feeds = vec![
            crate::opml::ImportedFeed {
                name: "Good".to_string(),
                url: format!("{base}/one"),
                display_url: None,
                category: None,
            },
            crate::opml::ImportedFeed {
                name: "Bad".to_string(),
                url: "http://127.0.0.1:1/unreachable".to_string(),
                display_url: None,
                category: None,
            },
        ];

        let result = import_opml_feeds(&db, &client, feeds).await;
        assert!(result.is_err());

        // Feeds are fetched concurrently up front via bulk_feeds, which
        // fails fast: one bad feed means the DB write loop never runs, so
        // even the good feed is left unpersisted.
        let db_feeds = database::list_feeds(&db, database::FeedScope::All)
            .await
            .unwrap();
        assert_eq!(db_feeds.len(), 0);

        server.abort();
    }

    #[test]
    fn check_url_accepts_plain_http_urls() {
        assert!(check_url("https://example.org/feed?atom").is_ok());
        assert!(check_url("http://example.org:8080/a/b.xml").is_ok());
        assert!(check_url("https://example.org/feed?tags=a|b&q='x'^2").is_ok());
    }

    #[test]
    fn check_url_rejects_pasted_junk() {
        for url in [
            " https://example.org/feed",
            "https://example.org/feed\n",
            "\"https://example.org/feed\"",
            "https://example.org/my feed",
            "<https://example.org/feed>",
            "ftp://example.org/feed",
            "example.org/feed",
            "",
        ] {
            assert!(
                matches!(check_url(url), Err(FeedError::InvalidUrl { .. })),
                "{url:?}"
            );
        }
    }

    #[tokio::test]
    async fn create_rejects_error_status_with_body_and_adds_nothing() {
        use axum::{Router, http::StatusCode, routing::get};

        let db = test_db().await;
        let app = Router::new().route(
            "/feed",
            get(|| async { (StatusCode::FORBIDDEN, "blocked: see /policy") }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let result =
            create_feed_with_articles(&db, &client, &format!("{base}/feed"), Default::default())
                .await;
        let Err(err @ FeedError::Http { status: 403, .. }) = result else {
            panic!("expected an HTTP error");
        };
        assert!(err.to_string().contains("blocked: see /policy"));
        let feeds = database::list_feeds(&db, database::FeedScope::All)
            .await
            .unwrap();
        assert!(feeds.is_empty());

        server.abort();
    }

    #[tokio::test]
    async fn create_schedules_next_poll_and_keeps_validators() {
        use axum::{Router, http::header, routing::get};

        let db = test_db().await;
        let app = Router::new().route(
            "/feed",
            get(|| async {
                (
                    [(header::ETAG, "\"v1\"")],
                    rss_with_link("https://example.com/a"),
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let before = crate::sync::unix_now();
        let feed =
            create_feed_with_articles(&db, &client, &format!("{base}/feed"), Default::default())
                .await
                .unwrap();
        let stored = database::list_feed(&db, feed.pk).await.unwrap().unwrap();
        assert_eq!(stored.etag.as_deref(), Some("\"v1\""));
        assert!(stored.next_poll_at.unwrap() >= before + stored.refresh_interval);
        assert!(stored.last_refresh.is_some());

        server.abort();
    }
}
