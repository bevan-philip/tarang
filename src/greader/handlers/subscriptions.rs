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

            let html_url = feed.html_url().to_string();

            Subscription {
                id: StreamId::Feed(FeedRef::Pk(feed.pk)).to_string(),
                title: feed.name,
                categories,
                html_url,
                url: feed.url,
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

            let label_lookup = match &category_change {
                CategoryChangeIntent::Add(label) => {
                    Some(get_or_create_category_pk(&db, label).await?)
                }
                CategoryChangeIntent::RemoveIfCurrent(label) => {
                    get_category_by_name(&db, label).await?.map(|c| c.pk)
                }
                CategoryChangeIntent::None => None,
            };
            let new_category =
                commands::resolve_category_change(&category_change, feed.category, label_lookup);

            if title.is_some() || new_category.is_some() {
                update_feed(
                    &db,
                    pk,
                    title.as_deref(),
                    None,
                    None,
                    None,
                    new_category,
                    None,
                )
                .await?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
            http: reqwest::Client::builder().no_proxy().build().unwrap(),
        }
    }

    async fn spawn_test_feed_server() -> (String, tokio::task::JoinHandle<()>) {
        use axum::{Router, routing::get};

        let app = Router::new().route(
            "/feed",
            get(|| async {
                r#"<rss version="2.0"><channel>
                    <title>Feed</title>
                    <link>https://example.com</link>
                    <item>
                        <title>Item</title>
                        <link>https://example.com/item</link>
                        <pubDate>Mon, 01 Jan 2026 00:00:00 GMT</pubDate>
                    </item>
                </channel></rss>"#
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, server)
    }

    #[tokio::test]
    async fn tag_list_includes_starred_and_category_folders() {
        let state = test_state().await;
        let category = create_category(&state.db, "News").await.unwrap();
        // GReaderVisible scope only surfaces categories with a visible feed.
        crate::database::create_feed(
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

        let Json(resp) = tag_list(State(state.clone())).await.unwrap();
        assert!(resp.tags.iter().any(|t| t.id == StreamId::Starred.to_string()));
        assert!(resp.tags.iter().any(|t| {
            t.id == StreamId::Label("News".to_string()).to_string()
                && t.kind.as_deref() == Some("folder")
        }));
    }

    #[tokio::test]
    async fn subscription_list_includes_feed_with_category() {
        let state = test_state().await;
        let category = create_category(&state.db, "News").await.unwrap();
        let feed = crate::database::create_feed(
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

        let Json(resp) = subscription_list(State(state.clone())).await.unwrap();
        assert_eq!(resp.subscriptions.len(), 1);
        let sub = &resp.subscriptions[0];
        assert_eq!(sub.id, StreamId::Feed(FeedRef::Pk(feed.pk)).to_string());
        assert_eq!(sub.categories.len(), 1);
        assert_eq!(sub.categories[0].label, "News");
    }

    #[tokio::test]
    async fn subscription_quickadd_creates_feed() {
        let state = test_state().await;
        let (base, server) = spawn_test_feed_server().await;

        let query = format!("quickadd={base}/feed");
        let params = MergedParams::from_query(&query);
        let Json(resp) = subscription_quickadd(State(state.clone()), params)
            .await
            .unwrap();
        assert_eq!(resp.num_results, 1);
        assert_eq!(resp.stream_name, "Feed");

        server.abort();
    }

    #[tokio::test]
    async fn subscription_quickadd_already_subscribed_falls_back_to_lookup() {
        let state = test_state().await;
        let (base, server) = spawn_test_feed_server().await;
        let url = format!("{base}/feed");

        crate::feed::create_feed_with_articles(
            &state.db,
            &state.http,
            &url,
            crate::feed::FeedOptions::default(),
        )
        .await
        .unwrap();

        let query = format!("quickadd={url}");
        let params = MergedParams::from_query(&query);
        let Json(resp) = subscription_quickadd(State(state.clone()), params)
            .await
            .unwrap();
        assert_eq!(resp.num_results, 1);
        assert_eq!(resp.stream_name, "Feed");

        let feeds = crate::database::list_feeds(&state.db, crate::database::FeedScope::All)
            .await
            .unwrap();
        assert_eq!(feeds.len(), 1, "the conflicting create must not duplicate the feed");

        server.abort();
    }

    #[tokio::test]
    async fn subscription_edit_subscribe_creates_feed() {
        let state = test_state().await;
        let (base, server) = spawn_test_feed_server().await;
        let url = format!("{base}/feed");

        let query = format!("ac=subscribe&s=feed/{url}");
        let params = MergedParams::from_query(&query);
        subscription_edit(State(state.clone()), params)
            .await
            .unwrap();

        let feeds = crate::database::list_feeds(&state.db, crate::database::FeedScope::All)
            .await
            .unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].url, url);

        server.abort();
    }

    #[tokio::test]
    async fn subscription_edit_unsubscribe_drops_feed() {
        let state = test_state().await;
        let feed = crate::database::create_feed(
            &state.db,
            "Feed",
            "https://example.com/feed",
            "",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();

        let query = format!("ac=unsubscribe&s=feed/{}", feed.pk);
        let params = MergedParams::from_query(&query);
        subscription_edit(State(state.clone()), params)
            .await
            .unwrap();

        let found = list_feed(&state.db, feed.pk).await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn subscription_edit_title_only_updates_title() {
        let state = test_state().await;
        let feed = crate::database::create_feed(
            &state.db,
            "Feed",
            "https://example.com/feed",
            "",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();

        let query = format!("ac=edit&s=feed/{}&t=New+Title", feed.pk);
        let params = MergedParams::from_query(&query);
        subscription_edit(State(state.clone()), params)
            .await
            .unwrap();

        let updated = list_feed(&state.db, feed.pk).await.unwrap().unwrap();
        assert_eq!(updated.name, "New Title");
    }

    #[tokio::test]
    async fn subscription_edit_category_change_moves_feed() {
        let state = test_state().await;
        let feed = crate::database::create_feed(
            &state.db,
            "Feed",
            "https://example.com/feed",
            "",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();

        let query = format!("ac=edit&s=feed/{}&a=user/-/label/News", feed.pk);
        let params = MergedParams::from_query(&query);
        subscription_edit(State(state.clone()), params)
            .await
            .unwrap();

        let updated = list_feed(&state.db, feed.pk).await.unwrap().unwrap();
        let category = get_category_by_name(&state.db, "News")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.category, Some(category.pk));
    }

    #[tokio::test]
    async fn rename_tag_renames_category() {
        let state = test_state().await;
        create_category(&state.db, "News").await.unwrap();

        let query = "s=user/-/label/News&dest=user/-/label/Tech";
        let params = MergedParams::from_query(query);
        rename_tag(State(state.clone()), params).await.unwrap();

        assert!(
            get_category_by_name(&state.db, "News")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            get_category_by_name(&state.db, "Tech")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn disable_tag_drops_category() {
        let state = test_state().await;
        create_category(&state.db, "News").await.unwrap();

        let query = "s=user/-/label/News";
        let params = MergedParams::from_query(query);
        disable_tag(State(state.clone()), params).await.unwrap();

        assert!(
            get_category_by_name(&state.db, "News")
                .await
                .unwrap()
                .is_none()
        );
    }
}
