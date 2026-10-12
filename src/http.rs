use reqwest::header::{HeaderMap, LOCATION};

const PROJECT_URL: &str = "https://github.com/bevan-philip/tarang";

// Redirects are followed by `get` rather than reqwest, so that callers can
// see the hop and act on permanent moves. Only a single hop is followed.
pub fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent(user_agent(
            option_env!("TARANG_RELEASE_VERSION"),
            env!("CARGO_PKG_VERSION"),
        ))
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    pub status: u16,
    pub location: String,
}

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub url: reqwest::Url,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub redirect: Option<Redirect>,
}

pub async fn get(
    client: &reqwest::Client,
    url: &str,
    headers: HeaderMap,
) -> Result<Response, reqwest::Error> {
    let mut request_url = url.to_string();
    let mut redirect = None;

    loop {
        let response = client
            .get(&request_url)
            .headers(headers.clone())
            .send()
            .await?;
        let status = response.status();
        let url = response.url().clone();

        let next = status
            .is_redirection()
            .then(|| response.headers().get(LOCATION))
            .flatten()
            .and_then(|v| v.to_str().ok())
            .and_then(|loc| url.join(loc).ok());

        match next {
            Some(next) if redirect.is_none() => {
                redirect = Some(Redirect {
                    status: status.as_u16(),
                    location: next.to_string(),
                });
                request_url = next.to_string();
            }
            _ => {
                let headers = response.headers().clone();
                let body = response.bytes().await?.to_vec();
                return Ok(Response {
                    status: status.as_u16(),
                    url,
                    headers,
                    body,
                    redirect,
                });
            }
        }
    }
}

pub fn describe_error(err: &dyn std::error::Error) -> String {
    let mut message = err.to_string();
    let mut source = err.source();
    while let Some(err) = source {
        message.push_str(": ");
        message.push_str(&err.to_string());
        source = err.source();
    }
    message
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

    #[tokio::test]
    async fn get_follows_a_single_redirect() {
        use axum::{
            Router,
            http::{StatusCode, header},
            routing::get as route,
        };

        let app = Router::new()
            .route(
                "/a",
                route(|| async { (StatusCode::MOVED_PERMANENTLY, [(header::LOCATION, "/b")]) }),
            )
            .route(
                "/b",
                route(|| async { (StatusCode::MOVED_PERMANENTLY, [(header::LOCATION, "/c")]) }),
            )
            .route("/c", route(|| async { "feed" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let client = client_builder().no_proxy().build().unwrap();
        let response = get(&client, &format!("{base}/a"), HeaderMap::new())
            .await
            .unwrap();
        assert_eq!(response.status, 301);
        assert_eq!(response.url.as_str(), format!("{base}/b"));
        assert_eq!(
            response.redirect,
            Some(Redirect {
                status: 301,
                location: format!("{base}/b"),
            })
        );

        server.abort();
    }
}
