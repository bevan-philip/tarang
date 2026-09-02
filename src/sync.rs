use crate::database::{Db, DbResult, list_feeds_due_for_refresh, update_feed_last_refresh};
use crate::feed::{FeedError, FeedResult, update_feed_articles};
use crate::filter;
use futures::stream::{self, StreamExt};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub async fn sync_feeds(db: &Db, client: &reqwest::Client) -> DbResult<()> {
    let feeds = list_feeds_due_for_refresh(db).await?;
    let filters = Arc::new(filter::load_compiled_filters(db).await?);

    stream::iter(feeds)
        .map(|feed| {
            let filters = Arc::clone(&filters);
            async move {
                update_feed_articles(db, feed.pk, &feed.url, client, &filters).await?;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;
                update_feed_last_refresh(db, feed.pk, now, now + feed.refresh_interval).await?;
                Ok::<(), FeedError>(())
            }
        })
        .buffer_unordered(8)
        .for_each(|res: FeedResult<()>| async {
            if let Err(e) = res {
                tracing::error!(error = %e, "feed sync failed");
            }
        })
        .await;

    Ok(())
}
