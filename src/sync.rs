use crate::database::{Db, list_feeds_due_for_refresh};
use crate::feed::update_feed_articles;
use futures::stream::{self, StreamExt};
use std::error::Error;

async fn sync_feed(db: &Db) -> Result<(), Box<dyn Error>> {
    let feeds = list_feeds_due_for_refresh(db).await?;

    stream::iter(feeds)
        .map(|feed| async move { update_feed_articles(db, feed.pk, feed.url).await })
        .buffer_unordered(8)
        .for_each(|res| async {
            if let Err(e) = res {
                eprintln!("feed sync failed: {e}")
            }
        })
        .await;

    Ok(())
}
