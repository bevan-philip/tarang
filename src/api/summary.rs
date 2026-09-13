use axum::extract::State;
use axum_jsonschema::Json;
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::HashMap;

use super::AppError;
use crate::{
    AppState,
    database::{self, ArticlePreview, Category, Feed, list_article_previews_for_feeds},
};

#[derive(Serialize, JsonSchema)]
pub struct Summary {
    categories: Vec<Category>,
    feeds: Vec<FeedOutline>,
}

#[derive(Serialize, JsonSchema)]
pub struct FeedOutline {
    #[serde(flatten)]
    #[schemars(flatten)]
    feed: Feed,
    category: Option<Category>,
    articles: Vec<ArticlePreview>,
}

pub async fn get_summary(
    State(AppState { db, .. }): State<AppState>,
) -> Result<Json<Summary>, AppError> {
    let feeds = database::list_feeds(&db, crate::database::FeedScope::All).await?;

    let categories = database::list_categories(&db, crate::database::FeedScope::All).await?;

    let mut articles_by_feed: HashMap<i64, Vec<ArticlePreview>> = HashMap::new();
    for article in list_article_previews_for_feeds(&db, 10).await? {
        articles_by_feed
            .entry(article.feed)
            .or_default()
            .push(article);
    }

    let category_by_pk: HashMap<i64, Category> =
        categories.iter().map(|c| (c.pk, c.clone())).collect();

    let feed_with_articles: Vec<FeedOutline> = feeds
        .into_iter()
        .map(|feed| FeedOutline {
            category: feed
                .category
                .and_then(|pk| category_by_pk.get(&pk).cloned()),
            articles: articles_by_feed.remove(&feed.pk).unwrap_or_default(),
            feed,
        })
        .collect();

    Ok(Json(Summary {
        categories,
        feeds: feed_with_articles,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{Db, ParsedArticle, create_articles, create_category, create_feed};
    use chrono::Utc;

    async fn test_state() -> AppState {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        AppState {
            db: Db {
                read: pool.clone(),
                write: pool,
            },
            http: reqwest::Client::new(),
            discovery: Default::default(),
        }
    }

    #[tokio::test]
    async fn get_summary_groups_articles_under_their_feed() {
        let state = test_state().await;
        let category = create_category(&state.db, "News").await.unwrap();
        let feed = create_feed(
            &state.db,
            "Feed",
            "https://example.com/feed",
            "",
            Some(category.pk),
            None,
            None,
            false,
        )
        .await
        .unwrap();
        create_articles(
            &state.db,
            feed.pk,
            &[ParsedArticle {
                url: "https://example.com/article".into(),
                guid: "guid-1".into(),
                title: Some("Title".into()),
                content: "content".into(),
                summary: None,
                published_at: Utc::now(),
            }],
            &[],
        )
        .await
        .unwrap();

        let Json(summary) = get_summary(State(state.clone())).await.unwrap();
        assert_eq!(summary.categories.len(), 1);
        assert_eq!(summary.feeds.len(), 1);
        assert_eq!(summary.feeds[0].feed.pk, feed.pk);
        assert_eq!(summary.feeds[0].category.as_ref().unwrap().pk, category.pk);
        assert_eq!(summary.feeds[0].articles.len(), 1);
    }
}
