use axum::Json;
use axum::extract::State;
use std::collections::HashMap;

use super::resolve_feed_pk;
use crate::AppState;
use crate::database::{
    Category, Db, DbError, create_category, drop_category, drop_feed, get_category_by_name,
    list_categories, list_feed, list_feeds, rename_category, update_feed,
};
use crate::feed::{FeedError, FeedOptions, create_feed_with_articles};
use crate::greader::GReaderError;
use crate::greader::commands::{self, CategoryChangeIntent, SubscriptionEditCommand};
use crate::greader::form::MergedParams;
use crate::greader::ids::{FeedRef, StreamId};
use crate::greader::responses::{
    QuickAddResponse, Subscription, SubscriptionCategory, SubscriptionListResponse, Tag,
    TagListResponse,
};

pub async fn tag_list(
    State(AppState { db, .. }): State<AppState>,
) -> Result<Json<TagListResponse>, GReaderError> {
    let mut tags = vec![Tag {
        id: StreamId::Starred.to_string(),
        label: None,
        kind: None,
    }];

    for category in list_categories(&db, crate::database::FeedScope::GReaderVisible).await? {
        tags.push(Tag {
            id: StreamId::Label(category.name.clone()).to_string(),
            label: Some(category.name),
            kind: Some("folder".to_string()),
        });
    }

    Ok(Json(TagListResponse { tags }))
}

pub async fn subscription_list(
    State(AppState { db, .. }): State<AppState>,
) -> Result<Json<SubscriptionListResponse>, GReaderError> {
    let feeds = list_feeds(&db, crate::database::FeedScope::GReaderVisible).await?;
    let categories = list_categories(&db, crate::database::FeedScope::GReaderVisible).await?;
    let category_by_pk: HashMap<i64, Category> =
        categories.into_iter().map(|c| (c.pk, c)).collect();

    let subscriptions = feeds
        .into_iter()
        .map(|feed| {
            let categories = feed
                .category
                .and_then(|pk| category_by_pk.get(&pk))
                .map(|c| {
                    vec![SubscriptionCategory {
                        id: StreamId::Label(c.name.clone()).to_string(),
                        label: c.name.clone(),
                    }]
                })
                .unwrap_or_default();

            Subscription {
                id: StreamId::Feed(FeedRef::Pk(feed.pk)).to_string(),
                title: feed.name,
                categories,
                url: feed.url.clone(),
                html_url: feed.url,
                icon_url: String::new(),
            }
        })
        .collect();

    Ok(Json(SubscriptionListResponse { subscriptions }))
}

pub async fn subscription_quickadd(
    State(AppState { db, http }): State<AppState>,
    params: MergedParams,
) -> Result<Json<QuickAddResponse>, GReaderError> {
    let url = params
        .get("quickadd")
        .ok_or_else(|| GReaderError::BadRequest("missing quickadd".into()))?;

    // Real clients quickadd defensively and expect a normal response on
    // "already subscribed," not a 409 — so fall back to a lookup instead
    // of surfacing the conflict.
    let feed = match create_feed_with_articles(&db, &http, url, FeedOptions::default()).await {
        Ok(feed) => feed,
        Err(FeedError::Db(DbError::AlreadyExists(_))) => crate::database::get_feed_by_url(&db, url)
            .await?
            .ok_or_else(|| GReaderError::BadRequest("feed lookup failed after conflict".into()))?,
        Err(e) => return Err(e.into()),
    };

    Ok(Json(QuickAddResponse {
        num_results: 1,
        stream_id: StreamId::Feed(FeedRef::Pk(feed.pk)).to_string(),
        stream_name: feed.name,
    }))
}

async fn get_or_create_category_pk(db: &Db, label: &str) -> Result<i64, GReaderError> {
    let category = match get_category_by_name(db, label).await? {
        Some(c) => c,
        None => create_category(db, label).await?,
    };
    Ok(category.pk)
}

pub async fn subscription_edit(
    State(AppState { db, http }): State<AppState>,
    params: MergedParams,
) -> Result<&'static str, GReaderError> {
    let action = params
        .get("ac")
        .ok_or_else(|| GReaderError::BadRequest("missing ac".into()))?;
    let stream = params
        .get("s")
        .ok_or_else(|| GReaderError::BadRequest("missing s".into()))?;

    let command = commands::parse_subscription_edit(
        action,
        stream,
        params.get("t"),
        params.get_all("a"),
        params.get_all("r"),
    )?;

    match command {
        SubscriptionEditCommand::Subscribe {
            url,
            title,
            category_label,
        } => {
            let category_pk = match category_label {
                Some(label) => Some(get_or_create_category_pk(&db, &label).await?),
                None => None,
            };

            match create_feed_with_articles(
                &db,
                &http,
                &url,
                FeedOptions {
                    name: title,
                    category: category_pk,
                    ..Default::default()
                },
            )
            .await
            {
                Ok(_) => {}
                Err(FeedError::Db(DbError::AlreadyExists(_))) => {}
                Err(e) => return Err(e.into()),
            }
        }
        SubscriptionEditCommand::Unsubscribe { feed_ref } => {
            let pk = resolve_feed_pk(&db, &feed_ref).await?;
            drop_feed(&db, pk).await?;
        }
        SubscriptionEditCommand::Edit {
            feed_ref,
            title,
            category_change,
        } => {
            let pk = resolve_feed_pk(&db, &feed_ref).await?;
            let feed = list_feed(&db, pk)
                .await?
                .ok_or_else(|| GReaderError::BadRequest(format!("feed {pk} not found")))?;

            let new_category = match category_change {
                CategoryChangeIntent::Add(label) => {
                    Some(Some(get_or_create_category_pk(&db, &label).await?))
                }
                CategoryChangeIntent::RemoveIfCurrent(label) => {
                    match get_category_by_name(&db, &label).await? {
                        Some(c) if feed.category == Some(c.pk) => Some(None),
                        _ => None,
                    }
                }
                CategoryChangeIntent::None => None,
            };

            if title.is_some() || new_category.is_some() {
                update_feed(&db, pk, title.as_deref(), None, None, new_category, None).await?;
            }
        }
    }

    Ok("OK")
}

pub async fn rename_tag(
    State(AppState { db, .. }): State<AppState>,
    params: MergedParams,
) -> Result<&'static str, GReaderError> {
    let s = params
        .get("s")
        .ok_or_else(|| GReaderError::BadRequest("missing s".into()))?;
    let dest = params
        .get("dest")
        .ok_or_else(|| GReaderError::BadRequest("missing dest".into()))?;

    let StreamId::Label(old_name) = StreamId::parse(s)? else {
        return Err(GReaderError::BadRequest("s must be a label stream".into()));
    };
    let StreamId::Label(new_name) = StreamId::parse(dest)? else {
        return Err(GReaderError::BadRequest(
            "dest must be a label stream".into(),
        ));
    };

    let category = get_category_by_name(&db, &old_name)
        .await?
        .ok_or_else(|| GReaderError::BadRequest(format!("unknown label: {old_name}")))?;

    rename_category(&db, category.pk, &new_name).await?;

    Ok("OK")
}

pub async fn disable_tag(
    State(AppState { db, .. }): State<AppState>,
    params: MergedParams,
) -> Result<&'static str, GReaderError> {
    for s in params.get_all("s") {
        if let StreamId::Label(name) = StreamId::parse(s)?
            && let Some(category) = get_category_by_name(&db, &name).await?
        {
            drop_category(&db, category.pk).await?;
        }
    }

    Ok("OK")
}
