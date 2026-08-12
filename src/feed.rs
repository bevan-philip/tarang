use crate::database;
use crate::database::Article;
use crate::database::Db;
use crate::database::ParsedArticle;

use feed_rs::{model::Entry, model::Feed, parser};
use std::error::Error;

pub async fn update_feed_articles(
    db: &Db,
    feed_pk: i64,
    feed_url: String,
) -> Result<Vec<Article>, Box<dyn Error + Send + Sync>> {
    let res = reqwest::get(feed_url).await?.text().await?;
    let feed = parser::Builder::new()
        .id_generator(|links, _title, _uri| {
            links.first().map(|l| l.href.clone()).unwrap_or_default()
        })
        .build()
        .parse(res.as_bytes())?;
    let articles = process_feed(feed).await?;
    let db_entries = database::create_articles(db, feed_pk, &articles).await?;

    Ok(db_entries)
}

async fn process_feed(feed: Feed) -> Result<Vec<ParsedArticle>, Box<dyn Error + Send + Sync>> {
    let mut v: Vec<ParsedArticle> = Vec::new();

    for entry in feed.entries {
        if let Some(article) = create_parsed_article(entry).await? {
            v.push(article);
        }
    }

    Ok(v)
}

async fn create_parsed_article(
    entry: Entry,
) -> Result<Option<ParsedArticle>, Box<dyn Error + Send + Sync>> {
    let Some(link) = entry.links.first() else {
        eprintln!("skipping entry {:?}: no link", entry.id);
        return Ok(None);
    };

    let Some(published_at) = entry.published.or(entry.updated) else {
        eprintln!("skipping entry {}: no published or updated date", link.href);
        return Ok(None);
    };

    let summary = entry.summary.as_ref().map(|s| s.content.clone());

    let content = entry
        .content
        .and_then(|c| c.body)
        .or_else(|| summary.clone())
        .unwrap_or_default();

    let title = entry.title.map(|t| t.content);

    Ok(Some(ParsedArticle {
        url: link.href.clone(),
        guid: entry.id,
        title,
        content,
        summary,
        published_at,
    }))
}
