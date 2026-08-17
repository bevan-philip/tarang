use opml::{Body, Head, OPML, Outline};

use std::{collections::HashMap, error::Error};

use crate::{
    database::{Category, Feed},
    feed,
};

struct ImportedFeed {
    name: String,
    url: String,
    category: Option<String>,
}
#[derive(Debug, thiserror::Error)]
pub enum OpmlError {
    #[error("failed to parse OPML: {0}")]
    Parse(#[from] opml::Error),
    #[error("outline '{0}' is missing an xmlUrl")]
    MissingUrl(String),
}

pub type OpmlResult<T> = Result<T, OpmlError>;

fn outline_to_feed(outline: opml::Outline, category: Option<String>) -> Option<ImportedFeed> {
    let url = outline.xml_url?;
    let name = outline.title.unwrap_or(outline.text);
    Some(ImportedFeed {
        name,
        url,
        category,
    })
}

async fn parse_opml(opml_string: &str) -> Result<Vec<ImportedFeed>, OpmlError> {
    let parsed = OPML::from_str(&opml_string)?;

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

async fn export_opml(feeds: Vec<Feed>, categories: Vec<Category>) -> Result<String, opml::Error> {
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

    #[tokio::test]
    async fn flat_feeds_only() {
        let opml = r#"<?xml version="1.0"?>
<opml version="2.0">
  <head><title>Test</title></head>
  <body>
    <outline text="Feed One" title="Feed One" type="rss" xmlUrl="https://example.com/one.xml"/>
    <outline text="Feed Two" title="Feed Two" type="rss" xmlUrl="https://example.com/two.xml"/>
  </body>
</opml>"#;

        let feeds = parse_opml(opml).await.unwrap();
        assert_eq!(feeds.len(), 2);
        assert!(feeds.iter().all(|f| f.category.is_none()));
    }

    #[tokio::test]
    async fn one_level_of_category_nesting() {
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

        let feeds = parse_opml(opml).await.unwrap();
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

    #[tokio::test]
    async fn deep_nesting_is_silently_dropped() {
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

        let feeds = parse_opml(opml).await.unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].url, "https://example.com/one.xml");
        assert_eq!(feeds[0].category, Some("News".to_string()));
    }
}
