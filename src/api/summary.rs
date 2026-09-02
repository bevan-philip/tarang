use axum::extract::State;
use axum_jsonschema::Json;
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::HashMap;

use super::AppError;
use crate::{
    AppState,
    database::{self, Article, Category, Feed, list_articles_for_feeds},
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
    articles: Vec<Article>,
}

pub async fn get_summary(
    State(AppState { db, .. }): State<AppState>,
) -> Result<Json<Summary>, AppError> {
    let feeds = database::list_feeds(&db).await?;

    let categories = database::list_categories(&db).await?;

    let mut articles_by_feed: HashMap<i64, Vec<Article>> = HashMap::new();
    for article in list_articles_for_feeds(&db, 10).await? {
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
