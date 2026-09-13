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
                <title>Test Feed</title>
                <link>https://example.com</link>
                <item>
                    <title>Article One</title>
                    <link>https://example.com/one</link>
                    <guid>guid-one</guid>
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
async fn quickadd_list_contents_edit_tag_and_unread_count_flow() {
    let (base, app_server) = spawn_app().await;
    let (feed_base, feed_server) = spawn_feed_server().await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let feed_url = format!("{feed_base}/feed.xml");

    let quickadd = client
        .post(format!("{base}/greader/reader/api/0/subscription/quickadd"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form_encode(&[("quickadd", feed_url.as_str())]))
        .send()
        .await
        .unwrap();
    assert_eq!(quickadd.status(), 200);
    let quickadd_body: serde_json::Value =
        serde_json::from_str(&quickadd.text().await.unwrap()).unwrap();
    let stream_id = quickadd_body["streamId"].as_str().unwrap().to_string();
    assert_eq!(quickadd_body["streamName"], "Test Feed");

    let list = client
        .get(format!("{base}/greader/reader/api/0/subscription/list"))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), 200);
    let list_body: serde_json::Value = serde_json::from_str(&list.text().await.unwrap()).unwrap();
    let subscriptions = list_body["subscriptions"].as_array().unwrap();
    assert_eq!(subscriptions.len(), 1);
    assert_eq!(subscriptions[0]["id"], stream_id);

    let contents = client
        .get(format!(
            "{base}/greader/reader/api/0/stream/contents/{stream_id}"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(contents.status(), 200);
    let contents_body: serde_json::Value =
        serde_json::from_str(&contents.text().await.unwrap()).unwrap();
    let items = contents_body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"], "Article One");
    let item_id = items[0]["id"].as_str().unwrap().to_string();

    let edit_tag = client
        .post(format!("{base}/greader/reader/api/0/edit-tag"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form_encode(&[
            ("i", item_id.as_str()),
            ("a", "user/-/state/com.google/read"),
        ]))
        .send()
        .await
        .unwrap();
    assert_eq!(edit_tag.status(), 200);

    let unread_count = client
        .get(format!("{base}/greader/reader/api/0/unread-count"))
        .send()
        .await
        .unwrap();
    assert_eq!(unread_count.status(), 200);
    let unread_body: serde_json::Value =
        serde_json::from_str(&unread_count.text().await.unwrap()).unwrap();
    let total = unread_body["unreadcounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "user/-/state/com.google/reading-list")
        .unwrap();
    assert_eq!(total["count"], 0);

    app_server.abort();
    feed_server.abort();
}

fn form_encode(pairs: &[(&str, &str)]) -> String {
    form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}
