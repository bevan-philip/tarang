use crate::database;
use crate::database::Article;
use crate::database::Db;
use crate::database::DbError;
use crate::database::ParsedArticle;

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
) -> FeedResult<Vec<Article>> {
    let (_title, articles) = get_feed_articles(client, feed_url).await?;
    let db_entries = database::create_articles(db, feed_pk, &articles).await?;

    Ok(db_entries)
}

pub async fn create_feed_with_articles(
    db: &Db,
    http: &reqwest::Client,
    name: Option<&str>,
    url: &str,
    category: Option<i64>,
    metadata: Option<&str>,
    refresh_interval: Option<i64>,
) -> FeedResult<database::Feed> {
    let (parsed_title, articles) = get_feed_articles(http, url).await?;
    let name = name
        .map(str::to_owned)
        .or(parsed_title)
        .unwrap_or_else(|| url.to_string());

    let feed = database::create_feed(db, &name, url, category, metadata, refresh_interval).await?;
    database::create_articles(db, feed.pk, &articles).await?;

    Ok(feed)
}

pub async fn get_feed_articles(
    client: &reqwest::Client,
    feed_url: &str,
) -> Result<(Option<String>, Vec<ParsedArticle>), FeedError> {
    let res = client.get(feed_url).send().await?.text().await?;
    let feed = parser::Builder::new()
        .id_generator(|links, _title, _uri| {
            links.first().map(|l| l.href.clone()).unwrap_or_default()
        })
        .build()
        .parse(res.as_bytes())?;
    let title = feed.title.as_ref().map(|t| t.content.clone());
    let articles = process_feed(feed).await;
    Ok((title, articles))
}

async fn process_feed(feed: ParsedFeed) -> Vec<ParsedArticle> {
    let mut v: Vec<ParsedArticle> = Vec::new();

    for entry in feed.entries {
        if let Some(article) = create_parsed_article(entry).await {
            v.push(article);
        }
    }

    v
}

async fn create_parsed_article(entry: Entry) -> Option<ParsedArticle> {
    let Some(link) = entry.links.first() else {
        tracing::warn!(entry_id = ?entry.id, "skipping entry: no link");
        return None;
    };

    let Some(published_at) = entry.published.or(entry.updated) else {
        tracing::warn!(url = %link.href, "skipping entry: no published or updated date");
        return None;
    };

    let summary = entry.summary.as_ref().map(|s| s.content.clone());

    let content = entry
        .content
        .and_then(|c| c.body)
        .or_else(|| summary.clone())
        .unwrap_or_default();

    let title = entry.title.map(|t| t.content);

    Some(ParsedArticle {
        url: link.href.clone(),
        guid: entry.id,
        title,
        content,
        summary,
        published_at,
    })
}
