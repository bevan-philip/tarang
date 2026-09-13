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
    /// Mirrors `compile_filter`'s leniency: an unrecognized match_type
    /// string falls back to `Contains` (so it's always safe to validate),
    /// and is rejected by the `filter` table's CHECK constraint at insert
    /// time.
    pub fn parse(s: &str) -> Self {
        match s {
            "regex" => MatchType::Regex,
            _ => MatchType::Contains,
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

// Callers today only need the validation side effect and discard the
// returned rule, but the fields are part of the public contract (and are
// exercised directly by this module's tests).
#[allow(dead_code)]
pub struct EffectiveRule {
    pub match_type: MatchType,
    pub pattern: String,
}

/// Merges optional PATCH fields for match_type/pattern onto an existing
/// filter row and validates the resulting effective rule. Returns `Ok(None)`
/// when neither field is being changed. Fixes a real gap: previously,
/// patching match_type alone (without pattern) skipped validation entirely,
/// even though update_filter would recompile the *existing* pattern under
/// the *new* match_type - meaning a PATCH that only flips match_type to
/// "regex" against an existing non-regex pattern could panic in production
/// via compile_filter's `.expect()`. This function guarantees that case is
/// validated before any write, surfacing an ordinary 400 instead.
pub fn validate_effective_rule(
    existing: Option<&database::Filter>,
    match_type: Option<&str>,
    pattern: Option<&str>,
) -> Result<Option<EffectiveRule>, regex::Error> {
    if match_type.is_none() && pattern.is_none() {
        return Ok(None);
    }

    let effective_match_type = match match_type {
        Some(mt) => MatchType::parse(mt),
        None => MatchType::parse(
            &existing
                .expect("existing filter required when only pattern changes")
                .match_type,
        ),
    };
    let effective_pattern = match pattern {
        Some(p) => p.to_string(),
        None => existing
            .expect("existing filter required when only match_type changes")
            .pattern
            .clone(),
    };

    validate_pattern(effective_match_type, &effective_pattern)?;

    Ok(Some(EffectiveRule {
        match_type: effective_match_type,
        pattern: effective_pattern,
    }))
}

pub fn compile_filter(
    row: &database::Filter,
    feeds: Option<HashSet<i64>>,
) -> Result<CompiledFilter, regex::Error> {
    let field = FilterField::from_str(&row.field);
    let match_type = MatchType::parse(&row.match_type);

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

pub fn compute_filter_matches(
    articles: &[database::Article],
    filters: &[CompiledFilter],
) -> Vec<(i64, i64)> {
    articles
        .iter()
        .flat_map(|article| {
            let title = article.title.as_deref().unwrap_or("");
            let content = article.content.as_str();
            filters
                .iter()
                .filter(move |f| f.applies_to(article.feed) && matches(f, title, content))
                .map(|f| (article.pk, f.pk))
        })
        .collect()
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

    #[test]
    fn match_type_parse_recognizes_known_values_and_falls_back_to_contains() {
        assert_eq!(MatchType::parse("regex"), MatchType::Regex);
        assert_eq!(MatchType::parse("contains"), MatchType::Contains);
        assert_eq!(MatchType::parse("nonsense"), MatchType::Contains);
    }

    fn filter_row(match_type: &str, pattern: &str) -> database::Filter {
        database::Filter {
            pk: 1,
            name: "test".to_string(),
            field: "both".to_string(),
            match_type: match_type.to_string(),
            pattern: pattern.to_string(),
            enabled: true,
            created_at: 0,
        }
    }

    #[test]
    fn validate_effective_rule_for_create_valid() {
        let result = validate_effective_rule(None, Some("regex"), Some(r"^foo\d+$")).unwrap();
        let rule = result.unwrap();
        assert_eq!(rule.match_type, MatchType::Regex);
        assert_eq!(rule.pattern, r"^foo\d+$");
    }

    #[test]
    fn validate_effective_rule_for_create_invalid() {
        let result = validate_effective_rule(None, Some("regex"), Some("(unclosed"));
        assert!(result.is_err());
    }

    #[test]
    fn validate_effective_rule_patch_neither_is_noop() {
        let result = validate_effective_rule(None, None, None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn validate_effective_rule_patch_pattern_only_uses_existing_match_type() {
        let existing = filter_row("contains", "old-pattern");
        let result = validate_effective_rule(Some(&existing), None, Some("(unclosed")).unwrap();
        // Existing match_type is "contains", so an unbalanced paren is fine.
        let rule = result.unwrap();
        assert_eq!(rule.match_type, MatchType::Contains);
        assert_eq!(rule.pattern, "(unclosed");
    }

    #[test]
    fn validate_effective_rule_patch_match_type_only_revalidates_existing_pattern() {
        // Regression case: an existing "contains" filter has a pattern that
        // is not valid regex. Patching match_type alone to "regex" must
        // revalidate that existing pattern and fail cleanly.
        let existing = filter_row("contains", "(unclosed");
        let result = validate_effective_rule(Some(&existing), Some("regex"), None);
        assert!(result.is_err());
    }

    #[test]
    fn validate_effective_rule_patch_both() {
        let existing = filter_row("contains", "old-pattern");
        let result =
            validate_effective_rule(Some(&existing), Some("regex"), Some(r"^new\d+$")).unwrap();
        let rule = result.unwrap();
        assert_eq!(rule.match_type, MatchType::Regex);
        assert_eq!(rule.pattern, r"^new\d+$");
    }

    fn fake_article(pk: i64, feed: i64, title: &str, content: &str) -> database::Article {
        database::Article {
            pk,
            feed,
            url: format!("https://example.com/{pk}"),
            guid: format!("guid-{pk}"),
            title: Some(title.to_string()),
            content: content.to_string(),
            summary: None,
            published_at: Some(1000),
            retrieved_at: 2000,
        }
    }

    #[test]
    fn compute_filter_matches_global_filter_across_feeds() {
        let articles = vec![
            fake_article(1, 10, "crypto news", ""),
            fake_article(2, 20, "crypto update", ""),
        ];
        let filters = vec![contains_filter(FilterField::Both, "crypto")];
        let mut pairs = compute_filter_matches(&articles, &filters);
        pairs.sort();
        assert_eq!(pairs, vec![(1, 1), (2, 1)]);
    }

    #[test]
    fn compute_filter_matches_scoped_filter() {
        let articles = vec![
            fake_article(1, 10, "crypto news", ""),
            fake_article(2, 20, "crypto update", ""),
        ];
        let mut filter = contains_filter(FilterField::Both, "crypto");
        filter.feeds = Some(HashSet::from([10]));
        let pairs = compute_filter_matches(&articles, std::slice::from_ref(&filter));
        assert_eq!(pairs, vec![(1, 1)]);
    }

    #[test]
    fn compute_filter_matches_no_match() {
        let articles = vec![fake_article(1, 10, "unrelated", "")];
        let filters = vec![contains_filter(FilterField::Both, "crypto")];
        assert!(compute_filter_matches(&articles, &filters).is_empty());
    }

    #[test]
    fn compute_filter_matches_empty_inputs() {
        assert!(compute_filter_matches(&[], &[]).is_empty());
        let filters = vec![contains_filter(FilterField::Both, "crypto")];
        assert!(compute_filter_matches(&[], &filters).is_empty());
        let articles = vec![fake_article(1, 10, "crypto news", "")];
        assert!(compute_filter_matches(&articles, &[]).is_empty());
    }

    #[test]
    fn compile_filter_field_title_row() {
        let row = filter_row_with_field("title", "contains", "spam");
        let compiled = compile_filter(&row, None).unwrap();
        assert_eq!(compiled.field, FilterField::Title);
        assert!(matches(&compiled, "spam alert", "harmless"));
        assert!(!matches(&compiled, "harmless", "spam in body"));
    }

    #[test]
    fn compile_filter_field_content_row() {
        let row = filter_row_with_field("content", "contains", "spam");
        let compiled = compile_filter(&row, None).unwrap();
        assert_eq!(compiled.field, FilterField::Content);
        assert!(!matches(&compiled, "spam alert", "harmless"));
        assert!(matches(&compiled, "harmless", "spam in body"));
    }

    #[test]
    fn compile_filter_unrecognized_field_defaults_to_both() {
        let row = filter_row_with_field("bogus", "contains", "spam");
        let compiled = compile_filter(&row, None).unwrap();
        assert_eq!(compiled.field, FilterField::Both);
        assert!(matches(&compiled, "spam alert", "harmless"));
        assert!(matches(&compiled, "harmless", "spam in body"));
    }

    #[test]
    fn compile_filter_contains_match_type_is_case_insensitive() {
        let row = filter_row_with_field("both", "contains", "SPAM");
        let compiled = compile_filter(&row, None).unwrap();
        assert!(matches!(compiled.matcher, CompiledMatcher::Contains(_)));
        assert!(matches(&compiled, "a spam alert", ""));
    }

    #[test]
    fn matches_text_regex_arm_for_title_field() {
        let row = filter_row_with_field("title", "regex", r"^\d+$");
        let compiled = compile_filter(&row, None).unwrap();
        assert!(matches(&compiled, "12345", "irrelevant"));
        assert!(!matches(&compiled, "not digits", "12345"));
    }

    #[test]
    fn matches_text_regex_arm_for_content_field() {
        let row = filter_row_with_field("content", "regex", r"^\d+$");
        let compiled = compile_filter(&row, None).unwrap();
        assert!(!matches(&compiled, "12345", "irrelevant"));
        assert!(matches(&compiled, "irrelevant", "12345"));
    }

    fn filter_row_with_field(field: &str, match_type: &str, pattern: &str) -> database::Filter {
        database::Filter {
            pk: 1,
            name: "test".to_string(),
            field: field.to_string(),
            match_type: match_type.to_string(),
            pattern: pattern.to_string(),
            enabled: true,
            created_at: 0,
        }
    }
}
