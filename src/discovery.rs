use regex::Regex;
use reqwest::Url;
use scraper::{Html, Selector};

use crate::config::DiscoveryConfig;
use crate::feed;

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("failed to fetch {url}: {source}")]
    Fetch { url: String, source: reqwest::Error },
    #[error("could not discover a feed for {url}")]
    NotFound { url: String },
    #[error("could not resolve a YouTube channel from {url}")]
    YouTubeChannelNotFound { url: String },
}

fn bluesky_feed_url(url: &Url) -> Option<String> {
    if url.host_str() != Some("bsky.app") {
        return None;
    }
    let segments: Vec<&str> = url.path_segments()?.collect();
    let handle = match segments.as_slice() {
        ["profile", handle] if !handle.is_empty() => *handle,
        ["profile", handle, ""] if !handle.is_empty() => *handle,
        _ => return None,
    };
    Some(format!("https://bsky.app/profile/{handle}/rss"))
}

fn is_youtube_host(url: &Url) -> bool {
    matches!(
        url.host_str(),
        Some("youtube.com" | "www.youtube.com" | "m.youtube.com" | "youtu.be")
    )
}

fn extract_channel_id_from_html(html: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let selector = Selector::parse(r#"link[rel~="canonical"]"#).ok()?;
    let href = document.select(&selector).next()?.value().attr("href")?;
    let re = Regex::new(r"/channel/(UC[\w-]+)").ok()?;
    re.captures(href).map(|c| c[1].to_string())
}

fn youtube_uploads_playlist_url(channel_id: &str, exclude_shorts: bool) -> String {
    if exclude_shorts && let Some(rest) = channel_id.strip_prefix("UC") {
        return format!("https://www.youtube.com/feeds/videos.xml?playlist_id=UULF{rest}");
    }
    format!("https://www.youtube.com/feeds/videos.xml?channel_id={channel_id}")
}

fn find_alternate_feed_link(html: &str, base: &Url) -> Option<String> {
    let document = Html::parse_document(html);
    let selector = Selector::parse(r#"link[rel~="alternate"]"#).ok()?;
    for link in document.select(&selector) {
        let value = link.value();
        let is_feed_type = matches!(
            value.attr("type"),
            Some("application/rss+xml" | "application/atom+xml" | "application/feed+json")
        );
        if !is_feed_type {
            continue;
        }
        let Some(href) = value.attr("href") else {
            continue;
        };
        if let Ok(resolved) = base.join(href) {
            return Some(resolved.to_string());
        }
    }
    None
}

fn resolve_from_fetched_page(
    original_url: &str,
    final_url: &Url,
    body: &str,
    config: &DiscoveryConfig,
) -> Result<String, DiscoveryError> {
    if feed::parse_feed(body.as_bytes(), final_url.as_str()).is_ok() {
        return Ok(original_url.to_string());
    }

    if is_youtube_host(final_url) {
        let channel_id = extract_channel_id_from_html(body).ok_or_else(|| {
            DiscoveryError::YouTubeChannelNotFound {
                url: original_url.to_string(),
            }
        })?;
        return Ok(youtube_uploads_playlist_url(
            &channel_id,
            config.youtube_exclude_shorts,
        ));
    }

    find_alternate_feed_link(body, final_url).ok_or_else(|| DiscoveryError::NotFound {
        url: original_url.to_string(),
    })
}

pub async fn resolve_feed_url(
    http: &reqwest::Client,
    config: &DiscoveryConfig,
    url: &str,
) -> Result<String, DiscoveryError> {
    if let Ok(parsed) = Url::parse(url)
        && let Some(feed_url) = bluesky_feed_url(&parsed)
    {
        return Ok(feed_url);
    }

    let response = http
        .get(url)
        .send()
        .await
        .map_err(|source| DiscoveryError::Fetch {
            url: url.to_string(),
            source,
        })?;
    let final_url = response.url().clone();
    let body = response
        .text()
        .await
        .map_err(|source| DiscoveryError::Fetch {
            url: url.to_string(),
            source,
        })?;

    resolve_from_fetched_page(url, &final_url, &body, config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn bluesky_matches_bare_profile() {
        assert_eq!(
            bluesky_feed_url(&url("https://bsky.app/profile/alice.bsky.social")),
            Some("https://bsky.app/profile/alice.bsky.social/rss".to_string())
        );
    }

    #[test]
    fn bluesky_matches_profile_with_trailing_slash() {
        assert_eq!(
            bluesky_feed_url(&url("https://bsky.app/profile/alice.bsky.social/")),
            Some("https://bsky.app/profile/alice.bsky.social/rss".to_string())
        );
    }

    #[test]
    fn bluesky_rejects_extra_path_segments() {
        assert_eq!(
            bluesky_feed_url(&url("https://bsky.app/profile/alice.bsky.social/post/123")),
            None
        );
    }

    #[test]
    fn bluesky_rejects_other_hosts() {
        assert_eq!(
            bluesky_feed_url(&url("https://example.com/profile/alice")),
            None
        );
    }

    #[test]
    fn youtube_host_matching() {
        assert!(is_youtube_host(&url(
            "https://www.youtube.com/channel/UC123"
        )));
        assert!(is_youtube_host(&url("https://youtu.be/abc123")));
        assert!(!is_youtube_host(&url("https://example.com/channel/UC123")));
    }

    #[test]
    fn playlist_url_excludes_shorts_for_uc_prefixed_id() {
        assert_eq!(
            youtube_uploads_playlist_url("UCabc123", true),
            "https://www.youtube.com/feeds/videos.xml?playlist_id=UULFabc123"
        );
    }

    #[test]
    fn playlist_url_includes_shorts_by_default() {
        assert_eq!(
            youtube_uploads_playlist_url("UCabc123", false),
            "https://www.youtube.com/feeds/videos.xml?channel_id=UCabc123"
        );
    }

    #[test]
    fn playlist_url_falls_back_for_non_uc_prefixed_id() {
        assert_eq!(
            youtube_uploads_playlist_url("XYZabc123", true),
            "https://www.youtube.com/feeds/videos.xml?channel_id=XYZabc123"
        );
    }

    #[test]
    fn channel_id_scraped_from_canonical_link() {
        let html = r#"<html><head>
            <link rel="canonical" href="https://www.youtube.com/channel/UCabc123XYZ">
        </head></html>"#;
        assert_eq!(
            extract_channel_id_from_html(html),
            Some("UCabc123XYZ".to_string())
        );
    }

    #[test]
    fn channel_id_missing_when_canonical_points_at_a_video() {
        let html = r#"<html><head>
            <link rel="canonical" href="https://www.youtube.com/watch?v=abc123">
        </head></html>"#;
        assert_eq!(extract_channel_id_from_html(html), None);
    }

    #[test]
    fn alternate_link_resolves_relative_href() {
        let html = r#"<html><head>
            <link rel="alternate" type="application/rss+xml" href="/feed.xml">
        </head></html>"#;
        let base = url("https://example.com/page");
        assert_eq!(
            find_alternate_feed_link(html, &base),
            Some("https://example.com/feed.xml".to_string())
        );
    }

    #[test]
    fn alternate_link_matches_json_feed_type() {
        let html = r#"<html><head>
            <link rel="alternate" type="application/feed+json" href="/feed.json">
        </head></html>"#;
        let base = url("https://example.com/page");
        assert_eq!(
            find_alternate_feed_link(html, &base),
            Some("https://example.com/feed.json".to_string())
        );
    }

    #[test]
    fn alternate_link_ignores_generic_json_type() {
        let html = r#"<html><head>
            <link rel="alternate" type="application/json" href="/data.json">
        </head></html>"#;
        let base = url("https://example.com/page");
        assert_eq!(find_alternate_feed_link(html, &base), None);
    }

    #[test]
    fn alternate_link_ignores_non_feed_rel_types() {
        let html = r#"<html><head>
            <link rel="alternate" hreflang="en" href="/en">
        </head></html>"#;
        let base = url("https://example.com/page");
        assert_eq!(find_alternate_feed_link(html, &base), None);
    }

    #[test]
    fn alternate_link_returns_none_when_absent() {
        let html = "<html><head></head></html>";
        let base = url("https://example.com/page");
        assert_eq!(find_alternate_feed_link(html, &base), None);
    }

    #[test]
    fn alternate_link_first_match_wins() {
        let html = r#"<html><head>
            <link rel="alternate" type="application/rss+xml" href="/first.xml">
            <link rel="alternate" type="application/atom+xml" href="/second.xml">
        </head></html>"#;
        let base = url("https://example.com/page");
        assert_eq!(
            find_alternate_feed_link(html, &base),
            Some("https://example.com/first.xml".to_string())
        );
    }

    const RSS_XML: &str = r#"<rss version="2.0"><channel><title>Feed</title><link>https://example.com</link></channel></rss>"#;

    #[test]
    fn resolve_from_fetched_page_passes_through_when_already_a_feed() {
        let final_url = url("https://example.com/feed.xml");
        let result = resolve_from_fetched_page(
            "https://example.com/feed.xml",
            &final_url,
            RSS_XML,
            &DiscoveryConfig::default(),
        );
        assert_eq!(result.unwrap(), "https://example.com/feed.xml");
    }

    #[test]
    fn resolve_from_fetched_page_resolves_youtube_channel() {
        let html = r#"<html><head>
            <link rel="canonical" href="https://www.youtube.com/channel/UCabc123">
        </head></html>"#;
        let final_url = url("https://www.youtube.com/@somechannel");
        let result = resolve_from_fetched_page(
            "https://www.youtube.com/@somechannel",
            &final_url,
            html,
            &DiscoveryConfig::default(),
        );
        assert_eq!(
            result.unwrap(),
            "https://www.youtube.com/feeds/videos.xml?channel_id=UCabc123"
        );
    }

    #[test]
    fn resolve_from_fetched_page_youtube_without_channel_id_errors() {
        let html = r#"<html><head>
            <link rel="canonical" href="https://www.youtube.com/watch?v=abc123">
        </head></html>"#;
        let final_url = url("https://www.youtube.com/watch?v=abc123");
        let result = resolve_from_fetched_page(
            "https://www.youtube.com/watch?v=abc123",
            &final_url,
            html,
            &DiscoveryConfig::default(),
        );
        assert!(matches!(
            result,
            Err(DiscoveryError::YouTubeChannelNotFound { .. })
        ));
    }

    #[test]
    fn resolve_from_fetched_page_falls_back_to_generic_alternate_link() {
        let html = r#"<html><head>
            <link rel="alternate" type="application/rss+xml" href="/feed.xml">
        </head></html>"#;
        let final_url = url("https://example.com/page");
        let result = resolve_from_fetched_page(
            "https://example.com/page",
            &final_url,
            html,
            &DiscoveryConfig::default(),
        );
        assert_eq!(result.unwrap(), "https://example.com/feed.xml");
    }

    #[test]
    fn resolve_from_fetched_page_not_found_when_nothing_resolves() {
        let final_url = url("https://example.com/page");
        let result = resolve_from_fetched_page(
            "https://example.com/page",
            &final_url,
            "<html><head></head></html>",
            &DiscoveryConfig::default(),
        );
        assert!(matches!(result, Err(DiscoveryError::NotFound { .. })));
    }

    #[tokio::test]
    async fn bluesky_short_circuits_without_a_network_call() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = resolve_feed_url(
            &client,
            &DiscoveryConfig::default(),
            "https://bsky.app/profile/alice.bsky.social",
        )
        .await;
        assert_eq!(
            result.unwrap(),
            "https://bsky.app/profile/alice.bsky.social/rss"
        );
    }

    async fn spawn_test_server() -> (String, tokio::task::JoinHandle<()>) {
        use axum::{Router, routing::get};

        let app = Router::new()
            .route("/feed.xml", get(|| async { RSS_XML }))
            .route(
                "/page",
                get(|| async {
                    axum::response::Html(
                        r#"<html><head>
                        <link rel="alternate" type="application/rss+xml" href="/feed.xml">
                    </head></html>"#,
                    )
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, server)
    }

    #[tokio::test]
    async fn passthrough_when_url_already_a_feed() {
        let (base, server) = spawn_test_server().await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let feed_url = format!("{base}/feed.xml");

        let result = resolve_feed_url(&client, &DiscoveryConfig::default(), &feed_url).await;
        assert_eq!(result.unwrap(), feed_url);

        server.abort();
    }

    #[tokio::test]
    async fn resolves_generic_alternate_link_from_a_page() {
        let (base, server) = spawn_test_server().await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let page_url = format!("{base}/page");

        let result = resolve_feed_url(&client, &DiscoveryConfig::default(), &page_url).await;
        assert_eq!(result.unwrap(), format!("{base}/feed.xml"));

        server.abort();
    }

    #[tokio::test]
    async fn unreachable_url_is_a_fetch_error() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = resolve_feed_url(
            &client,
            &DiscoveryConfig::default(),
            "http://127.0.0.1:1/unreachable",
        )
        .await;
        assert!(matches!(result, Err(DiscoveryError::Fetch { .. })));
    }
}
