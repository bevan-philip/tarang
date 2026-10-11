const PROJECT_URL: &str = "https://github.com/bevan-philip/tarang";

pub fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().user_agent(user_agent(
        option_env!("TARANG_RELEASE_VERSION"),
        env!("CARGO_PKG_VERSION"),
    ))
}

fn user_agent(release_tag: Option<&str>, package_version: &str) -> String {
    let version = release_tag
        .map(|tag| tag.strip_prefix('v').unwrap_or(tag))
        .filter(|v| !v.is_empty())
        .unwrap_or(package_version);
    format!("tarang/{version} (feed reader; +{PROJECT_URL})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_agent_uses_release_tag_without_v_prefix() {
        assert_eq!(
            user_agent(Some("v1.2.3"), "0.1.0"),
            "tarang/1.2.3 (feed reader; +https://github.com/bevan-philip/tarang)"
        );
    }

    #[test]
    fn user_agent_falls_back_to_package_version() {
        assert_eq!(
            user_agent(None, "0.1.0"),
            "tarang/0.1.0 (feed reader; +https://github.com/bevan-philip/tarang)"
        );
        assert_eq!(
            user_agent(Some(""), "0.1.0"),
            "tarang/0.1.0 (feed reader; +https://github.com/bevan-philip/tarang)"
        );
    }

    #[tokio::test]
    async fn client_asks_for_compressed_responses() {
        use axum::{Router, http::HeaderMap, routing::get};

        let app = Router::new().route(
            "/",
            get(|headers: HeaderMap| async move {
                headers
                    .get("accept-encoding")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default()
                    .to_string()
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let client = client_builder().no_proxy().build().unwrap();
        let accept_encoding = client
            .get(&base)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(accept_encoding.contains("gzip"), "{accept_encoding}");
        assert!(accept_encoding.contains("br"), "{accept_encoding}");

        server.abort();
    }
}
