use feed_rs::parser;
use std::error::Error;

mod database;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    scrape().await?;
    Ok(())
}

async fn scrape() -> Result<bool, Box<dyn Error>> {
    let res = reqwest::get("https://bphilip.uk/index.xml")
        .await?
        .text()
        .await?;

    let _ = rss_parse(res);

    Ok(true)
}

fn rss_parse(feed_content: String) -> Result<bool, Box<dyn Error>> {
    let feed = parser::parse(feed_content.as_bytes())?;

    for entry in feed.entries {
        println!(
            "{} at {}",
            entry.title.ok_or("entry missing title")?.content,
            entry.links[0].href
        )
    }

    Ok(true)
}
