use axum::Json;
use axum::extract::{Path, State};
use std::collections::HashMap;

use super::resolve_feed_pk;
use crate::AppState;
use crate::database::{
    ArticleQuery, ArticleWithState, Db, Feed, count_unread_total, get_category_by_name,
    list_articles_by_pks, list_articles_by_query, list_categories, list_feeds,
    list_unread_counts_by_category, list_unread_counts_by_feed,
};
use crate::greader::GReaderError;
use crate::greader::form::MergedParams;
use crate::greader::ids::{FeedRef, StreamId};
use crate::greader::item_id::{format_item_id_decimal, format_item_id_long, parse_item_id};
use crate::greader::responses::{
    Content, HrefRef, ItemRef, ItemRefsResponse, Origin, StreamContentsResponse, StreamItem,
    UnreadCountEntry, UnreadCountResponse,
};

#[derive(Debug, PartialEq)]
struct StreamParams {
    n: i64,
    c: Option<i64>,
    ot: Option<i64>,
    nt: Option<i64>,
    ascending: bool,
    exclude_read: bool,
}

fn parse_stream_params(params: &MergedParams) -> StreamParams {
    let n = params
        .get("n")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(20)
        .clamp(1, 1000);
    let c = params.get("c").and_then(|v| v.parse().ok());
    let ot = params.get("ot").and_then(|v| v.parse().ok());
    let nt = params.get("nt").and_then(|v| v.parse().ok());
    let ascending = params.get("r") == Some("o");
    let exclude_read = params
        .get_all("xt")
        .iter()
        .any(|v| v == "user/-/state/com.google/read");

    StreamParams {
        n,
        c,
        ot,
        nt,
        ascending,
        exclude_read,
    }
}

fn build_base_query(stream: &StreamId, sp: &StreamParams) -> ArticleQuery {
    let mut q = ArticleQuery {
        published_after: sp.ot,
        published_before: sp.nt,
        cursor: sp.c,
        ascending: sp.ascending,
        limit: sp.n,
        ..Default::default()
    };

    if *stream == StreamId::Starred {
        q.starred_only = true;
    }

    if sp.exclude_read {
        q.unread_only = true;
    }

    q
}

async fn stream_to_query(
    db: &Db,
    stream: &StreamId,
    sp: &StreamParams,
) -> Result<ArticleQuery, GReaderError> {
    let mut q = build_base_query(stream, sp);

    match stream {
        StreamId::Label(name) => {
            let category = get_category_by_name(db, name)
                .await?
                .ok_or_else(|| GReaderError::BadRequest(format!("unknown label: {name}")))?;
            q.category = Some(category.pk);
        }
        StreamId::Feed(feed_ref) => {
            q.feed = Some(resolve_feed_pk(db, feed_ref).await?);
        }
        _ => {}
    }

    Ok(q)
}

fn percent_decode_segment(raw: &str) -> String {
    form_urlencoded::parse(raw.as_bytes())
        .next()
        .map(|(k, _)| k.into_owned())
        .unwrap_or_default()
}

async fn build_lookup_maps(
    db: &Db,
) -> Result<(HashMap<i64, Feed>, HashMap<i64, String>), GReaderError> {
    let feeds = list_feeds(db, crate::database::FeedScope::GReaderVisible).await?;
    let categories = list_categories(db, crate::database::FeedScope::GReaderVisible).await?;
    let category_name_by_pk: HashMap<i64, String> =
        categories.into_iter().map(|c| (c.pk, c.name)).collect();

    let category_names_by_feed: HashMap<i64, String> = feeds
        .iter()
        .filter_map(|f| {
            f.category
                .and_then(|cpk| category_name_by_pk.get(&cpk))
                .map(|name| (f.pk, name.clone()))
        })
        .collect();

    let feeds_by_pk: HashMap<i64, Feed> = feeds.into_iter().map(|f| (f.pk, f)).collect();

    Ok((feeds_by_pk, category_names_by_feed))
}

