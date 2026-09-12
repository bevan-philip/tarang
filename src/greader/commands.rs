use super::GReaderError;
use super::handlers::label_names;
use super::ids::{FeedRef, StreamId};

pub(crate) const READ_TAG: &str = "user/-/state/com.google/read";
pub(crate) const STARRED_TAG: &str = "user/-/state/com.google/starred";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TagEditPlan {
    pub read: Option<bool>,
    pub starred: Option<bool>,
}

pub fn resolve_tag_edit(add: &[String], remove: &[String]) -> Result<TagEditPlan, GReaderError> {
    let add_read = add.iter().any(|s| s == READ_TAG);
    let remove_read = remove.iter().any(|s| s == READ_TAG);
    let add_starred = add.iter().any(|s| s == STARRED_TAG);
    let remove_starred = remove.iter().any(|s| s == STARRED_TAG);

    if add_read && remove_read {
        return Err(GReaderError::BadRequest(
            "conflicting edit: both adds and removes the read tag".into(),
        ));
    }
    if add_starred && remove_starred {
        return Err(GReaderError::BadRequest(
            "conflicting edit: both adds and removes the starred tag".into(),
        ));
    }

    let read = if add_read {
        Some(true)
    } else if remove_read {
        Some(false)
    } else {
        None
    };
    let starred = if add_starred {
        Some(true)
    } else if remove_starred {
        Some(false)
    } else {
        None
    };

    Ok(TagEditPlan { read, starred })
}

/// `ts` is a microsecond-epoch cutoff; absent or explicit 0 both mean "no
/// cutoff" (mark everything), matching how real clients send it.
pub fn parse_mark_all_cutoff(raw: Option<&str>) -> i64 {
    match raw.and_then(|v| v.parse::<i64>().ok()) {
        Some(0) | None => i64::MAX,
        Some(us) => us / 1_000_000,
    }
}

