use crate::database::{self, Db, DbResult};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterField {
    Title,
    Content,
    #[default]
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchType {
    Contains,
    Regex,
}

impl FilterField {
    fn from_str(s: &str) -> Self {
        match s {
            "title" => FilterField::Title,
            "content" => FilterField::Content,
            _ => FilterField::Both,
        }
    }
}

impl MatchType {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "contains" => Some(MatchType::Contains),
            "regex" => Some(MatchType::Regex),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum CompiledMatcher {
    Contains(String),
    Regex(regex::Regex),
}

#[derive(Debug)]
pub struct CompiledFilter {
    pub pk: i64,
    pub field: FilterField,
    pub matcher: CompiledMatcher,
    pub feeds: Option<HashSet<i64>>,
}

impl CompiledFilter {
    pub fn applies_to(&self, feed_pk: i64) -> bool {
        self.feeds.as_ref().is_none_or(|f| f.contains(&feed_pk))
    }
}

pub fn validate_pattern(match_type: MatchType, pattern: &str) -> Result<(), regex::Error> {
    if match_type == MatchType::Regex {
        regex::Regex::new(pattern)?;
    }
    Ok(())
}

pub fn compile_filter(
    row: &database::Filter,
    feeds: Option<HashSet<i64>>,
) -> Result<CompiledFilter, regex::Error> {
    let field = FilterField::from_str(&row.field);
    let match_type = MatchType::from_str(&row.match_type).unwrap_or(MatchType::Contains);

    let matcher = match match_type {
        MatchType::Contains => CompiledMatcher::Contains(row.pattern.to_lowercase()),
        MatchType::Regex => CompiledMatcher::Regex(regex::Regex::new(&row.pattern)?),
    };

    Ok(CompiledFilter {
        pk: row.pk,
        field,
        matcher,
        feeds,
    })
}

fn matches_text(matcher: &CompiledMatcher, text: &str) -> bool {
    match matcher {
        CompiledMatcher::Contains(needle) => text.to_lowercase().contains(needle.as_str()),
        CompiledMatcher::Regex(re) => re.is_match(text),
    }
}

pub fn matches(filter: &CompiledFilter, title: &str, content: &str) -> bool {
    match filter.field {
        FilterField::Title => matches_text(&filter.matcher, title),
        FilterField::Content => matches_text(&filter.matcher, content),
        FilterField::Both => {
            matches_text(&filter.matcher, title) || matches_text(&filter.matcher, content)
        }
    }
}

pub async fn load_compiled_filters(db: &Db) -> DbResult<Vec<CompiledFilter>> {
    let rows = database::list_filters(db).await?;
    let pairs = database::list_all_filter_feed_pairs(db).await?;

    let mut feeds_by_filter: HashMap<i64, HashSet<i64>> = HashMap::new();
    for (filter_pk, feed_pk) in pairs {
        feeds_by_filter
            .entry(filter_pk)
            .or_default()
            .insert(feed_pk);
    }

    let filters = rows
        .into_iter()
        .filter_map(|row| {
            let feeds = feeds_by_filter.remove(&row.pk);
            match compile_filter(&row, feeds) {
                Ok(filter) => Some(filter),
                Err(err) => {
                    tracing::warn!(filter = row.pk, error = %err, "skipping filter with invalid pattern");
                    None
                }
            }
        })
        .collect();

    Ok(filters)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contains_filter(field: FilterField, pattern: &str) -> CompiledFilter {
        CompiledFilter {
            pk: 1,
            field,
            matcher: CompiledMatcher::Contains(pattern.to_lowercase()),
            feeds: None,
        }
    }

    fn regex_filter(field: FilterField, pattern: &str) -> CompiledFilter {
        CompiledFilter {
            pk: 1,
            field,
            matcher: CompiledMatcher::Regex(regex::Regex::new(pattern).unwrap()),
            feeds: None,
        }
    }

    #[test]
    fn contains_is_case_insensitive() {
        let f = contains_filter(FilterField::Both, "CRYPTO");
        assert!(matches(&f, "Crypto news today", ""));
        assert!(matches(&f, "", "the crypto market"));
        assert!(!matches(&f, "unrelated title", "unrelated body"));
    }

    #[test]
    fn title_field_ignores_content() {
        let f = contains_filter(FilterField::Title, "spam");
        assert!(matches(&f, "spam alert", "harmless body"));
        assert!(!matches(&f, "harmless title", "spam in the body"));
    }

    #[test]
    fn content_field_ignores_title() {
        let f = contains_filter(FilterField::Content, "spam");
        assert!(!matches(&f, "spam alert", "harmless body"));
        assert!(matches(&f, "harmless title", "spam in the body"));
    }

    #[test]
    fn both_fields_checked_independently_not_concatenated() {
        // If title+content were concatenated into one string, "pre" + "fix"
        // would match "^prefix$" even though neither field alone does.
        let f = regex_filter(FilterField::Both, "^prefix$");
        assert!(!matches(&f, "pre", "fix"));

        let f2 = regex_filter(FilterField::Both, "^full match$");
        assert!(matches(&f2, "full match", "unrelated"));
        assert!(matches(&f2, "unrelated", "full match"));
    }

    #[test]
    fn validate_pattern_accepts_valid_regex() {
        assert!(validate_pattern(MatchType::Regex, r"^foo\d+$").is_ok());
    }

    #[test]
    fn validate_pattern_rejects_invalid_regex() {
        assert!(validate_pattern(MatchType::Regex, "(unclosed").is_err());
    }

    #[test]
    fn validate_pattern_always_accepts_contains() {
        assert!(validate_pattern(MatchType::Contains, "(unclosed").is_ok());
    }

    #[test]
    fn compile_filter_accepts_valid_regex_row() {
        let row = database::Filter {
            pk: 1,
            name: "test".to_string(),
            field: "both".to_string(),
            match_type: "regex".to_string(),
            pattern: r"^foo\d+$".to_string(),
            enabled: true,
            created_at: 0,
        };
        assert!(compile_filter(&row, None).is_ok());
    }

    #[test]
    fn compile_filter_rejects_invalid_regex_row() {
        let row = database::Filter {
            pk: 1,
            name: "test".to_string(),
            field: "both".to_string(),
            match_type: "regex".to_string(),
            pattern: "(unclosed".to_string(),
            enabled: true,
            created_at: 0,
        };
        assert!(compile_filter(&row, None).is_err());
    }

    #[test]
    fn global_filter_applies_to_any_feed() {
        let f = contains_filter(FilterField::Both, "spam");
        assert!(f.applies_to(1));
        assert!(f.applies_to(42));
    }

    #[test]
    fn scoped_filter_applies_only_to_listed_feeds() {
        let mut f = contains_filter(FilterField::Both, "spam");
        f.feeds = Some(HashSet::from([1, 2]));
        assert!(f.applies_to(1));
        assert!(f.applies_to(2));
        assert!(!f.applies_to(3));
    }
}
