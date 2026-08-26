use serde::Serialize;

#[derive(Serialize)]
pub struct UserInfoResponse {
    #[serde(rename = "userId")]
    pub user_id: String,
    #[serde(rename = "userName")]
    pub user_name: String,
    #[serde(rename = "userProfileId")]
    pub user_profile_id: String,
    #[serde(rename = "userEmail")]
    pub user_email: String,
}

#[derive(Serialize)]
pub struct Tag {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Serialize)]
pub struct TagListResponse {
    pub tags: Vec<Tag>,
}

#[derive(Serialize)]
pub struct SubscriptionCategory {
    pub id: String,
    pub label: String,
}

#[derive(Serialize)]
pub struct Subscription {
    pub id: String,
    pub title: String,
    pub categories: Vec<SubscriptionCategory>,
    pub url: String,
    #[serde(rename = "htmlUrl")]
    pub html_url: String,
    #[serde(rename = "iconUrl")]
    pub icon_url: String,
}

#[derive(Serialize)]
pub struct SubscriptionListResponse {
    pub subscriptions: Vec<Subscription>,
}

#[derive(Serialize)]
pub struct QuickAddResponse {
    #[serde(rename = "numResults")]
    pub num_results: i32,
    #[serde(rename = "streamId")]
    pub stream_id: String,
    #[serde(rename = "streamName")]
    pub stream_name: String,
}

#[derive(Serialize)]
pub struct UnreadCountEntry {
    pub id: String,
    pub count: i64,
}

#[derive(Serialize)]
pub struct UnreadCountResponse {
    pub max: i32,
    pub unreadcounts: Vec<UnreadCountEntry>,
}

#[derive(Serialize)]
pub struct ItemRef {
    pub id: String,
}

#[derive(Serialize)]
pub struct ItemRefsResponse {
    #[serde(rename = "itemRefs")]
    pub item_refs: Vec<ItemRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation: Option<String>,
}

#[derive(Serialize)]
pub struct HrefRef {
    pub href: String,
}

#[derive(Serialize)]
pub struct Content {
    pub content: String,
}

#[derive(Serialize)]
pub struct Origin {
    #[serde(rename = "streamId")]
    pub stream_id: String,
    pub title: String,
    #[serde(rename = "htmlUrl")]
    pub html_url: String,
}

#[derive(Serialize)]
pub struct StreamItem {
    pub id: String,
    pub categories: Vec<String>,
    pub title: String,
    pub published: i64,
    pub updated: i64,
    pub canonical: Vec<HrefRef>,
    pub alternate: Vec<HrefRef>,
    pub summary: Content,
    pub author: String,
    pub origin: Origin,
}

#[derive(Serialize)]
pub struct StreamContentsResponse {
    pub id: String,
    pub updated: i64,
    pub items: Vec<StreamItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation: Option<String>,
}
