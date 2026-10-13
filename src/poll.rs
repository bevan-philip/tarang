use reqwest::header::{
    ETAG, HeaderMap, HeaderValue, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED, RETRY_AFTER,
};

use crate::database::Feed;
use crate::feed::{ParsedFeedResult, parse_feed};
use crate::http::{Redirect, Response};

const MAX_FAILURES: i64 = 10;
const MAX_BACKOFF_SECS: i64 = 7 * 24 * 60 * 60;
const ERROR_BODY_CHARS: usize = 500;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Validators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl Validators {
    pub fn of(feed: &Feed) -> Self {
        Validators {
            etag: feed.etag.clone(),
            last_modified: feed.last_modified.clone(),
        }
    }

    fn from_headers(headers: &HeaderMap) -> Self {
        let get = |name| {
            headers
                .get(name)
                .and_then(|v: &HeaderValue| v.to_str().ok())
                .map(str::to_string)
        };
        Validators {
            etag: get(ETAG),
            last_modified: get(LAST_MODIFIED),
        }
    }

    pub fn request_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(value) = self.etag.as_deref().and_then(|v| v.parse().ok()) {
            headers.insert(IF_NONE_MATCH, value);
        }
        if let Some(value) = self.last_modified.as_deref().and_then(|v| v.parse().ok()) {
            headers.insert(IF_MODIFIED_SINCE, value);
        }
        headers
    }
}

pub enum Fetch {
    Updated {
        validators: Validators,
        feed: ParsedFeedResult,
    },
    NotModified,
    NotAFeed(String),
    HttpError {
        status: u16,
        retry_after: Option<i64>,
        body: String,
    },
    RequestError(String),
}

pub struct Outcome {
    pub fetch: Fetch,
    pub moved_to: Option<String>,
    pub redirected_to: Option<String>,
}

impl From<Fetch> for Outcome {
    fn from(fetch: Fetch) -> Self {
        Outcome {
            fetch,
            moved_to: None,
            redirected_to: None,
        }
    }
}

pub fn interpret(response: &Result<Response, String>, now: i64) -> Outcome {
    let response = match response {
        Ok(response) => response,
        Err(message) => return Fetch::RequestError(message.clone()).into(),
    };
    Outcome {
        fetch: fetch(response, now),
        moved_to: permanent_target(&response.redirects),
        redirected_to: temporary_target(&response.redirects),
    }
}

// The leading run of 301/308 hops is a permanent move; anything after a
// temporary hop is not.
fn permanent_target(redirects: &[Redirect]) -> Option<String> {
    redirects
        .iter()
        .take_while(|r| is_permanent(r))
        .last()
        .map(|r| r.location.clone())
}

fn temporary_target(redirects: &[Redirect]) -> Option<String> {
    redirects
        .iter()
        .any(|r| !is_permanent(r))
        .then(|| redirects.last().map(|r| r.location.clone()))
        .flatten()
}

fn fetch(response: &Response, now: i64) -> Fetch {
    match response.status {
        200 => match parse_feed(&response.body, response.url.as_str()) {
            Ok(feed) => Fetch::Updated {
                validators: Validators::from_headers(&response.headers),
                feed,
            },
            Err(e) => Fetch::NotAFeed(e.to_string()),
        },
        304 => Fetch::NotModified,
        status => Fetch::HttpError {
            status,
            retry_after: response
                .headers
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| parse_retry_after(v, now)),
            body: String::from_utf8_lossy(&response.body)
                .trim()
                .chars()
                .take(ERROR_BODY_CHARS)
                .collect(),
        },
    }
}

