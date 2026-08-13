use crate::database::{Db, list_feeds_due_for_refresh, update_feed_last_refresh};
use crate::feed::update_feed_articles;
use futures::stream::{self, StreamExt};
use std::error::Error;
use std::time::{SystemTime, UNIX_EPOCH};

pub async fn sync_feeds(db: &Db) -> Result<(), Box<dyn Error + Send + Sync>> {
    let feeds = list_feeds_due_for_refresh(db).await?;

    stream::iter(feeds)
        .map(|feed| async move {
            update_feed_articles(db, feed.pk, feed.url).await?;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;
            update_feed_last_refresh(db, feed.pk, now, now + feed.refresh_interval).await
        })
        .buffer_unordered(8)
        .for_each(|res| async {
            if let Err(e) = res {
                eprintln!("feed sync failed: {e}")
            }
        })
        .await;

    Ok(())
}