fn build_contents_response(
    id: String,
    articles: Vec<ArticleWithState>,
    feeds_by_pk: &HashMap<i64, Feed>,
    category_names_by_feed: &HashMap<i64, String>,
    continuation: Option<String>,
) -> StreamContentsResponse {
    let updated = articles
        .iter()
        .filter_map(|a| a.published_at)
        .max()
        .unwrap_or(0);

    let items = articles
        .into_iter()
        .map(|a| {
            let feed = feeds_by_pk.get(&a.feed);
            let mut categories = vec![StreamId::ReadingList.to_string()];
            if a.is_starred {
                categories.push(StreamId::Starred.to_string());
            }
            if let Some(label) = category_names_by_feed.get(&a.feed) {
                categories.push(StreamId::Label(label.clone()).to_string());
            }

            let published = a.published_at.unwrap_or(0);

            StreamItem {
                id: format_item_id_long(a.pk),
                categories,
                title: a.title.unwrap_or_default(),
                published,
                updated: published,
                canonical: vec![HrefRef {
                    href: a.url.clone(),
                }],
                alternate: vec![HrefRef { href: a.url }],
                summary: Content { content: a.content },
                author: String::new(),
                origin: Origin {
                    stream_id: feed
                        .map(|f| StreamId::Feed(FeedRef::Pk(f.pk)).to_string())
                        .unwrap_or_default(),
                    title: feed.map(|f| f.name.clone()).unwrap_or_default(),
                    html_url: feed.map(|f| f.display_url.clone()).unwrap_or_default(),
                },
            }
        })
        .collect();

    StreamContentsResponse {
        id,
        updated,
        items,
        continuation,
    }
}

pub async fn stream_items_ids(
    State(AppState { db, .. }): State<AppState>,
    params: MergedParams,
) -> Result<Json<ItemRefsResponse>, GReaderError> {
    let s = params
        .get("s")
        .unwrap_or("user/-/state/com.google/reading-list");
    let stream = StreamId::parse(s)?;
    let sp = parse_stream_params(&params);
    let query = stream_to_query(&db, &stream, &sp).await?;

    let articles =
        list_articles_by_query(&db, &query, crate::database::FeedScope::GReaderVisible).await?;

    let continuation = if articles.len() as i64 == sp.n {
        articles.last().map(|a| a.pk.to_string())
    } else {
        None
    };

    let item_refs = articles
        .iter()
        .map(|a| ItemRef {
            id: format_item_id_decimal(a.pk),
        })
        .collect();

    Ok(Json(ItemRefsResponse {
        item_refs,
        continuation,
    }))
}

pub async fn stream_items_contents(
    State(AppState { db, .. }): State<AppState>,
    params: MergedParams,
) -> Result<Json<StreamContentsResponse>, GReaderError> {
    let pks: Result<Vec<i64>, GReaderError> = params
        .get_all("i")
        .iter()
        .map(|s| parse_item_id(s))
        .collect();
    let pks = pks?;

    let articles =
        list_articles_by_pks(&db, &pks, crate::database::FeedScope::GReaderVisible).await?;
    let (feeds_by_pk, category_names_by_feed) = build_lookup_maps(&db).await?;

    let response = build_contents_response(
        StreamId::ReadingList.to_string(),
        articles,
        &feeds_by_pk,
        &category_names_by_feed,
        None,
    );

    Ok(Json(response))
}

pub async fn stream_contents(
    State(AppState { db, .. }): State<AppState>,
    Path(raw_stream_id): Path<String>,
    params: MergedParams,
) -> Result<Json<StreamContentsResponse>, GReaderError> {
    let decoded = percent_decode_segment(&raw_stream_id);
    let stream = StreamId::parse(&decoded)?;
    let sp = parse_stream_params(&params);
    let query = stream_to_query(&db, &stream, &sp).await?;

    let articles =
        list_articles_by_query(&db, &query, crate::database::FeedScope::GReaderVisible).await?;
    let (feeds_by_pk, category_names_by_feed) = build_lookup_maps(&db).await?;

    let continuation = if articles.len() as i64 == sp.n {
        articles.last().map(|a| a.pk.to_string())
    } else {
        None
    };

    let response = build_contents_response(
        stream.to_string(),
        articles,
        &feeds_by_pk,
        &category_names_by_feed,
        continuation,
    );

    Ok(Json(response))
}

