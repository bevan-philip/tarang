use opml::{Body, Head, OPML, Outline};

use std::collections::HashMap;

use crate::database::{Category, Feed};

#[derive(PartialEq, Eq, Hash)]
pub struct ImportedFeed {
    pub name: String,
    pub url: String,
    pub display_url: Option<String>,
    pub category: Option<String>,
}
#[derive(Debug, thiserror::Error)]
pub enum OpmlError {
    #[error("failed to parse OPML: {0}")]
    Parse(#[from] opml::Error),
}

pub type OpmlResult<T> = Result<T, OpmlError>;

fn outline_to_feed(outline: opml::Outline, category: Option<String>) -> Option<ImportedFeed> {
    let url = outline.xml_url?;
    let name = outline.title.unwrap_or(outline.text);
    Some(ImportedFeed {
        name,
        url,
        display_url: outline.html_url.filter(|u| !u.is_empty()),
        category,
    })
}

pub fn parse_opml(opml_string: &str) -> OpmlResult<Vec<ImportedFeed>> {
    let parsed = OPML::from_str(opml_string)?;

    let mut imported_feeds: Vec<ImportedFeed> = Vec::new();

    for outline in parsed.body.outlines {
        if outline.outlines.is_empty() {
            if let Some(feed) = outline_to_feed(outline, None) {
                imported_feeds.push(feed);
            }
            continue;
        }

        let folder_name = outline.title.unwrap_or(outline.text);

        for child in outline.outlines {
            if let Some(feed) = outline_to_feed(child, Some(folder_name.clone())) {
                imported_feeds.push(feed);
            }
        }
    }

    Ok(imported_feeds)
}

pub fn export_opml(feeds: Vec<Feed>, categories: Vec<Category>) -> OpmlResult<String> {
    let mut outlines: Vec<Outline> = Vec::new();

    let mut feeds_by_category: HashMap<Option<i64>, Vec<Feed>> =
        feeds.into_iter().fold(HashMap::new(), |mut acc, f| {
            acc.entry(f.category).or_default().push(f);
            acc
        });

    for feed in feeds_by_category.remove(&None).unwrap_or_default() {
        let outline = Outline {
            text: feed.name.clone(),
            title: Some(feed.name),
            xml_url: Some(feed.url),
            html_url: (!feed.display_url.is_empty()).then_some(feed.display_url),
            r#type: Some(String::from("rss")),
            ..Default::default()
        };
        outlines.push(outline)
    }

    for category in categories {
        let mut feed_outlines: Vec<Outline> = Vec::new();

        for feed in feeds_by_category
            .remove(&Some(category.pk))
            .unwrap_or_default()
        {
            let outline = Outline {
                text: feed.name.clone(),
                title: Some(feed.name),
                xml_url: Some(feed.url),
                html_url: (!feed.display_url.is_empty()).then_some(feed.display_url),
                r#type: Some(String::from("rss")),
                ..Default::default()
            };
            feed_outlines.push(outline)
        }

        outlines.push(Outline {
            text: category.name.clone(),
            title: Some(category.name),
            outlines: feed_outlines,
            ..Default::default()
        });
    }

    let body = Body { outlines };
    let opml = OPML {
        head: Some(Head {
            title: Some(String::from("Tarang RSS export")),
            ..Default::default()
        }),
        body,
        ..Default::default()
    };

    Ok(opml.to_string()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_feeds_only() {
        let opml = r#"<?xml version="1.0"?>
<opml version="2.0">
  <head><title>Test</title></head>
  <body>
    <outline text="Feed One" title="Feed One" type="rss" xmlUrl="https://example.com/one.xml"/>
    <outline text="Feed Two" title="Feed Two" type="rss" xmlUrl="https://example.com/two.xml"/>
  </body>
</opml>"#;

        let feeds = parse_opml(opml).unwrap();
        assert_eq!(feeds.len(), 2);
        assert!(feeds.iter().all(|f| f.category.is_none()));
    }

    #[test]
    fn one_level_of_category_nesting() {
        let opml = r#"<?xml version="1.0"?>
<opml version="2.0">
  <head><title>Test</title></head>
  <body>
    <outline text="News" title="News">
      <outline text="Feed One" title="Feed One" type="rss" xmlUrl="https://example.com/one.xml"/>
    </outline>
    <outline text="Feed Two" title="Feed Two" type="rss" xmlUrl="https://example.com/two.xml"/>
  </body>
</opml>"#;

        let feeds = parse_opml(opml).unwrap();
        assert_eq!(feeds.len(), 2);
        assert_eq!(
            feeds
                .iter()
                .find(|f| f.url.ends_with("one.xml"))
                .unwrap()
                .category,
            Some("News".to_string())
        );
        assert_eq!(
            feeds
                .iter()
                .find(|f| f.url.ends_with("two.xml"))
                .unwrap()
                .category,
            None
        );
    }

    #[test]
    fn deep_nesting_is_silently_dropped() {
        let opml = r#"<?xml version="1.0"?>
<opml version="2.0">
  <head><title>Test</title></head>
  <body>
    <outline text="News" title="News">
      <outline text="Feed One" title="Feed One" type="rss" xmlUrl="https://example.com/one.xml"/>
      <outline text="Tech" title="Tech">
        <outline text="Feed Two" title="Feed Two" type="rss" xmlUrl="https://example.com/two.xml"/>
      </outline>
    </outline>
  </body>
</opml>"#;

        let feeds = parse_opml(opml).unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].url, "https://example.com/one.xml");
        assert_eq!(feeds[0].category, Some("News".to_string()));
    }

