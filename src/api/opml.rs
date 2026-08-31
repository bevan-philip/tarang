use axum::{
    Json,
    extract::{Multipart, State},
    http::StatusCode,
};
use std::collections::HashMap;

use super::{
    AppError,
    feed::{PostFeedReq, post_feed},
};
use crate::{
    AppState,
    database::{self, StarredArticles, list_categories, list_feeds, list_starred_articles},
    opml::{self, export_opml},
};

pub async fn upload_opml(
    State(AppState { db, http }): State<AppState>,
    mut multipart: Multipart,
) -> Result<StatusCode, AppError> {
    while let Some(field) = multipart.next_field().await? {
        let opml_bytes = field.bytes().await?;
        let opml = str::from_utf8(&opml_bytes)?;

        let feeds = opml::parse_opml(opml).await?;
        let categories = list_categories(&db).await?;

        let mut category_map: HashMap<String, i64> =
            categories.into_iter().map(|c| (c.name, c.pk)).collect();

        for feed in feeds {
            let Some(category) = feed.category.as_deref() else {
                let add_feed = PostFeedReq {
                    name: feed.name,
                    url: feed.url,
                    category_id: None,
                    metadata: None,
                    refresh_interval: None,
                };
                let _ = post_feed(
                    State(AppState {
                        db: db.clone(),
                        http: http.clone(),
                    }),
                    Json(add_feed),
                )
                .await;

                continue;
            };

            if !category_map.contains_key(category) {
                let new_category = database::create_category(&db, category).await?;
                category_map.insert(new_category.name, new_category.pk);
            }

            let add_feed = PostFeedReq {
                name: feed.name,
                url: feed.url,
                category_id: Some(category_map[&feed.category.unwrap()]),
                metadata: None,
                refresh_interval: None,
            };
            let _ = post_feed(
                State(AppState {
                    db: db.clone(),
                    http: http.clone(),
                }),
                Json(add_feed),
            )
            .await;
        }
    }

    Ok(StatusCode::CREATED)
}

pub async fn get_opml(State(AppState { db, .. }): State<AppState>) -> Result<String, AppError> {
    let feeds = list_feeds(&db).await?;
    let categories = list_categories(&db).await?;

    Ok(export_opml(feeds, categories).await?)
}

pub async fn get_export_starred_articles(
    State(AppState { db, .. }): State<AppState>,
) -> Result<Json<Vec<StarredArticles>>, AppError> {
    let articles = list_starred_articles(&db).await?;

    Ok(Json(articles))
}
