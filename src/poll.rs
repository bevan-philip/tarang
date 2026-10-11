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

pub enum Outcome {
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

impl Outcome {
    pub fn is_success(&self) -> bool {
        matches!(self, Outcome::Updated { .. } | Outcome::NotModified)
    }
}

pub fn interpret(response: &Result<Response, String>, now: i64) -> Outcome {
    let response = match response {
        Ok(response) => response,
        Err(message) => return Outcome::RequestError(message.clone()),
    };

    match response.status {
        200 => match parse_feed(&response.body, response.url.as_str()) {
            Ok(feed) => Outcome::Updated {
                validators: Validators::from_headers(&response.headers),
                feed,
            },
            Err(e) => Outcome::NotAFeed(e.to_string()),
        },
        304 => Outcome::NotModified,
        status => Outcome::HttpError {
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

// The leading run of 301/308 hops is a permanent move; anything after a
// temporary hop is not.
fn permanent_target(redirects: &[Redirect]) -> Option<String> {
    redirects
        .iter()
        .take_while(|r| matches!(r.status, 301 | 308))
        .last()
        .map(|r| r.location.clone())
}

fn temporary_target(redirects: &[Redirect]) -> Option<String> {
    redirects
        .iter()
        .any(|r| !matches!(r.status, 301 | 308))
        .then(|| redirects.last().map(|r| r.location.clone()))
        .flatten()
}

pub fn next_state(feed: &Feed, outcome: &Outcome, redirects: &[Redirect], now: i64) -> PollUpdate {
    let interval = feed.refresh_interval;
    let base = PollUpdate {
        moved_to: permanent_target(redirects),
        redirected_to: temporary_target(redirects),
        ..Default::default()
    };

    let failure = |message: String, retry_after: Option<i64>, may_disable: bool| {
        let failure_count = feed.failure_count + 1;
        let delay = backoff(interval, failure_count).max(retry_after.unwrap_or(0));
        PollUpdate {
            next_poll_at: now + delay,
            failure_count,
            disabled: may_disable && failure_count >= MAX_FAILURES,
            last_error: Some(message),
            ..base.clone()
        }
    };

    match outcome {
        Outcome::Updated { validators, .. } => PollUpdate {
            last_refresh: Some(now),
            next_poll_at: now + interval,
            validators: Some(validators.clone()),
            ..base
        },
        Outcome::NotModified => PollUpdate {
            last_refresh: Some(now),
            next_poll_at: now + interval,
            ..base
        },
        Outcome::NotAFeed(message) => failure(format!("not a feed: {message}"), None, true),
        Outcome::HttpError {
            status: 410, body, ..
        } => PollUpdate {
            disabled: true,
            ..failure(http_error_message(410, body), None, true)
        },
        Outcome::HttpError {
            status: 429,
            retry_after,
            body,
        } => failure(http_error_message(429, body), *retry_after, false),
        Outcome::HttpError {
            status,
            retry_after,
            body,
        } => failure(http_error_message(*status, body), *retry_after, true),
        Outcome::RequestError(message) => failure(message.clone(), None, true),
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
        Outcome::HttpError {
            status,
            retry_after,
            body: String::new(),
        }
    }

    fn redirect(status: u16, location: &str) -> Redirect {
        Redirect {
            status,
            location: location.to_string(),
        }
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
        let Outcome::Updated { validators, .. } = outcome else {
            panic!("expected Updated");
        };
        assert_eq!(validators.etag.as_deref(), Some("\"v2\""));
        assert_eq!(validators.last_modified.as_deref(), Some("lm2"));
    }

    #[test]
    fn interpret_200_with_one_validator_clears_the_other() {
        let outcome = interpret(&response(200, &[("etag", "\"v2\"")], RSS), NOW);
        let Outcome::Updated { validators, .. } = outcome else {
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
        assert!(matches!(outcome, Outcome::NotAFeed(_)));
    }

    #[test]
    fn interpret_304_is_not_modified() {
        assert!(matches!(
            interpret(&response(304, &[], ""), NOW),
            Outcome::NotModified
        ));
    }

    #[test]
    fn interpret_error_keeps_retry_after_and_body() {
        let outcome = interpret(
            &response(429, &[("retry-after", "120")], "  slow down  "),
            NOW,
        );
        let Outcome::HttpError {
            status,
            retry_after,
            body,
        } = outcome
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
        let outcome = Outcome::Updated {
            validators: validators.clone(),
            feed: parse_feed(RSS.as_bytes(), "https://example.com/feed").unwrap(),
        };
        let update = next_state(&feed(3), &outcome, &[], NOW);
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
        let update = next_state(&feed(0), &Outcome::NotModified, &[], NOW);
        assert_eq!(update.validators, None);
        assert_eq!(update.last_refresh, Some(NOW));
        assert_eq!(update.next_poll_at, NOW + 3600);
    }

    #[test]
    fn failures_back_off_exponentially_and_never_speed_up() {
        let error = Outcome::RequestError("connection refused".to_string());
        let first = next_state(&feed(0), &error, &[], NOW);
        assert_eq!(first.next_poll_at, NOW + 7200);
        assert_eq!(first.failure_count, 1);
        assert_eq!(first.last_error.as_deref(), Some("connection refused"));
        assert_eq!(first.last_refresh, None);
        assert_eq!(first.validators, None);

        let third = next_state(&feed(2), &error, &[], NOW);
        assert_eq!(third.next_poll_at, NOW + 3600 * 8);

        let capped = next_state(&feed(8), &error, &[], NOW);
        assert_eq!(capped.next_poll_at, NOW + MAX_BACKOFF_SECS);
    }

    #[test]
    fn repeated_failures_disable_the_feed() {
        let error = http_error(404, None);
        assert!(!next_state(&feed(MAX_FAILURES - 2), &error, &[], NOW).disabled);
        assert!(next_state(&feed(MAX_FAILURES - 1), &error, &[], NOW).disabled);

        let not_a_feed = Outcome::NotAFeed("bad xml".to_string());
        assert!(next_state(&feed(MAX_FAILURES - 1), &not_a_feed, &[], NOW).disabled);
    }

    #[test]
    fn gone_disables_immediately() {
        let update = next_state(&feed(0), &http_error(410, None), &[], NOW);
        assert!(update.disabled);
        assert_eq!(update.last_error.as_deref(), Some("HTTP 410"));
    }

    #[test]
    fn too_many_requests_honours_retry_after_and_never_disables() {
        let update = next_state(&feed(0), &http_error(429, Some(86400)), &[], NOW);
        assert_eq!(update.next_poll_at, NOW + 86400);
        assert_eq!(update.validators, None);

        let no_hint = next_state(&feed(0), &http_error(429, None), &[], NOW);
        assert_eq!(no_hint.next_poll_at, NOW + 7200);

        assert!(!next_state(&feed(50), &http_error(429, None), &[], NOW).disabled);
    }

    #[test]
    fn short_retry_after_does_not_speed_up_polling() {
        let update = next_state(&feed(0), &http_error(503, Some(5)), &[], NOW);
        assert_eq!(update.next_poll_at, NOW + 7200);
    }

    #[test]
    fn permanent_redirects_move_the_feed() {
        let redirects = [
            redirect(301, "https://a.example/feed"),
            redirect(308, "https://b.example/feed"),
            redirect(302, "https://c.example/feed"),
        ];
        let update = next_state(&feed(0), &Outcome::NotModified, &redirects, NOW);
        assert_eq!(update.moved_to.as_deref(), Some("https://b.example/feed"));
        assert_eq!(
            update.redirected_to.as_deref(),
            Some("https://c.example/feed")
        );
    }

    #[test]
    fn temporary_redirect_first_does_not_move_the_feed() {
        let redirects = [
            redirect(302, "https://a.example/feed"),
            redirect(301, "https://b.example/feed"),
        ];
        let update = next_state(&feed(0), &Outcome::NotModified, &redirects, NOW);
        assert_eq!(update.moved_to, None);
        assert_eq!(
            update.redirected_to.as_deref(),
            Some("https://b.example/feed")
        );
    }

    #[test]
    fn no_redirects_clears_redirected_to() {
        let update = next_state(&feed(0), &Outcome::NotModified, &[], NOW);
        assert_eq!(update.moved_to, None);
        assert_eq!(update.redirected_to, None);
    }
}
