use tarang::{AppState, build_app, database::Db};

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

async fn spawn_app() -> (String, tokio::task::JoinHandle<()>) {
    let state = AppState {
        db: test_db().await,
        http: reqwest::Client::new(),
    };
    let app = build_app(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, server)
}

async fn spawn_feed_server() -> (String, tokio::task::JoinHandle<()>) {
    use axum::{Router, routing::get};

    let app = Router::new().route(
        "/feed.xml",
        get(|| async {
            r#"<rss version="2.0"><channel>
                <title>Native Feed</title>
                <link>https://example.com</link>
                <item>
                    <title>Native Article</title>
                    <link>https://example.com/native</link>
                    <guid>guid-native</guid>
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
async fn create_feed_filter_patch_article_and_delete_feed_flow() {
    let (base, app_server) = spawn_app().await;
    let (feed_base, feed_server) = spawn_feed_server().await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let feed_url = format!("{feed_base}/feed.xml");

    let post_feed = client
        .post(format!("{base}/tarang/v1/feed"))
        .header("content-type", "application/json")
        .body(serde_json::json!({ "url": feed_url }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(post_feed.status(), 200);
    let post_feed_body: serde_json::Value =
        serde_json::from_str(&post_feed.text().await.unwrap()).unwrap();
    assert_eq!(post_feed_body["name"], "Native Feed");
    let feed_id = post_feed_body["id"].as_i64().unwrap();

    let get_feed = client
        .get(format!("{base}/tarang/v1/feed/{feed_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_feed.status(), 200);
    let get_feed_body: serde_json::Value =
        serde_json::from_str(&get_feed.text().await.unwrap()).unwrap();
    let articles = get_feed_body["articles"].as_array().unwrap();
    assert_eq!(articles.len(), 1);
    let article_id = articles[0]["pk"].as_i64().unwrap();

    let post_filter = client
        .post(format!("{base}/tarang/v1/filter"))
        .header("content-type", "application/json")
        .body(
            serde_json::json!({
                "name": "Block native",
                "match_type": "contains",
                "pattern": "Native Article",
            })
            .to_string(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(post_filter.status(), 200);
    let filter_body: serde_json::Value =
        serde_json::from_str(&post_filter.text().await.unwrap()).unwrap();
    assert_eq!(
        filter_body["matched_articles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["pk"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![article_id]
    );

    let patch_article = client
        .patch(format!("{base}/tarang/v1/article/{article_id}"))
        .header("content-type", "application/json")
        .body(serde_json::json!({ "is_read": true, "is_starred": true }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(patch_article.status(), 200);
    let patch_body: serde_json::Value =
        serde_json::from_str(&patch_article.text().await.unwrap()).unwrap();
    assert_eq!(patch_body["is_read"], true);
    assert_eq!(patch_body["is_starred"], true);

    let delete_feed = client
        .delete(format!("{base}/tarang/v1/feed/{feed_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_feed.status(), 200);

    let get_after_delete = client
        .get(format!("{base}/tarang/v1/feed/{feed_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_after_delete.status(), 404);

    app_server.abort();
    feed_server.abort();
}