    #[test]
    fn import_reads_html_url() {
        let opml = r#"<?xml version="1.0"?>
<opml version="2.0">
  <head><title>Test</title></head>
  <body>
    <outline text="Feed One" title="Feed One" type="rss" xmlUrl="https://example.com/one.xml" htmlUrl="https://example.com/one"/>
    <outline text="Feed Two" title="Feed Two" type="rss" xmlUrl="https://example.com/two.xml"/>
  </body>
</opml>"#;

        let feeds = parse_opml(opml).unwrap();
        assert_eq!(
            feeds
                .iter()
                .find(|f| f.url.ends_with("one.xml"))
                .unwrap()
                .display_url,
            Some("https://example.com/one".to_string())
        );
        assert_eq!(
            feeds
                .iter()
                .find(|f| f.url.ends_with("two.xml"))
                .unwrap()
                .display_url,
            None
        );
    }

    #[test]
    fn import_treats_empty_html_url_as_absent() {
        let opml = r#"<?xml version="1.0"?>
<opml version="2.0">
  <head><title>Test</title></head>
  <body>
    <outline text="Feed" title="Feed" type="rss" xmlUrl="https://example.com/feed.xml" htmlUrl=""/>
  </body>
</opml>"#;

        let feeds = parse_opml(opml).unwrap();
        assert_eq!(feeds[0].display_url, None);
    }

    fn make_feed(pk: i64, name: &str, url: &str, category: Option<i64>) -> Feed {
        make_feed_with_display_url(pk, name, url, "", category)
    }

    fn make_feed_with_display_url(
        pk: i64,
        name: &str,
        url: &str,
        display_url: &str,
        category: Option<i64>,
    ) -> Feed {
        Feed {
            pk,
            name: name.to_string(),
            url: url.to_string(),
            display_url: display_url.to_string(),
            category,
            metadata: String::new(),
            refresh_interval: 0,
            last_refresh: None,
            next_poll_at: None,
            greader_hidden: false,
        }
    }

    #[test]
    fn export_uncategorized_feeds_are_flat() {
        let feeds = vec![
            make_feed(1, "Feed One", "https://example.com/one.xml", None),
            make_feed(2, "Feed Two", "https://example.com/two.xml", None),
        ];

        let xml = export_opml(feeds, vec![]).unwrap();
        let parsed = OPML::from_str(&xml).unwrap();

        assert_eq!(parsed.body.outlines.len(), 2);

        let one = parsed
            .body
            .outlines
            .iter()
            .find(|o| o.text == "Feed One")
            .unwrap();
        assert!(one.outlines.is_empty());
        assert_eq!(one.xml_url, Some("https://example.com/one.xml".to_string()));
        assert_eq!(one.html_url, None);

        let two = parsed
            .body
            .outlines
            .iter()
            .find(|o| o.text == "Feed Two")
            .unwrap();
        assert!(two.outlines.is_empty());
        assert_eq!(two.xml_url, Some("https://example.com/two.xml".to_string()));
    }

    #[test]
    fn export_writes_html_url_when_set() {
        let feeds = vec![make_feed_with_display_url(
            1,
            "Feed One",
            "https://example.com/one.xml",
            "https://example.com/one",
            None,
        )];

        let xml = export_opml(feeds, vec![]).unwrap();
        let parsed = OPML::from_str(&xml).unwrap();

        assert_eq!(
            parsed.body.outlines[0].html_url,
            Some("https://example.com/one".to_string())
        );
    }

    #[test]
    fn export_groups_feeds_by_category() {
        let feeds = vec![
            make_feed(1, "Feed One", "https://example.com/one.xml", Some(10)),
            make_feed(2, "Feed Two", "https://example.com/two.xml", Some(20)),
            make_feed(3, "Feed Three", "https://example.com/three.xml", None),
        ];
        let categories = vec![
            Category {
                pk: 10,
                name: "News".to_string(),
            },
            Category {
                pk: 20,
                name: "Tech".to_string(),
            },
        ];

        let xml = export_opml(feeds, categories).unwrap();
        let parsed = OPML::from_str(&xml).unwrap();

        assert_eq!(parsed.body.outlines.len(), 3);

        let news = parsed
            .body
            .outlines
            .iter()
            .find(|o| o.text == "News")
            .unwrap();
        assert_eq!(news.outlines.len(), 1);
        assert_eq!(
            news.outlines[0].xml_url,
            Some("https://example.com/one.xml".to_string())
        );

        let tech = parsed
            .body
            .outlines
            .iter()
            .find(|o| o.text == "Tech")
            .unwrap();
        assert_eq!(tech.outlines.len(), 1);
        assert_eq!(
            tech.outlines[0].xml_url,
            Some("https://example.com/two.xml".to_string())
        );

        let uncategorized = parsed
            .body
            .outlines
            .iter()
            .find(|o| o.text == "Feed Three")
            .unwrap();
        assert!(uncategorized.outlines.is_empty());
        assert_eq!(
            uncategorized.xml_url,
            Some("https://example.com/three.xml".to_string())
        );
    }

    #[test]
    fn export_feeds_with_unknown_category_are_dropped() {
        let feeds = vec![make_feed(
            1,
            "Feed One",
            "https://example.com/one.xml",
            Some(99),
        )];

        let xml = export_opml(feeds, vec![]).unwrap();

        // The exported body ends up with no outlines at all, since the feed's
        // category (99) never matches any category in `categories` and so is
        // never drained out of `feeds_by_category`. The `opml` crate's parser
        // rejects a body with no outlines per spec, which is itself evidence
        // the feed was silently dropped from the export.
        assert!(matches!(
            OPML::from_str(&xml),
            Err(opml::Error::BodyHasNoOutlines)
        ));
    }
}
