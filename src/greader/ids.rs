use super::GReaderError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedRef {
    Pk(i64),
    Url(String),
}

impl FeedRef {
    pub fn parse(value: &str) -> FeedRef {
        match value.parse::<i64>() {
            Ok(pk) => FeedRef::Pk(pk),
            Err(_) => FeedRef::Url(value.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamId {
    ReadingList,
    Read,
    Starred,
    KeptUnread,
    Broadcast,
    BroadcastFriends,
    Like,
    Label(String),
    Feed(FeedRef),
}

impl StreamId {
    pub fn parse(raw: &str) -> Result<StreamId, GReaderError> {
        // "user/<userid>/..." carries a user-id slot that's parsed and
        // discarded (tarang is single-tenant); anything without that
        // prefix (e.g. "feed/1") is dispatched on directly.
        let rest = match raw.strip_prefix("user/") {
            Some(after_user) => after_user.split_once('/').map(|(_, r)| r).unwrap_or(""),
            None => raw,
        };

        if let Some(name) = rest.strip_prefix("label/") {
            return Ok(StreamId::Label(name.to_string()));
        }

        if let Some(state) = rest.strip_prefix("state/com.google/") {
            return match state {
                "reading-list" => Ok(StreamId::ReadingList),
                "read" => Ok(StreamId::Read),
                "starred" => Ok(StreamId::Starred),
                "kept-unread" => Ok(StreamId::KeptUnread),
                "broadcast" => Ok(StreamId::Broadcast),
                "broadcast-friends" => Ok(StreamId::BroadcastFriends),
                "like" => Ok(StreamId::Like),
                other => Err(GReaderError::BadRequest(format!(
                    "unknown state stream: {other}"
                ))),
            };
        }

        if let Some(value) = rest.strip_prefix("feed/") {
            return Ok(StreamId::Feed(FeedRef::parse(value)));
        }

        Err(GReaderError::BadRequest(format!(
            "unrecognized stream id: {raw}"
        )))
    }
}

impl std::fmt::Display for StreamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamId::ReadingList => write!(f, "user/-/state/com.google/reading-list"),
            StreamId::Read => write!(f, "user/-/state/com.google/read"),
            StreamId::Starred => write!(f, "user/-/state/com.google/starred"),
            StreamId::KeptUnread => write!(f, "user/-/state/com.google/kept-unread"),
            StreamId::Broadcast => write!(f, "user/-/state/com.google/broadcast"),
            StreamId::BroadcastFriends => write!(f, "user/-/state/com.google/broadcast-friends"),
            StreamId::Like => write!(f, "user/-/state/com.google/like"),
            StreamId::Label(name) => write!(f, "user/-/label/{name}"),
            StreamId::Feed(FeedRef::Pk(pk)) => write!(f, "feed/{pk}"),
            StreamId::Feed(FeedRef::Url(url)) => write!(f, "feed/{url}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_ref_parse_numeric_is_pk() {
        assert_eq!(FeedRef::parse("42"), FeedRef::Pk(42));
    }

    #[test]
    fn feed_ref_parse_non_numeric_is_url() {
        assert_eq!(
            FeedRef::parse("https://example.com/feed.xml"),
            FeedRef::Url("https://example.com/feed.xml".to_string())
        );
    }

    #[test]
    fn stream_id_parse_state_variants() {
        let cases = [
            (
                "user/-/state/com.google/reading-list",
                StreamId::ReadingList,
            ),
            ("user/-/state/com.google/read", StreamId::Read),
            ("user/-/state/com.google/starred", StreamId::Starred),
            ("user/-/state/com.google/kept-unread", StreamId::KeptUnread),
            ("user/-/state/com.google/broadcast", StreamId::Broadcast),
            (
                "user/-/state/com.google/broadcast-friends",
                StreamId::BroadcastFriends,
            ),
            ("user/-/state/com.google/like", StreamId::Like),
        ];
        for (raw, expected) in cases {
            assert_eq!(StreamId::parse(raw).unwrap(), expected, "{raw}");
        }
    }

    #[test]
    fn stream_id_parse_state_without_user_prefix() {
        assert_eq!(
            StreamId::parse("state/com.google/read").unwrap(),
            StreamId::Read
        );
    }

    #[test]
    fn stream_id_parse_label_with_user_prefix() {
        assert_eq!(
            StreamId::parse("user/-/label/News").unwrap(),
            StreamId::Label("News".to_string())
        );
    }

    #[test]
    fn stream_id_parse_label_without_user_prefix() {
        assert_eq!(
            StreamId::parse("label/News").unwrap(),
            StreamId::Label("News".to_string())
        );
    }

    #[test]
    fn stream_id_parse_feed_pk() {
        assert_eq!(
            StreamId::parse("feed/123").unwrap(),
            StreamId::Feed(FeedRef::Pk(123))
        );
    }

    #[test]
    fn stream_id_parse_feed_url() {
        assert_eq!(
            StreamId::parse("feed/http://example.com/feed.xml").unwrap(),
            StreamId::Feed(FeedRef::Url("http://example.com/feed.xml".to_string()))
        );
    }

    #[test]
    fn stream_id_parse_unknown_state_errors() {
        let result = StreamId::parse("user/-/state/com.google/bogus");
        assert!(result.is_err());
    }

    #[test]
    fn stream_id_parse_unrecognized_raw_errors() {
        let result = StreamId::parse("something/else");
        assert!(result.is_err());
    }

    #[test]
    fn display_round_trips_state_variants() {
        let cases = [
            (
                StreamId::ReadingList,
                "user/-/state/com.google/reading-list",
            ),
            (StreamId::Read, "user/-/state/com.google/read"),
            (StreamId::Starred, "user/-/state/com.google/starred"),
            (StreamId::KeptUnread, "user/-/state/com.google/kept-unread"),
            (StreamId::Broadcast, "user/-/state/com.google/broadcast"),
            (
                StreamId::BroadcastFriends,
                "user/-/state/com.google/broadcast-friends",
            ),
            (StreamId::Like, "user/-/state/com.google/like"),
        ];
        for (stream, expected) in cases {
            assert_eq!(stream.to_string(), expected);
            assert_eq!(StreamId::parse(expected).unwrap(), stream);
        }
    }

    #[test]
    fn display_label_round_trips() {
        let stream = StreamId::Label("News".to_string());
        let text = stream.to_string();
        assert_eq!(text, "user/-/label/News");
        assert_eq!(StreamId::parse(&text).unwrap(), stream);
    }

    #[test]
    fn display_feed_pk_has_no_user_prefix() {
        let stream = StreamId::Feed(FeedRef::Pk(123));
        let text = stream.to_string();
        assert_eq!(text, "feed/123");
        assert_eq!(StreamId::parse(&text).unwrap(), stream);
    }

    #[test]
    fn display_feed_url_has_no_user_prefix() {
        let stream = StreamId::Feed(FeedRef::Url("http://example.com/feed.xml".to_string()));
        let text = stream.to_string();
        assert_eq!(text, "feed/http://example.com/feed.xml");
        assert_eq!(StreamId::parse(&text).unwrap(), stream);
    }
}
