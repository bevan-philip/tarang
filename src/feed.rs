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
    let feed = parser::parse(res.as_bytes())?;
    let articles = process_feed(feed).await?;
    let db_entries = database::create_articles(db, feed_pk, &articles).await?;

    Ok(db_entries)
}

async fn process_feed(feed: Feed) -> Result<Vec<ParsedArticle>, Box<dyn Error + Send + Sync>> {
    let mut v: Vec<ParsedArticle> = Vec::new();

    for entry in feed.entries {
        v.push(create_parsed_article(entry).await?);
    }

    Ok(v)
}

async fn create_parsed_article(entry: Entry) -> Result<ParsedArticle, Box<dyn Error + Send + Sync>> {
    Ok(ParsedArticle {
        author: entry.authors.first().ok_or("no author")?.name.clone(),
        content: entry
            .content
            .ok_or("no content")?
            .body
            .ok_or("no content body")?,
        published_at: entry.published.ok_or("no published date")?,
        url: entry.links.first().ok_or("no link")?.href.clone(),
    })
}