pub(crate) fn dedupe_single_label(labels: &[String]) -> Result<Option<String>, GReaderError> {
    let mut distinct: Vec<&String> = labels.iter().collect();
    distinct.dedup();
    if distinct.len() > 1 {
        return Err(GReaderError::BadRequest(
            "ambiguous: multiple labels in a single edit".into(),
        ));
    }
    Ok(labels.first().cloned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CategoryChangeIntent {
    None,
    Add(String),
    RemoveIfCurrent(String),
}

/// Resolves what a feed's `category` column should become given an edit
/// intent and its current value. Returns `None` for "leave unchanged" and
/// `Some(new_value)` otherwise, mirroring `update_feed`'s own
/// `Option<Option<i64>>` convention for an optional nullable field.
/// `label_lookup` is the caller-resolved pk for the intent's label: the
/// freshly created/fetched pk for `Add`, the result of a name lookup for
/// `RemoveIfCurrent` (or `None` if no such category exists), and unused for
/// `None`.
pub fn resolve_category_change(
    category_change: &CategoryChangeIntent,
    current_category: Option<i64>,
    label_lookup: Option<i64>,
) -> Option<Option<i64>> {
    match category_change {
        CategoryChangeIntent::Add(_) => {
            Some(Some(label_lookup.expect("caller resolves pk for Add")))
        }
        CategoryChangeIntent::RemoveIfCurrent(_) => match label_lookup {
            Some(pk) if current_category == Some(pk) => Some(None),
            _ => None,
        },
        CategoryChangeIntent::None => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscriptionEditCommand {
    Subscribe {
        url: String,
        title: Option<String>,
        category_label: Option<String>,
    },
    Unsubscribe {
        feed_ref: FeedRef,
    },
    Edit {
        feed_ref: FeedRef,
        title: Option<String>,
        category_change: CategoryChangeIntent,
    },
}

pub fn parse_subscription_edit(
    action: &str,
    stream: &str,
    title: Option<&str>,
    add_labels_raw: &[String],
    remove_labels_raw: &[String],
) -> Result<SubscriptionEditCommand, GReaderError> {
    let StreamId::Feed(feed_ref) = StreamId::parse(stream)? else {
        return Err(GReaderError::BadRequest("s must be a feed stream".into()));
    };

    match action {
        "subscribe" => {
            let FeedRef::Url(url) = &feed_ref else {
                return Err(GReaderError::BadRequest(
                    "subscribe requires a feed url".into(),
                ));
            };
            let add_labels = label_names(add_labels_raw);
            let category_label = dedupe_single_label(&add_labels)?;
            Ok(SubscriptionEditCommand::Subscribe {
                url: url.clone(),
                title: title.map(str::to_string),
                category_label,
            })
        }
        "unsubscribe" => Ok(SubscriptionEditCommand::Unsubscribe { feed_ref }),
        "edit" => {
            let add_labels = label_names(add_labels_raw);
            let remove_labels = label_names(remove_labels_raw);

            let add_label = dedupe_single_label(&add_labels)?;
            let remove_label = dedupe_single_label(&remove_labels)?;

            let category_change = if let Some(label) = add_label {
                CategoryChangeIntent::Add(label)
            } else if let Some(label) = remove_label {
                CategoryChangeIntent::RemoveIfCurrent(label)
            } else {
                CategoryChangeIntent::None
            };

            Ok(SubscriptionEditCommand::Edit {
                feed_ref,
                title: title.map(str::to_string),
                category_change,
            })
        }
        other => Err(GReaderError::BadRequest(format!("unknown ac: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_tag_edit_add_read() {
        let plan = resolve_tag_edit(&[READ_TAG.to_string()], &[]).unwrap();
        assert_eq!(plan.read, Some(true));
        assert_eq!(plan.starred, None);
    }

    #[test]
    fn resolve_tag_edit_remove_read() {
        let plan = resolve_tag_edit(&[], &[READ_TAG.to_string()]).unwrap();
        assert_eq!(plan.read, Some(false));
        assert_eq!(plan.starred, None);
    }

    #[test]
    fn resolve_tag_edit_conflicting_read_errors() {
        let result = resolve_tag_edit(&[READ_TAG.to_string()], &[READ_TAG.to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn resolve_tag_edit_add_starred() {
        let plan = resolve_tag_edit(&[STARRED_TAG.to_string()], &[]).unwrap();
        assert_eq!(plan.starred, Some(true));
        assert_eq!(plan.read, None);
    }

    #[test]
    fn resolve_tag_edit_remove_starred() {
        let plan = resolve_tag_edit(&[], &[STARRED_TAG.to_string()]).unwrap();
        assert_eq!(plan.starred, Some(false));
        assert_eq!(plan.read, None);
    }

    #[test]
    fn resolve_tag_edit_conflicting_starred_errors() {
        let result = resolve_tag_edit(&[STARRED_TAG.to_string()], &[STARRED_TAG.to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn resolve_tag_edit_both_independent() {
        let plan = resolve_tag_edit(&[READ_TAG.to_string()], &[STARRED_TAG.to_string()]).unwrap();
        assert_eq!(plan.read, Some(true));
        assert_eq!(plan.starred, Some(false));
    }

    #[test]
    fn resolve_tag_edit_neither() {
        let plan = resolve_tag_edit(&[], &[]).unwrap();
        assert_eq!(plan.read, None);
        assert_eq!(plan.starred, None);
    }

    #[test]
    fn parse_mark_all_cutoff_none_and_zero_are_no_cutoff() {
        assert_eq!(parse_mark_all_cutoff(None), i64::MAX);
        assert_eq!(parse_mark_all_cutoff(Some("0")), i64::MAX);
    }

    #[test]
    fn parse_mark_all_cutoff_converts_microseconds_to_seconds() {
        assert_eq!(parse_mark_all_cutoff(Some("5000000")), 5);
    }

    #[test]
    fn parse_mark_all_cutoff_unparseable_is_no_cutoff() {
        assert_eq!(parse_mark_all_cutoff(Some("not-a-number")), i64::MAX);
    }

    #[test]
    fn parse_mark_all_cutoff_negative_sanity_check() {
        assert_eq!(parse_mark_all_cutoff(Some("-1000000")), -1);
    }

    #[test]
    fn dedupe_single_label_empty() {
        assert_eq!(dedupe_single_label(&[]).unwrap(), None);
    }

    #[test]
    fn dedupe_single_label_one() {
        let labels = vec!["news".to_string()];
        assert_eq!(
            dedupe_single_label(&labels).unwrap(),
            Some("news".to_string())
        );
    }

    #[test]
    fn dedupe_single_label_two_distinct_errors() {
        let labels = vec!["news".to_string(), "tech".to_string()];
        assert!(dedupe_single_label(&labels).is_err());
    }

    #[test]
    fn dedupe_single_label_adjacent_duplicate_is_ok() {
        let labels = vec!["news".to_string(), "news".to_string()];
        assert_eq!(
            dedupe_single_label(&labels).unwrap(),
            Some("news".to_string())
        );
    }

    #[test]
    fn resolve_category_change_add_uses_resolved_pk() {
        let intent = CategoryChangeIntent::Add("News".to_string());
        assert_eq!(
            resolve_category_change(&intent, None, Some(5)),
            Some(Some(5))
        );
    }

    #[test]
    fn resolve_category_change_remove_if_current_matches_clears() {
        let intent = CategoryChangeIntent::RemoveIfCurrent("News".to_string());
        assert_eq!(
            resolve_category_change(&intent, Some(5), Some(5)),
            Some(None)
        );
    }

    #[test]
    fn resolve_category_change_remove_if_current_does_not_match_is_noop() {
        let intent = CategoryChangeIntent::RemoveIfCurrent("News".to_string());
        assert_eq!(resolve_category_change(&intent, Some(7), Some(5)), None);
    }

    #[test]
    fn resolve_category_change_remove_if_current_label_missing_is_noop() {
        let intent = CategoryChangeIntent::RemoveIfCurrent("News".to_string());
        assert_eq!(resolve_category_change(&intent, Some(5), None), None);
    }

    #[test]
    fn resolve_category_change_none_is_noop() {
        assert_eq!(
            resolve_category_change(&CategoryChangeIntent::None, Some(5), None),
            None
        );
    }

    #[test]
    fn parse_subscription_edit_subscribe_with_url() {
        let cmd = parse_subscription_edit(
            "subscribe",
            "feed/https://example.com/feed.xml",
            Some("My Feed"),
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(
            cmd,
            SubscriptionEditCommand::Subscribe {
                url: "https://example.com/feed.xml".to_string(),
                title: Some("My Feed".to_string()),
                category_label: None,
            }
        );
    }

    #[test]
    fn parse_subscription_edit_subscribe_with_pk_stream_errors() {
        let result = parse_subscription_edit("subscribe", "feed/42", None, &[], &[]);
        assert!(result.is_err());
    }

    #[test]
    fn parse_subscription_edit_subscribe_with_one_label() {
        let cmd = parse_subscription_edit(
            "subscribe",
            "feed/https://example.com/feed.xml",
            None,
            &["user/-/label/News".to_string()],
            &[],
        )
        .unwrap();
        assert_eq!(
            cmd,
            SubscriptionEditCommand::Subscribe {
                url: "https://example.com/feed.xml".to_string(),
                title: None,
                category_label: Some("News".to_string()),
            }
        );
    }

    #[test]
    fn parse_subscription_edit_subscribe_with_two_labels_errors() {
        let result = parse_subscription_edit(
            "subscribe",
            "feed/https://example.com/feed.xml",
            None,
            &[
                "user/-/label/News".to_string(),
                "user/-/label/Tech".to_string(),
            ],
            &[],
        );
        assert!(result.is_err());
    }

    #[test]
    fn parse_subscription_edit_unsubscribe_preserves_feed_ref() {
        let cmd = parse_subscription_edit("unsubscribe", "feed/42", None, &[], &[]).unwrap();
        assert_eq!(
            cmd,
            SubscriptionEditCommand::Unsubscribe {
                feed_ref: FeedRef::Pk(42)
            }
        );
    }

    #[test]
    fn parse_subscription_edit_edit_add_only() {
        let cmd = parse_subscription_edit(
            "edit",
            "feed/42",
            None,
            &["user/-/label/News".to_string()],
            &[],
        )
        .unwrap();
        assert_eq!(
            cmd,
            SubscriptionEditCommand::Edit {
                feed_ref: FeedRef::Pk(42),
                title: None,
                category_change: CategoryChangeIntent::Add("News".to_string()),
            }
        );
    }

    #[test]
    fn parse_subscription_edit_edit_remove_only() {
        let cmd = parse_subscription_edit(
            "edit",
            "feed/42",
            None,
            &[],
            &["user/-/label/News".to_string()],
        )
        .unwrap();
        assert_eq!(
            cmd,
            SubscriptionEditCommand::Edit {
                feed_ref: FeedRef::Pk(42),
                title: None,
                category_change: CategoryChangeIntent::RemoveIfCurrent("News".to_string()),
            }
        );
    }

    #[test]
    fn parse_subscription_edit_edit_neither() {
        let cmd = parse_subscription_edit("edit", "feed/42", None, &[], &[]).unwrap();
        assert_eq!(
            cmd,
            SubscriptionEditCommand::Edit {
                feed_ref: FeedRef::Pk(42),
                title: None,
                category_change: CategoryChangeIntent::None,
            }
        );
    }

    #[test]
    fn parse_subscription_edit_edit_both_add_wins() {
        let cmd = parse_subscription_edit(
            "edit",
            "feed/42",
            None,
            &["user/-/label/News".to_string()],
            &["user/-/label/Tech".to_string()],
        )
        .unwrap();
        assert_eq!(
            cmd,
            SubscriptionEditCommand::Edit {
                feed_ref: FeedRef::Pk(42),
                title: None,
                category_change: CategoryChangeIntent::Add("News".to_string()),
            }
        );
    }

    #[test]
    fn parse_subscription_edit_edit_title_passthrough() {
        let cmd = parse_subscription_edit("edit", "feed/42", Some("New title"), &[], &[]).unwrap();
        assert_eq!(
            cmd,
            SubscriptionEditCommand::Edit {
                feed_ref: FeedRef::Pk(42),
                title: Some("New title".to_string()),
                category_change: CategoryChangeIntent::None,
            }
        );
    }

    #[test]
    fn parse_subscription_edit_unknown_action_errors() {
        let result = parse_subscription_edit("frobnicate", "feed/42", None, &[], &[]);
        assert!(result.is_err());
    }

    #[test]
    fn parse_subscription_edit_non_feed_stream_errors() {
        let result = parse_subscription_edit(
            "edit",
            "user/-/state/com.google/reading-list",
            None,
            &[],
            &[],
        );
        assert!(result.is_err());
    }
}
