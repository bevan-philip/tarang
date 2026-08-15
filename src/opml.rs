use opml::OPML;

use std::error::Error;

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
