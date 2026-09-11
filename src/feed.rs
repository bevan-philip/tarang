use std::collections::HashMap;

use crate::database;
use crate::database::Article;
use crate::database::Db;
use crate::database::DbError;
use crate::database::ParsedArticle;
use crate::filter::{self, CompiledFilter};

use feed_rs::{model::Entry, model::Feed as ParsedFeed, parser};

#[derive(Debug, thiserror::Error)]
pub enum FeedError {
    #[error("failed to fetch feed: {0}")]
    Fetch(#[from] reqwest::Error),
    #[error("failed to parse feed: {0}")]
    Parse(#[from] feed_rs::parser::ParseFeedError),
    #[error(transparent)]
    Db(#[from] DbError),
}

pub type FeedResult<T> = Result<T, FeedError>;

pub async fn update_feed_articles(
    db: &Db,
    feed_pk: i64,
    feed_url: &str,
    client: &reqwest::Client,
    filters: &[CompiledFilter],
) -> FeedResult<Vec<Article>> {
    let (_title, articles) = get_feed_articles(client, feed_url).await?;
    let db_entries = database::create_articles(db, feed_pk, &articles, filters).await?;

    Ok(db_entries)
}

#[derive(Default)]
pub struct FeedOptions {
    pub name: Option<String>,
    pub category: Option<i64>,
    pub metadata: Option<String>,
    pub refresh_interval: Option<i64>,
    pub greader_hidden: bool,
}

pub async fn create_feed_with_articles(
    db: &Db,
    http: &reqwest::Client,
    url: &str,
    options: FeedOptions,
) -> FeedResult<database::Feed> {
    let (parsed_title, articles) = get_feed_articles(http, url).await?;
    let name = options
        .name
        .or(parsed_title)
        .unwrap_or_else(|| url.to_string());

    let feed = database::create_feed(
        db,
        &name,
        url,
        options.category,
        options.metadata,
        options.refresh_interval,
        options.greader_hidden,
    )
    .await?;
    let filters = filter::load_compiled_filters(db).await?;
    database::create_articles(db, feed.pk, &articles, &filters).await?;

    Ok(feed)
}

pub async fn bulk_feeds(
    http: &reqwest::Client,
    feeds: Vec<crate::opml::ImportedFeed>,
) -> Result<HashMap<crate::opml::ImportedFeed, (Option<String>, Vec<ParsedArticle>)>, FeedError> {
    let results =
        futures::future::try_join_all(feeds.iter().map(|feed| get_feed_articles(http, &feed.url)))
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

    for feed in bulk_fetch {
        let category_pk = match feed.0.category.as_deref() {
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

        let db_feed = database::create_feed(
            db,
            &feed.0.name,
            &feed.0.url,
            category_pk,
            None,
            None,
            false,
        )
        .await?;
        let filters = filter::load_compiled_filters(db).await?;
        database::create_articles(db, db_feed.pk, &feed.1.1, &filters).await?;
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
    pub articles: Vec<ParsedArticle>,
    pub skipped: Vec<SkipReason>,
}

pub fn parse_feed(bytes: &[u8]) -> Result<ParsedFeedResult, FeedError> {
    let feed = parser::Builder::new()
        .id_generator(|links, _title, _uri| {
            links.first().map(|l| l.href.clone()).unwrap_or_default()
        })
        .build()
        .parse(bytes)?;
    let title = feed.title.as_ref().map(|t| t.content.clone());
    let (articles, skipped) = process_feed(feed);
    Ok(ParsedFeedResult {
        title,
        articles,
        skipped,
    })
}

pub async fn get_feed_articles(
    client: &reqwest::Client,
    feed_url: &str,
) -> Result<(Option<String>, Vec<ParsedArticle>), FeedError> {
    let res = client.get(feed_url).send().await?.text().await?;
    let ParsedFeedResult {
        title,
        articles,
        skipped,
    } = parse_feed(res.as_bytes())?;
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
    Ok((title, articles))
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

    const RSS_HAPPY_PATH: &str = r#"<rss version="2.0"><channel>
        <title>RSS Feed</title>
        <item>
            <title>Item One</title>
            <link>https://example.com/one</link>
            <pubDate>Mon, 01 Jan 2026 00:00:00 GMT</pubDate>
            <description>Item one body</description>
        </item>
    </channel></rss>"#;

    const ATOM_HAPPY_PATH: &str = r#"<feed xmlns="http://www.w3.org/2005/Atom">
        <title>Atom Feed</title>
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
        let result = parse_feed(RSS_HAPPY_PATH.as_bytes()).unwrap();
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
        let result = parse_feed(ATOM_HAPPY_PATH.as_bytes()).unwrap();
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
            <entry>
                <id>urn:test:no-link</id>
                <updated>2026-01-01T00:00:00Z</updated>
            </entry>
        </feed>"#;
        let result = parse_feed(xml.as_bytes()).unwrap();
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
            <entry>
                <id>urn:test:no-date</id>
                <link href="https://example.com/no-date"/>
            </entry>
        </feed>"#;
        let result = parse_feed(xml.as_bytes()).unwrap();
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
            <entry>
                <link href="https://example.com/first"/>
                <link href="https://example.com/second"/>
                <updated>2026-01-01T00:00:00Z</updated>
            </entry>
        </feed>"#;
        let result = parse_feed(xml.as_bytes()).unwrap();
        assert_eq!(result.articles.len(), 1);
        assert_eq!(result.articles[0].guid, "https://example.com/first");
        assert_eq!(result.articles[0].url, "https://example.com/first");
    }

    #[test]
    fn content_falls_back_to_summary_then_empty() {
        let with_summary = r#"<feed xmlns="http://www.w3.org/2005/Atom">
            <title>Feed</title>
            <entry>
                <id>urn:test:summary-only</id>
                <link href="https://example.com/summary-only"/>
                <updated>2026-01-01T00:00:00Z</updated>
                <summary>Just a summary</summary>
            </entry>
        </feed>"#;
        let result = parse_feed(with_summary.as_bytes()).unwrap();
        assert_eq!(result.articles[0].content, "Just a summary");
        assert_eq!(
            result.articles[0].summary.as_deref(),
            Some("Just a summary")
        );

        let with_neither = r#"<feed xmlns="http://www.w3.org/2005/Atom">
            <title>Feed</title>
            <entry>
                <id>urn:test:neither</id>
                <link href="https://example.com/neither"/>
                <updated>2026-01-01T00:00:00Z</updated>
            </entry>
        </feed>"#;
        let result = parse_feed(with_neither.as_bytes()).unwrap();
        assert_eq!(result.articles[0].content, "");
        assert_eq!(result.articles[0].summary, None);
    }

    #[test]
    fn invalid_bytes_return_parse_error() {
        let result = parse_feed(b"not a feed at all");
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
                category: Some("News".to_string()),
            },
            crate::opml::ImportedFeed {
                name: "Two".to_string(),
                url: format!("{base}/two"),
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
                category: Some("Same".to_string()),
            },
            crate::opml::ImportedFeed {
                name: "Two".to_string(),
                url: format!("{base}/two"),
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
                category: None,
            },
            crate::opml::ImportedFeed {
                name: "Bad".to_string(),
                url: "http://127.0.0.1:1/unreachable".to_string(),
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
}