pub fn parse_retry_after(value: &str, now: i64) -> Option<i64> {
    let value = value.trim();
    if let Ok(secs) = value.parse::<i64>() {
        return Some(secs.max(0));
    }
    let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    Some((date.timestamp() - now).max(0))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PollUpdate {
    pub last_refresh: Option<i64>,
    pub next_poll_at: i64,
    pub failure_count: i64,
    pub disabled: bool,
    pub last_error: Option<String>,
    pub validators: Option<Validators>,
    pub moved_to: Option<String>,
    pub redirected_to: Option<String>,
}

fn backoff(refresh_interval: i64, failure_count: i64) -> i64 {
    let factor = 1i64 << failure_count.clamp(0, 20);
    refresh_interval
        .saturating_mul(factor)
        .min(MAX_BACKOFF_SECS)
        .max(refresh_interval)
}

fn is_permanent(redirect: &Redirect) -> bool {
    matches!(redirect.status, 301 | 308)
}

// `moved_to_taken` is whether another feed already has the URL this feed
// moved to; the feed is then disabled rather than duplicated. A move is only
// followed once the new URL has served the feed.
pub fn next_state(feed: &Feed, outcome: &Outcome, moved_to_taken: bool, now: i64) -> PollUpdate {
    let interval = feed.refresh_interval;
    let moved_to = outcome.moved_to.clone().filter(|url| *url != feed.url);
    let base = PollUpdate {
        redirected_to: outcome.redirected_to.clone(),
        ..Default::default()
    };

    let success = |validators: Option<Validators>| {
        let update = PollUpdate {
            last_refresh: Some(now),
            next_poll_at: now + interval,
            validators,
            ..base.clone()
        };
        match moved_to.clone() {
            Some(moved_to) if moved_to_taken => PollUpdate {
                disabled: true,
                last_error: Some(format!(
                    "moved permanently to {moved_to}, which another feed already uses"
                )),
                ..update
            },
            moved_to => PollUpdate { moved_to, ..update },
        }
    };

    let failure = |message: String, retry_after: Option<i64>, may_disable: bool| {
        let failure_count = feed.failure_count + 1;
        let retry_after = retry_after.unwrap_or(0).min(MAX_BACKOFF_SECS);
        let delay = backoff(interval, failure_count).max(retry_after);
        PollUpdate {
            next_poll_at: now + delay,
            failure_count,
            disabled: may_disable && failure_count >= MAX_FAILURES,
            last_error: Some(message),
            ..base.clone()
        }
    };

    match &outcome.fetch {
        Fetch::Updated { validators, .. } => success(Some(validators.clone())),
        Fetch::NotModified => success(None),
        Fetch::NotAFeed(message) => failure(format!("not a feed: {message}"), None, true),
        Fetch::HttpError {
            status: 410, body, ..
        } => PollUpdate {
            disabled: true,
            ..failure(http_error_message(410, body), None, true)
        },
        Fetch::HttpError {
            status: 429,
            retry_after,
            body,
        } => failure(http_error_message(429, body), *retry_after, false),
        Fetch::HttpError {
            status,
            retry_after,
            body,
        } => failure(http_error_message(*status, body), *retry_after, true),
        Fetch::RequestError(message) => failure(message.clone(), None, true),
    }
}

fn http_error_message(status: u16, body: &str) -> String {
    if body.is_empty() {
        format!("HTTP {status}")
    } else {
        format!("HTTP {status}: {body}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000;

    fn feed(failure_count: i64) -> Feed {
        Feed {
            pk: 1,
            name: "Feed".to_string(),
            url: "https://example.com/feed".to_string(),
            display_url: String::new(),
            category: None,
            metadata: "{}".to_string(),
            refresh_interval: 3600,
            last_refresh: None,
            next_poll_at: None,
            greader_hidden: false,
            etag: None,
            last_modified: None,
            failure_count,
            disabled: false,
            last_error: None,
            moved_from: None,
            redirected_to: None,
        }
    }

    fn response(status: u16, headers: &[(&str, &str)], body: &str) -> Result<Response, String> {
        let mut map = HeaderMap::new();
        for (name, value) in headers {
            map.insert(
                reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        Ok(Response {
            status,
            url: reqwest::Url::parse("https://example.com/feed").unwrap(),
            headers: map,
            body: body.as_bytes().to_vec(),
            redirects: vec![],
        })
    }

    const RSS: &str = r#"<rss version="2.0"><channel><title>T</title><link>https://example.com</link></channel></rss>"#;

    fn http_error(status: u16, retry_after: Option<i64>) -> Outcome {
        Fetch::HttpError {
            status,
            retry_after,
            body: String::new(),
        }
        .into()
    }

    fn redirected(hops: &[(u16, &str)]) -> Outcome {
        let mut response = response(304, &[], "");
        response.as_mut().unwrap().redirects = hops
            .iter()
            .map(|(status, location)| Redirect {
                status: *status,
                location: location.to_string(),
            })
            .collect();
        interpret(&response, NOW)
    }

    #[test]
    fn request_headers_echo_validators_unaltered() {
        let validators = Validators {
            etag: Some("W/\"abc\"".to_string()),
            last_modified: Some("Mon, 01 Jan 2026 00:00:00 GMT".to_string()),
        };
        let headers = validators.request_headers();
        assert_eq!(headers[IF_NONE_MATCH], "W/\"abc\"");
        assert_eq!(headers[IF_MODIFIED_SINCE], "Mon, 01 Jan 2026 00:00:00 GMT");
    }

    #[test]
    fn first_request_is_unconditional() {
        assert!(Validators::default().request_headers().is_empty());
    }

    #[test]
    fn interpret_200_feed_keeps_both_validators_from_that_response() {
        let outcome = interpret(
            &response(200, &[("etag", "\"v2\""), ("last-modified", "lm2")], RSS),
            NOW,
        );
        let Fetch::Updated { validators, .. } = outcome.fetch else {
            panic!("expected Updated");
        };
        assert_eq!(validators.etag.as_deref(), Some("\"v2\""));
        assert_eq!(validators.last_modified.as_deref(), Some("lm2"));
    }

    #[test]
    fn interpret_200_with_one_validator_clears_the_other() {
        let outcome = interpret(&response(200, &[("etag", "\"v2\"")], RSS), NOW);
        let Fetch::Updated { validators, .. } = outcome.fetch else {
            panic!("expected Updated");
        };
        assert_eq!(
            validators,
            Validators {
                etag: Some("\"v2\"".to_string()),
                last_modified: None
            }
        );
    }

    #[test]
    fn interpret_200_non_feed_is_not_a_feed() {
        let outcome = interpret(&response(200, &[], "<html></html>"), NOW);
        assert!(matches!(outcome.fetch, Fetch::NotAFeed(_)));
    }

    #[test]
    fn interpret_304_is_not_modified() {
        assert!(matches!(
            interpret(&response(304, &[], ""), NOW).fetch,
            Fetch::NotModified
        ));
    }

    #[test]
    fn interpret_error_keeps_retry_after_and_body() {
        let outcome = interpret(
            &response(429, &[("retry-after", "120")], "  slow down  "),
            NOW,
        );
        let Fetch::HttpError {
            status,
            retry_after,
            body,
        } = outcome.fetch
        else {
            panic!("expected HttpError");
        };
        assert_eq!(status, 429);
        assert_eq!(retry_after, Some(120));
        assert_eq!(body, "slow down");
    }

    #[test]
    fn retry_after_accepts_seconds_and_http_dates() {
        assert_eq!(parse_retry_after("3600", NOW), Some(3600));
        let date = chrono::DateTime::from_timestamp(NOW + 90, 0)
            .unwrap()
            .format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string();
        assert_eq!(parse_retry_after(&date, NOW), Some(90));
        assert_eq!(parse_retry_after("soon", NOW), None);
    }

    #[test]
    fn success_resets_failures_and_stores_validators() {
        let validators = Validators {
            etag: Some("\"v\"".to_string()),
            last_modified: None,
        };
        let outcome = Fetch::Updated {
            validators: validators.clone(),
            feed: parse_feed(RSS.as_bytes(), "https://example.com/feed").unwrap(),
        }
        .into();
        let update = next_state(&feed(3), &outcome, false, NOW);
        assert_eq!(
            update,
            PollUpdate {
                last_refresh: Some(NOW),
                next_poll_at: NOW + 3600,
                validators: Some(validators),
                ..Default::default()
            }
        );
    }

    #[test]
    fn not_modified_keeps_existing_validators() {
        let update = next_state(&feed(0), &Fetch::NotModified.into(), false, NOW);
        assert_eq!(update.validators, None);
        assert_eq!(update.last_refresh, Some(NOW));
        assert_eq!(update.next_poll_at, NOW + 3600);
    }

    #[test]
    fn failures_back_off_exponentially_and_never_speed_up() {
        let error = Fetch::RequestError("connection refused".to_string()).into();
        let first = next_state(&feed(0), &error, false, NOW);
        assert_eq!(first.next_poll_at, NOW + 7200);
        assert_eq!(first.failure_count, 1);
        assert_eq!(first.last_error.as_deref(), Some("connection refused"));
        assert_eq!(first.last_refresh, None);
        assert_eq!(first.validators, None);

        let third = next_state(&feed(2), &error, false, NOW);
        assert_eq!(third.next_poll_at, NOW + 3600 * 8);

        let capped = next_state(&feed(8), &error, false, NOW);
        assert_eq!(capped.next_poll_at, NOW + MAX_BACKOFF_SECS);
    }

    #[test]
    fn repeated_failures_disable_the_feed() {
        let error = http_error(404, None);
        assert!(!next_state(&feed(MAX_FAILURES - 2), &error, false, NOW).disabled);
        assert!(next_state(&feed(MAX_FAILURES - 1), &error, false, NOW).disabled);

        let not_a_feed = Fetch::NotAFeed("bad xml".to_string()).into();
        assert!(next_state(&feed(MAX_FAILURES - 1), &not_a_feed, false, NOW).disabled);
    }

    #[test]
    fn gone_disables_immediately() {
        let update = next_state(&feed(0), &http_error(410, None), false, NOW);
        assert!(update.disabled);
        assert_eq!(update.last_error.as_deref(), Some("HTTP 410"));
    }

    #[test]
    fn too_many_requests_honours_retry_after_and_never_disables() {
        let update = next_state(&feed(0), &http_error(429, Some(86400)), false, NOW);
        assert_eq!(update.next_poll_at, NOW + 86400);
        assert_eq!(update.validators, None);

        let no_hint = next_state(&feed(0), &http_error(429, None), false, NOW);
        assert_eq!(no_hint.next_poll_at, NOW + 7200);

        assert!(!next_state(&feed(50), &http_error(429, None), false, NOW).disabled);
    }

    #[test]
    fn short_retry_after_does_not_speed_up_polling() {
        let update = next_state(&feed(0), &http_error(503, Some(5)), false, NOW);
        assert_eq!(update.next_poll_at, NOW + 7200);
    }

    #[test]
    fn permanent_redirect_moves_the_feed() {
        for status in [301, 308] {
            let outcome = redirected(&[(status, "https://a.example/feed")]);
            assert_eq!(outcome.moved_to.as_deref(), Some("https://a.example/feed"));
            assert_eq!(outcome.redirected_to, None);

            let update = next_state(&feed(0), &outcome, false, NOW);
            assert_eq!(update.moved_to.as_deref(), Some("https://a.example/feed"));
            assert_eq!(update.redirected_to, None);
            assert!(!update.disabled);
        }
    }

    #[test]
    fn temporary_redirect_does_not_move_the_feed() {
        for status in [302, 303, 307] {
            let outcome = redirected(&[(status, "https://a.example/feed")]);
            assert_eq!(outcome.moved_to, None);

            let update = next_state(&feed(0), &outcome, false, NOW);
            assert_eq!(update.moved_to, None);
            assert_eq!(
                update.redirected_to.as_deref(),
                Some("https://a.example/feed")
            );
        }
    }

    #[test]
    fn leading_permanent_redirects_move_the_feed() {
        let outcome = redirected(&[
            (301, "https://a.example/feed"),
            (308, "https://b.example/feed"),
            (302, "https://c.example/feed"),
        ]);
        assert_eq!(outcome.moved_to.as_deref(), Some("https://b.example/feed"));
        assert_eq!(
            outcome.redirected_to.as_deref(),
            Some("https://c.example/feed")
        );
    }

    #[test]
    fn temporary_redirect_first_does_not_move_the_feed() {
        let outcome = redirected(&[
            (302, "https://a.example/feed"),
            (301, "https://b.example/feed"),
        ]);
        assert_eq!(outcome.moved_to, None);
        assert_eq!(
            outcome.redirected_to.as_deref(),
            Some("https://b.example/feed")
        );
    }

    #[test]
    fn no_redirect_clears_redirected_to() {
        let update = next_state(&feed(0), &Fetch::NotModified.into(), false, NOW);
        assert_eq!(update.moved_to, None);
        assert_eq!(update.redirected_to, None);
    }

    #[test]
    fn redirect_to_its_own_url_is_not_a_move() {
        let outcome = redirected(&[(301, "https://example.com/feed")]);
        let update = next_state(&feed(0), &outcome, true, NOW);
        assert_eq!(update.moved_to, None);
        assert!(!update.disabled);
    }

    #[test]
    fn failed_fetch_after_a_permanent_redirect_does_not_move() {
        let mut failed = http_error(404, None);
        failed.moved_to = Some("https://a.example/feed".to_string());
        let update = next_state(&feed(0), &failed, false, NOW);
        assert_eq!(update.moved_to, None);
        assert_eq!(update.last_error.as_deref(), Some("HTTP 404"));

        let update = next_state(&feed(0), &failed, true, NOW);
        assert_eq!(update.moved_to, None);
        assert!(!update.disabled);
    }

    #[test]
    fn huge_retry_after_is_capped() {
        let update = next_state(&feed(0), &http_error(429, Some(i64::MAX)), false, NOW);
        assert_eq!(update.next_poll_at, NOW + MAX_BACKOFF_SECS);
    }

    #[test]
    fn moving_onto_another_feeds_url_disables_this_feed() {
        let outcome = redirected(&[(301, "https://a.example/feed")]);
        let update = next_state(&feed(0), &outcome, true, NOW);
        assert!(update.disabled);
        assert_eq!(update.moved_to, None);
        assert_eq!(
            update.last_error.as_deref(),
            Some("moved permanently to https://a.example/feed, which another feed already uses")
        );
    }
}