pub async fn unread_count(
    State(AppState { db, .. }): State<AppState>,
) -> Result<Json<UnreadCountResponse>, GReaderError> {
    let mut counts = Vec::new();

    let total = count_unread_total(&db, crate::database::FeedScope::GReaderVisible).await?;
    counts.push(UnreadCountEntry {
        id: StreamId::ReadingList.to_string(),
        count: total,
    });

    for fc in list_unread_counts_by_feed(&db, crate::database::FeedScope::GReaderVisible).await? {
        counts.push(UnreadCountEntry {
            id: StreamId::Feed(FeedRef::Pk(fc.feed)).to_string(),
            count: fc.count,
        });
    }

    let categories = list_categories(&db, crate::database::FeedScope::GReaderVisible).await?;
    let category_name_by_pk: HashMap<i64, String> =
        categories.into_iter().map(|c| (c.pk, c.name)).collect();

    for cc in
        list_unread_counts_by_category(&db, crate::database::FeedScope::GReaderVisible).await?
    {
        if let Some(name) = category_name_by_pk.get(&cc.category) {
            counts.push(UnreadCountEntry {
                id: StreamId::Label(name.clone()).to_string(),
                count: cc.count,
            });
        }
    }

    Ok(Json(UnreadCountResponse {
        max: 1000,
        unreadcounts: counts,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_stream_params_defaults() {
        let params = MergedParams::from_query("");
        let sp = parse_stream_params(&params);
        assert_eq!(sp.n, 20);
        assert_eq!(sp.c, None);
        assert_eq!(sp.ot, None);
        assert_eq!(sp.nt, None);
        assert!(!sp.ascending);
        assert!(!sp.exclude_read);
    }

    #[test]
    fn parse_stream_params_clamps_n() {
        let sp = parse_stream_params(&MergedParams::from_query("n=0"));
        assert_eq!(sp.n, 1);
        let sp = parse_stream_params(&MergedParams::from_query("n=5000"));
        assert_eq!(sp.n, 1000);
        let sp = parse_stream_params(&MergedParams::from_query("n=not-a-number"));
        assert_eq!(sp.n, 20);
    }

    #[test]
    fn parse_stream_params_r_o_is_ascending() {
        let sp = parse_stream_params(&MergedParams::from_query("r=o"));
        assert!(sp.ascending);
        let sp = parse_stream_params(&MergedParams::from_query("r=n"));
        assert!(!sp.ascending);
    }

    #[test]
    fn parse_stream_params_xt_read_excludes_read() {
        let sp = parse_stream_params(&MergedParams::from_query(
            "xt=user%2F-%2Fstate%2Fcom.google%2Fread",
        ));
        assert!(sp.exclude_read);
        let sp = parse_stream_params(&MergedParams::from_query("xt=something-else"));
        assert!(!sp.exclude_read);
    }

    fn default_stream_params() -> StreamParams {
        StreamParams {
            n: 20,
            c: None,
            ot: None,
            nt: None,
            ascending: false,
            exclude_read: false,
        }
    }

    #[test]
    fn build_base_query_starred_sets_starred_only() {
        let q = build_base_query(&StreamId::Starred, &default_stream_params());
        assert!(q.starred_only);
        assert_eq!(q.feed, None);
        assert_eq!(q.category, None);
    }

    #[test]
    fn build_base_query_reading_list_has_no_scope() {
        let q = build_base_query(&StreamId::ReadingList, &default_stream_params());
        assert!(!q.starred_only);
        assert_eq!(q.feed, None);
        assert_eq!(q.category, None);
    }

    #[test]
    fn build_base_query_passes_through_paging_fields() {
        let sp = StreamParams {
            n: 50,
            c: Some(123),
            ot: Some(1000),
            nt: Some(2000),
            ascending: true,
            exclude_read: true,
        };
        let q = build_base_query(&StreamId::ReadingList, &sp);
        assert_eq!(q.limit, 50);
        assert_eq!(q.cursor, Some(123));
        assert_eq!(q.published_after, Some(1000));
        assert_eq!(q.published_before, Some(2000));
        assert!(q.ascending);
        assert!(q.unread_only);
    }
}
