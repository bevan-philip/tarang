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
