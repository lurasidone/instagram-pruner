use std::collections::HashSet;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures_util::stream::{self, StreamExt};
use tokio::sync::mpsc;

use crate::client::InstagramClient;
use crate::error::IgError;
use crate::error::IgResult;
use crate::events::{ChannelStats, LogLevel, Phase, PrunerEvent};
use crate::history::{text_preview, AttachmentMeta};
use crate::model::{Comment, DirectMessage, DirectThread, Media};
use crate::util::truncate;

use super::dates::{dm_timestamp_us, media_taken_at};
use super::filters::{
    in_date_window, should_delete_comment, should_delete_dm, should_delete_media,
};
use super::resolve::ResolvedTargets;
use super::{ItemKind, Plan, PlannedDelete};

// DM thread endpoint caps `limit` at 100; 50 keeps replies small enough to page quickly.
const DM_PAGE_SIZE: u32 = 50;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ScanLimits {
    pub max_pages: u32,
    pub emit_channel_events: bool,
    pub page_size: u32,
    pub concurrency: u32,
}

impl ScanLimits {
    pub fn first_pass(concurrency: u32) -> Self {
        Self {
            max_pages: 500,
            emit_channel_events: true,
            page_size: 33,
            concurrency: concurrency.max(1),
        }
    }

    pub fn verify_pass(concurrency: u32) -> Self {
        Self {
            max_pages: 500,
            emit_channel_events: false,
            page_size: 33,
            concurrency: concurrency.max(1),
        }
    }
}

pub(crate) struct ScanCtx<'a> {
    pub client: &'a InstagramClient,
    pub before: Option<DateTime<Utc>>,
    pub after: Option<DateTime<Utc>>,
    pub limits: ScanLimits,
    pub cancel: &'a Arc<AtomicBool>,
    pub tx: &'a mpsc::Sender<PrunerEvent>,
}

impl ScanCtx<'_> {
    async fn channel_started(&self, id: &str, name: &str, index: usize, total: usize) {
        if !self.limits.emit_channel_events {
            return;
        }
        let _ = self
            .tx
            .send(PrunerEvent::ChannelStarted {
                channel_id: id.to_string(),
                channel_name: name.to_string(),
                index,
                total,
                phase: Phase::Scan,
            })
            .await;
    }

    async fn channel_done(&self, id: &str, scanned: u64, added: u64, failed: u64) {
        if !self.limits.emit_channel_events {
            return;
        }
        let _ = self
            .tx
            .send(PrunerEvent::ChannelDone {
                channel_id: id.to_string(),
                stats: ChannelStats {
                    scanned,
                    deleted: added,
                    skipped: scanned.saturating_sub(added),
                    failed,
                },
                phase: Phase::Scan,
            })
            .await;
    }

    async fn warn(&self, message: String) {
        self.walker().warn(message).await;
    }

    fn walker(&self) -> PageWalker<'_> {
        PageWalker {
            limits: self.limits,
            cancel: self.cancel.as_ref(),
            tx: self.tx,
        }
    }
}

#[derive(Default)]
struct PassOutcome {
    scanned: u64,
    added: u64,
    failed: u64,
    entries: Vec<PlannedDelete>,
}

impl PassOutcome {
    fn absorb(&mut self, other: PassOutcome) {
        self.scanned += other.scanned;
        self.added += other.added;
        self.failed += other.failed;
        self.entries.extend(other.entries);
    }
}

struct PageLog<'a> {
    label: &'a str,
    error: String,
    suffix: &'a str,
    emit_progress: bool,
    cap: Option<(&'a str, &'a str)>,
}

/// The slice of `ScanCtx` that a page walk needs, so the walk can be driven
/// without a client.
struct PageWalker<'a> {
    limits: ScanLimits,
    cancel: &'a AtomicBool,
    tx: &'a mpsc::Sender<PrunerEvent>,
}

impl PageWalker<'_> {
    async fn warn(&self, message: String) {
        let _ = self
            .tx
            .send(PrunerEvent::Log {
                level: LogLevel::Warning,
                message,
            })
            .await;
    }

    async fn emit_progress(&self, log: &PageLog<'_>, pages: u32, out: &PassOutcome) {
        if !log.emit_progress || !self.limits.emit_channel_events {
            return;
        }
        let _ = self
            .tx
            .send(PrunerEvent::Log {
                level: LogLevel::Info,
                message: format!(
                    "···  {}  p{pages}  {} seen · {} {}",
                    log.label, out.scanned, out.added, log.suffix
                ),
            })
            .await;
    }

    // One cursor-walk skeleton: fetch a page, hand the items to `visit`, emit
    // progress, advance the cursor while it keeps changing. `start` seeds DM
    // threads with their preview-batch cursor; `seed` carries counters
    // accumulated before the walk. `fetch` takes the cursor by value so its
    // future stays a single concrete `Send` type.
    async fn walk<T, F, Fut, V>(
        &self,
        limit: usize,
        seed: PassOutcome,
        start: Option<String>,
        log: &PageLog<'_>,
        mut fetch: F,
        mut visit: V,
    ) -> PassOutcome
    where
        F: FnMut(Option<String>) -> Fut,
        Fut: Future<Output = IgResult<(Vec<T>, Option<String>)>> + Send,
        V: FnMut(&[T], &mut PassOutcome) -> bool,
    {
        let mut out = seed;
        let mut cursor = start;
        let mut pages = 0u32;

        while pages < self.limits.max_pages {
            if self.cancel.load(Ordering::Relaxed) {
                break;
            }
            if limit > 0 && out.entries.len() >= limit {
                break;
            }
            pages += 1;
            let (items, next) = match fetch(cursor.clone()).await {
                Ok(page) => page,
                Err(IgError::Cancelled) => break,
                Err(e) => {
                    out.failed += 1;
                    self.warn(format!("{}: {e}", log.error)).await;
                    break;
                }
            };
            if items.is_empty() {
                break;
            }
            let stop = visit(&items, &mut out);
            self.emit_progress(log, pages, &out).await;
            if stop {
                break;
            }
            match next {
                Some(n) if !n.is_empty() && Some(n.clone()) != cursor => cursor = Some(n),
                _ => break,
            }
        }

        if pages >= self.limits.max_pages {
            if let Some((label, tail)) = log.cap {
                self.warn(format!(
                    "{label}: hit page cap ({pages}/{}); {tail}",
                    self.limits.max_pages
                ))
                .await;
            }
        }

        out
    }
}

pub(crate) async fn build_plan(ctx: &ScanCtx<'_>, scope: &ResolvedTargets, limit: usize) -> Plan {
    let mut plan = Plan::default();
    let total = super::resolve::target_count(scope);
    let mut index = 0usize;

    if scope.posts {
        if ctx.cancel.load(Ordering::Relaxed) {
            return plan;
        }
        ctx.channel_started("posts", "Your posts", index, total)
            .await;
        let out = scan_posts(ctx, limit).await;
        plan.entries.extend(out.entries);
        plan.messages_scanned += out.scanned;
        plan.channels_processed += 1;
        ctx.channel_done("posts", out.scanned, out.added, out.failed)
            .await;
        index += 1;
        if limit > 0 && plan.entries.len() >= limit {
            plan.entries.truncate(limit);
            return plan;
        }
    }

    if limit == 0 && scope.threads.len() > 1 {
        scan_threads_parallel(ctx, &scope.threads, total, index, &mut plan).await;
    } else {
        for thread in &scope.threads {
            if ctx.cancel.load(Ordering::Relaxed) {
                break;
            }
            if limit > 0 && plan.entries.len() >= limit {
                break;
            }
            let tid = thread.id();
            let tname = thread.display_name();
            ctx.channel_started(&tid, &tname, index, total).await;
            let remaining = if limit > 0 {
                limit.saturating_sub(plan.entries.len())
            } else {
                0
            };
            let out = scan_thread(ctx, thread, remaining).await;
            plan.entries.extend(out.entries);
            plan.messages_scanned += out.scanned;
            plan.channels_processed += 1;
            ctx.channel_done(&tid, out.scanned, out.added, out.failed)
                .await;
            index += 1;
        }
    }

    let content = scope.content;
    let mut passes: Vec<ContentPass> = Vec::new();
    if content.likes {
        passes.push(ContentPass::Feed(ActivityFeed::Likes));
    }
    if content.saved {
        passes.push(ContentPass::Feed(ActivityFeed::Saved));
    }
    if content.comments {
        passes.push(ContentPass::Comments);
    }
    if content.archived {
        passes.push(ContentPass::Feed(ActivityFeed::Archived));
    }
    if content.reposts {
        passes.push(ContentPass::Reposts);
    }

    let index_base = index;
    let mut results: Vec<(usize, PassOutcome)> = Vec::new();
    if limit > 0 {
        for (pass_offset, pass) in passes.into_iter().enumerate() {
            if ctx.cancel.load(Ordering::Relaxed) || plan.entries.len() >= limit {
                break;
            }
            let remaining = limit.saturating_sub(plan.entries.len());
            results.push(
                run_content_pass(ctx, pass, index_base + pass_offset, total, remaining).await,
            );
        }
    } else {
        let pass_fanout = passes.len().min(ctx.limits.concurrency as usize).max(1);
        results = stream::iter(passes)
            .enumerate()
            .map(|(pass_offset, pass)| {
                run_content_pass(ctx, pass, index_base + pass_offset, total, 0)
            })
            .buffer_unordered(pass_fanout)
            .collect()
            .await;
    }

    results.sort_by_key(|(pass_index, _)| *pass_index);
    for (_pass_index, out) in results {
        plan.entries.extend(out.entries);
        plan.messages_scanned += out.scanned;
        plan.channels_processed += 1;
    }

    if limit > 0 && plan.entries.len() > limit {
        plan.entries.truncate(limit);
    }
    plan
}

async fn run_content_pass(
    ctx: &ScanCtx<'_>,
    pass: ContentPass,
    pass_index: usize,
    total: usize,
    limit: usize,
) -> (usize, PassOutcome) {
    if ctx.cancel.load(Ordering::Relaxed) {
        return (pass_index, PassOutcome::default());
    }
    ctx.channel_started(pass.channel_id(), pass.channel_name(), pass_index, total)
        .await;
    let out = pass.scan(ctx, limit).await;
    ctx.channel_done(pass.channel_id(), out.scanned, out.added, out.failed)
        .await;
    (pass_index, out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentPass {
    Feed(ActivityFeed),
    Comments,
    Reposts,
}

impl ContentPass {
    fn channel_id(self) -> &'static str {
        match self {
            Self::Feed(feed) => feed.channel_id(),
            Self::Comments => "comments",
            Self::Reposts => "reposts",
        }
    }

    fn channel_name(self) -> &'static str {
        match self {
            Self::Feed(feed) => feed.channel_name(),
            Self::Comments => "Own comments",
            Self::Reposts => "Reposts",
        }
    }

    async fn scan(self, ctx: &ScanCtx<'_>, limit: usize) -> PassOutcome {
        match self {
            Self::Feed(feed) => scan_activity_feed(ctx, feed, limit).await,
            Self::Comments => scan_comments(ctx, limit).await,
            Self::Reposts => scan_reposts(ctx, limit).await,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActivityFeed {
    Likes,
    Saved,
    Archived,
}

impl ActivityFeed {
    fn channel_id(self) -> &'static str {
        match self {
            Self::Likes => "likes",
            Self::Saved => "saved",
            Self::Archived => "archived",
        }
    }

    fn channel_name(self) -> &'static str {
        match self {
            Self::Likes => "Liked posts",
            Self::Saved => "Saved posts",
            Self::Archived => "Archived posts",
        }
    }

    fn item_kind(self) -> ItemKind {
        match self {
            Self::Likes => ItemKind::Like,
            Self::Saved => ItemKind::Saved,
            Self::Archived => ItemKind::Archived,
        }
    }

    async fn page(
        self,
        client: &InstagramClient,
        cursor: Option<&str>,
    ) -> IgResult<(Vec<Media>, Option<String>)> {
        match self {
            Self::Likes => client.liked_medias_page(cursor).await,
            Self::Saved => client.saved_medias_page(cursor).await,
            Self::Archived => client.archived_medias_page(cursor).await,
        }
    }
}

fn undo_item_id(feed: ActivityFeed, media: &Media, self_uid: &str) -> String {
    match feed {
        ActivityFeed::Likes => strip_mccr(&media.media_id()),
        ActivityFeed::Saved => media.pk.clone(),
        ActivityFeed::Archived => {
            let full = media.media_id();
            if full.contains('_') || self_uid.is_empty() {
                full
            } else {
                format!("{full}_{self_uid}")
            }
        }
    }
}

// Like ids come back cache-marked ("<id>_mccr<hex>"); the unlike endpoint wants the bare id.
fn strip_mccr(id: &str) -> String {
    match id.rsplit_once("_mccr") {
        Some((head, marker))
            if !head.is_empty()
                && !marker.is_empty()
                && marker.chars().all(|c| c.is_ascii_hexdigit()) =>
        {
            head.to_string()
        }
        _ => id.to_string(),
    }
}

async fn scan_activity_feed(ctx: &ScanCtx<'_>, feed: ActivityFeed, limit: usize) -> PassOutcome {
    let client = ctx.client;
    let uid = client.user_id();
    let uname = client.username();
    let channel_id = feed.channel_id();

    ctx.walker()
        .walk::<Media, _, _, _>(
            limit,
            PassOutcome::default(),
            None,
            &PageLog {
                label: channel_id,
                error: format!("{channel_id} page failed"),
                suffix: "to undo",
                emit_progress: true,
                cap: Some((channel_id, "older items may remain")),
            },
            move |cursor: Option<String>| async move { feed.page(client, cursor.as_deref()).await },
            |items: &[Media], out: &mut PassOutcome| {
                for media in items {
                    out.scanned += 1;
                    if !in_date_window(media_taken_at(media.taken_at), ctx.before, ctx.after) {
                        continue;
                    }
                    let mut entry = PlannedDelete::from_media(media, &uname);
                    entry.kind = feed.item_kind();
                    entry.target_id = channel_id.to_string();
                    entry.target_name = feed.channel_name().to_string();
                    entry.item_id = undo_item_id(feed, media, &uid);
                    if entry.item_id.is_empty() {
                        continue;
                    }
                    entry.author_name = media.author_username();
                    entry.author_id = media.author_id();
                    out.entries.push(entry);
                    out.added += 1;
                    if limit > 0 && out.entries.len() >= limit {
                        break;
                    }
                }
                false
            },
        )
        .await
}

// Own-comments pass: the per-post body is itself async (bounded fan-out of comment
// pages), so it cannot ride the synchronous visitor of `PageWalker::walk`.
async fn scan_comments(ctx: &ScanCtx<'_>, limit: usize) -> PassOutcome {
    let uid = ctx.client.user_id();
    let page_size = ctx.limits.page_size;
    let fanout = if limit > 0 {
        1
    } else {
        ctx.limits.concurrency.max(1) as usize
    };
    let log = PageLog {
        label: "comments",
        error: "comments: own feed page failed".into(),
        suffix: "own",
        emit_progress: true,
        cap: None,
    };

    let mut out = PassOutcome::default();
    let mut cursor: Option<String> = None;
    let mut pages = 0u32;

    while pages < ctx.limits.max_pages {
        if ctx.cancel.load(Ordering::Relaxed) {
            break;
        }
        if limit > 0 && out.entries.len() >= limit {
            break;
        }
        pages += 1;
        let (posts, next) = match ctx
            .client
            .user_medias_page(&uid, cursor.as_deref(), page_size)
            .await
        {
            Ok(page) => page,
            Err(IgError::Cancelled) => break,
            Err(e) => {
                out.failed += 1;
                ctx.warn(format!("{}: {e}", log.error)).await;
                break;
            }
        };
        if posts.is_empty() {
            break;
        }

        let mut stopped = false;
        if fanout > 1 {
            let mut scans = Vec::with_capacity(posts.len());
            for media in &posts {
                scans.push(scan_post_comments(ctx, media, limit));
            }
            let results: Vec<PassOutcome> =
                stream::iter(scans).buffer_unordered(fanout).collect().await;
            for result in results {
                out.absorb(result);
            }
            stopped = ctx.cancel.load(Ordering::Relaxed);
        } else {
            for media in &posts {
                if ctx.cancel.load(Ordering::Relaxed) || (limit > 0 && out.entries.len() >= limit) {
                    stopped = true;
                    break;
                }
                let result = scan_post_comments(ctx, media, limit).await;
                out.absorb(result);
                if limit > 0 && out.entries.len() >= limit {
                    stopped = true;
                    break;
                }
            }
        }
        if stopped {
            break;
        }

        ctx.walker().emit_progress(&log, pages, &out).await;
        match next {
            Some(n) if !n.is_empty() && Some(n.clone()) != cursor => cursor = Some(n),
            _ => break,
        }
    }

    out
}

async fn scan_post_comments(ctx: &ScanCtx<'_>, media: &Media, limit: usize) -> PassOutcome {
    let client = ctx.client;
    let uid = client.user_id();
    let uname = client.username();
    let media_id = media.media_id();
    let media_id_ref = media_id.as_str();

    ctx.walker()
        .walk::<Comment, _, _, _>(
            limit,
            PassOutcome::default(),
            None,
            &PageLog {
                label: "comments",
                error: format!("comments on {media_id}"),
                suffix: "own",
                emit_progress: false,
                cap: None,
            },
            move |cursor: Option<String>| async move {
                client
                    .media_comments_page(media_id_ref, cursor.as_deref())
                    .await
            },
            |comments: &[Comment], out: &mut PassOutcome| {
                for comment in comments {
                    out.scanned += 1;
                    if !should_delete_comment(comment, &uid, media_id_ref, ctx.before, ctx.after) {
                        continue;
                    }
                    out.entries
                        .push(PlannedDelete::from_comment(comment, media, &uname));
                    out.added += 1;
                    if limit > 0 && out.entries.len() >= limit {
                        break;
                    }
                }
                false
            },
        )
        .await
}

async fn scan_reposts(ctx: &ScanCtx<'_>, limit: usize) -> PassOutcome {
    let client = ctx.client;
    let collections = match client.collections().await {
        Ok(c) => c,
        Err(e) => {
            ctx.warn(format!("reposts: collections/list/ failed: {e}"))
                .await;
            return PassOutcome {
                failed: 1,
                ..Default::default()
            };
        }
    };

    let repost_cols: Vec<_> = collections
        .iter()
        .filter(|c| {
            let name = c.collection_name.to_ascii_lowercase();
            let ty = c
                .collection_type
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            name.contains("repost") || name.contains("reshare") || ty.contains("repost")
        })
        .collect();

    if repost_cols.is_empty() {
        ctx.warn(format!(
            "reposts: no repost collection on this account ({} collection(s) seen: {}): \
             Instagram exposes no repost endpoint, so nothing can be undone",
            collections.len(),
            collections
                .iter()
                .map(|c| c.collection_name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
        .await;
        return PassOutcome::default();
    }

    let uname = client.username();
    let mut out = PassOutcome::default();

    for collection in repost_cols {
        if limit > 0 && out.entries.len() >= limit {
            break;
        }
        let remaining = if limit > 0 {
            limit.saturating_sub(out.entries.len())
        } else {
            0
        };
        let collection_id = collection.collection_id.as_str();
        let collection_name = collection.collection_name.as_str();
        let page = ctx
            .walker()
            .walk::<Media, _, _, _>(
                remaining,
                PassOutcome::default(),
                None,
                &PageLog {
                    label: "reposts",
                    error: format!("reposts: collection {collection_id} failed"),
                    suffix: "own",
                    emit_progress: false,
                    cap: None,
                },
                move |cursor: Option<String>| async move {
                    client
                        .collection_medias_page(collection_id, cursor.as_deref())
                        .await
                },
                |items: &[Media], out: &mut PassOutcome| {
                    for media in items {
                        out.scanned += 1;
                        if !in_date_window(media_taken_at(media.taken_at), ctx.before, ctx.after) {
                            continue;
                        }
                        let mut entry = PlannedDelete::from_media(media, &uname);
                        entry.kind = ItemKind::Repost;
                        entry.target_id = collection_id.to_string();
                        entry.target_name = format!("Reposts ({collection_name})");
                        entry.item_id = media.pk.clone();
                        if entry.item_id.is_empty() {
                            continue;
                        }
                        out.entries.push(entry);
                        out.added += 1;
                        if remaining > 0 && out.entries.len() >= remaining {
                            break;
                        }
                    }
                    false
                },
            )
            .await;
        out.absorb(page);
    }

    out
}

async fn scan_threads_parallel(
    ctx: &ScanCtx<'_>,
    threads: &[DirectThread],
    total: usize,
    index_base: usize,
    plan: &mut Plan,
) {
    let _ = ctx
        .tx
        .send(PrunerEvent::Log {
            level: LogLevel::Info,
            message: format!(
                "SCAN  {} DM thread(s) ×{} parallel · full history · own only",
                threads.len(),
                ctx.limits.concurrency
            ),
        })
        .await;

    let mut results: Vec<(usize, PassOutcome)> = stream::iter(threads.iter().cloned().enumerate())
        .map(|(offset, thread)| {
            let index = index_base + offset;
            async move {
                let tid = thread.id();
                let tname = thread.display_name();
                ctx.channel_started(&tid, &tname, index, total).await;
                let out = scan_thread(ctx, &thread, 0).await;
                ctx.channel_done(&tid, out.scanned, out.added, out.failed)
                    .await;
                (index, out)
            }
        })
        .buffer_unordered(ctx.limits.concurrency as usize)
        .collect()
        .await;

    results.sort_by_key(|(index, _)| *index);
    for (_index, out) in results {
        plan.entries.extend(out.entries);
        plan.messages_scanned += out.scanned;
        plan.channels_processed += 1;
    }
}

async fn scan_posts(ctx: &ScanCtx<'_>, limit: usize) -> PassOutcome {
    let client = ctx.client;
    let uid = client.user_id();
    let uid_ref = uid.as_str();
    let uname = client.username();
    let page_size = ctx.limits.page_size;
    let after = ctx.after;

    ctx.walker()
        .walk::<Media, _, _, _>(
            limit,
            PassOutcome::default(),
            None,
            &PageLog {
                label: "posts",
                error: "posts page failed".into(),
                suffix: "own",
                emit_progress: true,
                cap: Some(("posts", "older media may remain unscanned")),
            },
            move |cursor: Option<String>| async move {
                client
                    .user_medias_page(uid_ref, cursor.as_deref(), page_size)
                    .await
            },
            |items: &[Media], out: &mut PassOutcome| {
                let mut page_all_old = after.is_some();
                let mut saw_timestamp = false;
                for media in items {
                    out.scanned += 1;
                    match media_taken_at(media.taken_at) {
                        Some(t) => {
                            saw_timestamp = true;
                            if let Some(a) = after {
                                if t > a {
                                    page_all_old = false;
                                }
                            }
                        }
                        None => page_all_old = false,
                    }
                    if !should_delete_media(media, &uid, ctx.before, ctx.after) {
                        continue;
                    }
                    out.entries.push(PlannedDelete::from_media(media, &uname));
                    out.added += 1;
                    if limit > 0 && out.entries.len() >= limit {
                        break;
                    }
                }
                after.is_some() && page_all_old && saw_timestamp
            },
        )
        .await
}

async fn scan_thread(
    ctx: &ScanCtx<'_>,
    thread: &DirectThread,
    remaining_limit: usize,
) -> PassOutcome {
    let client = ctx.client;
    let tid = thread.id();
    let tid_ref = tid.as_str();
    let tname = thread.display_name();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut out = PassOutcome::default();

    if !thread.items.is_empty() {
        let past_after = absorb_dm_page(
            ctx,
            &tid,
            &tname,
            &thread.items,
            remaining_limit,
            &mut seen_ids,
            &mut out,
        );
        // the thread payload carries a first batch plus the cursor for the rest
        if ctx.limits.emit_channel_events && out.scanned > 0 {
            let _ = ctx
                .tx
                .send(PrunerEvent::Log {
                    level: LogLevel::Info,
                    message: format!(
                        "···  {}  preview  {} seen · {} own",
                        truncate(&tname, 28),
                        out.scanned,
                        out.added
                    ),
                })
                .await;
        }
        if remaining_limit > 0 && out.added as usize >= remaining_limit {
            return out;
        }
        if !thread.has_older || past_after {
            return out;
        }
    }

    let start = if thread.items.is_empty() {
        None
    } else {
        thread
            .oldest_cursor
            .clone()
            .or_else(|| thread.prev_cursor.clone())
            .filter(|s| !s.is_empty())
    };
    let label = truncate(&tname, 28);
    let cap_label = format!("thread {tid}");

    ctx.walker()
        .walk::<DirectMessage, _, _, _>(
            remaining_limit,
            out,
            start,
            &PageLog {
                label: label.as_str(),
                error: format!("thread {tid} page failed"),
                suffix: "own",
                emit_progress: true,
                cap: Some((cap_label.as_str(), "older own messages may remain")),
            },
            move |cursor: Option<String>| async move {
                let page = client
                    .direct_thread(tid_ref, DM_PAGE_SIZE, cursor.as_deref())
                    .await?;
                let next = if page.has_older {
                    page.oldest_cursor
                        .or(page.prev_cursor)
                        .filter(|s| !s.is_empty())
                } else {
                    None
                };
                Ok::<_, IgError>((page.items, next))
            },
            |items: &[DirectMessage], out: &mut PassOutcome| {
                let remaining = remaining_limit.saturating_sub(out.added as usize);
                absorb_dm_page(ctx, &tid, &tname, items, remaining, &mut seen_ids, out)
            },
        )
        .await
}

fn absorb_dm_page(
    ctx: &ScanCtx<'_>,
    tid: &str,
    tname: &str,
    items: &[DirectMessage],
    remaining: usize,
    seen_ids: &mut HashSet<String>,
    out: &mut PassOutcome,
) -> bool {
    let mut page_all_old = ctx.after.is_some();
    let mut saw_timestamp = false;
    let mut page_added = 0u64;
    let uid = ctx.client.user_id();
    let uname = ctx.client.username();

    for msg in items {
        out.scanned += 1;
        match dm_timestamp_us(msg.timestamp) {
            Some(t) => {
                saw_timestamp = true;
                if let Some(a) = ctx.after {
                    if t > a {
                        page_all_old = false;
                    }
                }
            }
            None => page_all_old = false,
        }

        if !should_delete_dm(msg, &uid, ctx.before, ctx.after) {
            continue;
        }
        let mid = msg.message_id();
        if !seen_ids.insert(mid) {
            continue;
        }
        out.entries
            .push(PlannedDelete::from_dm(msg, tid, tname, &uid, &uname));
        out.added += 1;
        page_added += 1;
        if remaining > 0 && page_added as usize >= remaining {
            break;
        }
    }

    ctx.after.is_some() && page_all_old && saw_timestamp
}

impl PlannedDelete {
    pub fn from_media(media: &Media, self_username: &str) -> Self {
        let caption = media.caption_text();
        let kind_label = media.kind_label();
        let preview = text_preview(&caption, kind_label);
        let ts = media_taken_at(media.taken_at)
            .map(|d| d.to_rfc3339())
            .unwrap_or_default();
        let mut attachments = Vec::new();
        if let Some(url) = media.preview_url() {
            attachments.push(AttachmentMeta {
                filename: format!(
                    "{}.{}",
                    media.code.clone().unwrap_or_else(|| media.pk.clone()),
                    ext_for(kind_label)
                ),
                url: url.clone(),
            });
        }
        Self {
            target_id: "posts".into(),
            target_name: "Your posts".into(),
            item_id: media.media_id(),
            kind: ItemKind::Media,
            preview,
            author_id: media.author_id(),
            author_name: if self_username.is_empty() {
                media.author_username()
            } else {
                self_username.to_string()
            },
            content: caption,
            timestamp: ts,
            media_type: Some(kind_label.to_string()),
            attachments,
        }
    }

    pub fn from_dm(
        msg: &DirectMessage,
        thread_id: &str,
        thread_name: &str,
        self_id: &str,
        self_username: &str,
    ) -> Self {
        let content = msg.text.clone().unwrap_or_default();
        let kind_label = humanize_item_type(msg.item_type.as_deref());
        let preview = text_preview(&content, &kind_label);
        Self {
            target_id: thread_id.to_string(),
            target_name: thread_name.to_string(),
            item_id: msg.message_id(),
            kind: ItemKind::Dm,
            preview: truncate(&preview, 80),
            author_id: self_id.to_string(),
            author_name: self_username.to_string(),
            content,
            timestamp: msg.timestamp_rfc3339(),
            media_type: Some(kind_label),
            attachments: Vec::new(),
        }
    }

    pub fn from_comment(comment: &Comment, media: &Media, self_username: &str) -> Self {
        let content = comment.text.clone();
        let preview = text_preview(&content, "comment");
        let ts = media_taken_at(comment.created_at)
            .map(|d| d.to_rfc3339())
            .unwrap_or_default();
        Self {
            target_id: media.media_id(),
            target_name: format!("Comments on {}", media.kind_label()),
            item_id: comment.pk.clone(),
            kind: ItemKind::Comment,
            preview: truncate(&preview, 80),
            author_id: comment.author_id(),
            author_name: if self_username.is_empty() {
                comment.author_username()
            } else {
                self_username.to_string()
            },
            content,
            timestamp: ts,
            media_type: Some("comment".into()),
            attachments: Vec::new(),
        }
    }
}

fn ext_for(kind: &str) -> &'static str {
    match kind {
        "video" | "reel" | "igtv" => "mp4",
        _ => "jpg",
    }
}

fn humanize_item_type(item_type: Option<&str>) -> String {
    match item_type.map(str::trim).filter(|s| !s.is_empty()) {
        None => "message".into(),
        Some("text") => "text".into(),
        Some("media") => "media".into(),
        Some("raven_media") => "view-once".into(),
        Some("voice_media") => "voice note".into(),
        Some("animated_media") => "gif".into(),
        Some("like") => "heart".into(),
        Some("link") => "link".into(),
        Some("reel_share" | "clip" | "xma_clip") => "reel".into(),
        Some("story_share" | "xma_story_share") => "story share".into(),
        Some("media_share" | "xma_media_share" | "felix_share") => "shared post".into(),
        Some("profile" | "xma_profile") => "profile share".into(),
        Some("location") => "location".into(),
        Some("action_log") => "system activity".into(),
        Some("video_call_event" | "voice_call_event" | "call" | "video_call") => {
            "call event".into()
        }
        Some("placeholder") => "placeholder".into(),
        Some(other) if other.ends_with("_log") => "system activity".into(),
        Some(other) => other.replace('_', " "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn strip_mccr_normalises_cache_marked_like_ids() {
        assert_eq!(
            strip_mccr("3474265637340066486_5515149943_mccr40091ae1"),
            "3474265637340066486_5515149943"
        );
        assert_eq!(
            strip_mccr("3992123904827341000_331150405"),
            "3992123904827341000_331150405"
        );
    }

    #[test]
    fn strip_mccr_leaves_non_marker_ids_untouched() {
        assert_eq!(strip_mccr("111_222"), "111_222");
        assert_eq!(strip_mccr(""), "");
        assert_eq!(strip_mccr("111_222_mccr"), "111_222_mccr");
        assert_eq!(strip_mccr("111_222_mccrZZZ"), "111_222_mccrZZZ");
        assert_eq!(strip_mccr("3954879423574379060"), "3954879423574379060");
    }

    struct WalkHarness {
        limits: ScanLimits,
        tx: mpsc::Sender<PrunerEvent>,
        rx: mpsc::Receiver<PrunerEvent>,
        cancel: AtomicBool,
        calls: Arc<AtomicUsize>,
    }

    impl WalkHarness {
        fn new(max_pages: u32) -> Self {
            let (tx, rx) = mpsc::channel(64);
            Self {
                limits: ScanLimits {
                    max_pages,
                    emit_channel_events: true,
                    page_size: 10,
                    concurrency: 1,
                },
                tx,
                rx,
                cancel: AtomicBool::new(false),
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn walker(&self) -> PageWalker<'_> {
            PageWalker {
                limits: self.limits,
                cancel: &self.cancel,
                tx: &self.tx,
            }
        }

        /// Shared counter the fake `fetch` bumps on every call.
        fn counter(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.calls)
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::Relaxed)
        }

        fn warnings(&mut self) -> usize {
            let mut count = 0;
            while let Ok(event) = self.rx.try_recv() {
                if matches!(
                    event,
                    PrunerEvent::Log {
                        level: LogLevel::Warning,
                        ..
                    }
                ) {
                    count += 1;
                }
            }
            count
        }
    }

    fn page_log(cap: Option<(&'static str, &'static str)>) -> PageLog<'static> {
        PageLog {
            label: "test",
            error: "test page failed".into(),
            suffix: "seen",
            emit_progress: false,
            cap,
        }
    }

    fn count_items(items: &[u32], out: &mut PassOutcome) -> bool {
        out.scanned += items.len() as u64;
        out.added += items.len() as u64;
        false
    }

    #[tokio::test]
    async fn walk_advances_while_the_cursor_keeps_changing() {
        let h = WalkHarness::new(10);
        let seen = h.counter();
        let out = h
            .walker()
            .walk(
                0,
                PassOutcome::default(),
                None,
                &page_log(None),
                move |cursor: Option<String>| {
                    let seen = Arc::clone(&seen);
                    async move {
                        let page = match cursor.as_deref() {
                            None => (vec![1], Some("a".to_string())),
                            Some("a") => (vec![2], Some("b".to_string())),
                            _ => (vec![3], Some("b".to_string())),
                        };
                        seen.fetch_add(1, Ordering::Relaxed);
                        Ok::<_, IgError>(page)
                    }
                },
                count_items,
            )
            .await;

        // the cursor is handed back unchanged, so the walk stops instead of looping
        assert_eq!(h.calls(), 3);
        assert_eq!(out.scanned, 3);
        assert_eq!(out.failed, 0);
    }

    #[tokio::test]
    async fn walk_stops_on_an_empty_page() {
        let h = WalkHarness::new(10);
        let seen = h.counter();
        let out = h
            .walker()
            .walk(
                0,
                PassOutcome::default(),
                None,
                &page_log(None),
                move |_cursor: Option<String>| {
                    let seen = Arc::clone(&seen);
                    async move {
                        seen.fetch_add(1, Ordering::Relaxed);
                        Ok::<_, IgError>((Vec::<u32>::new(), Some("a".to_string())))
                    }
                },
                count_items,
            )
            .await;

        assert_eq!(h.calls(), 1);
        assert_eq!(out.scanned, 0);
    }

    #[tokio::test]
    async fn walk_stops_at_the_page_cap_and_warns_once() {
        let mut h = WalkHarness::new(2);
        let seen = h.counter();
        let out = h
            .walker()
            .walk(
                0,
                PassOutcome::default(),
                None,
                &page_log(Some(("posts", "older items may remain"))),
                move |_cursor: Option<String>| {
                    let seen = Arc::clone(&seen);
                    async move {
                        let page = seen.fetch_add(1, Ordering::Relaxed);
                        Ok::<_, IgError>((vec![1], Some(format!("c{page}"))))
                    }
                },
                count_items,
            )
            .await;

        assert_eq!(h.calls(), 2);
        assert_eq!(out.scanned, 2);
        assert_eq!(h.warnings(), 1);
    }

    #[tokio::test]
    async fn walk_stops_once_the_scan_limit_is_reached() {
        let h = WalkHarness::new(10);
        let seen = h.counter();
        let mut visit = |items: &[u32], out: &mut PassOutcome| {
            out.scanned += items.len() as u64;
            out.entries.push(PlannedDelete::default());
            false
        };
        let out = h
            .walker()
            .walk(
                1,
                PassOutcome::default(),
                None,
                &page_log(None),
                move |_cursor: Option<String>| {
                    let seen = Arc::clone(&seen);
                    async move {
                        seen.fetch_add(1, Ordering::Relaxed);
                        Ok::<_, IgError>((vec![7], Some("next".to_string())))
                    }
                },
                &mut visit,
            )
            .await;

        assert_eq!(h.calls(), 1);
        assert_eq!(out.entries.len(), 1);
    }

    #[tokio::test]
    async fn walk_stops_when_the_visitor_says_stop() {
        let h = WalkHarness::new(10);
        let seen = h.counter();
        let out = h
            .walker()
            .walk(
                0,
                PassOutcome::default(),
                None,
                &page_log(None),
                move |_cursor: Option<String>| {
                    let seen = Arc::clone(&seen);
                    async move {
                        seen.fetch_add(1, Ordering::Relaxed);
                        Ok::<_, IgError>((vec![1], Some("next".to_string())))
                    }
                },
                |_items: &[u32], _out: &mut PassOutcome| true,
            )
            .await;

        assert_eq!(h.calls(), 1);
        assert_eq!(out.scanned, 0);
    }

    #[tokio::test]
    async fn walk_does_not_fetch_when_already_cancelled() {
        let h = WalkHarness::new(10);
        h.cancel.store(true, Ordering::Relaxed);
        let seen = h.counter();
        let out = h
            .walker()
            .walk(
                0,
                PassOutcome::default(),
                None,
                &page_log(None),
                move |_cursor: Option<String>| {
                    let seen = Arc::clone(&seen);
                    async move {
                        seen.fetch_add(1, Ordering::Relaxed);
                        Ok::<_, IgError>((vec![1], None))
                    }
                },
                count_items,
            )
            .await;

        assert_eq!(h.calls(), 0);
        assert_eq!(out.failed, 0);
    }

    #[tokio::test]
    async fn walk_treats_an_aborted_fetch_as_a_stop_not_a_failure() {
        let mut h = WalkHarness::new(10);
        let out = h
            .walker()
            .walk(
                0,
                PassOutcome::default(),
                None,
                &page_log(None),
                |_cursor: Option<String>| async { Err::<(Vec<u32>, _), _>(IgError::Cancelled) },
                count_items,
            )
            .await;

        assert_eq!(out.failed, 0);
        assert_eq!(h.warnings(), 0);
    }

    #[tokio::test]
    async fn walk_counts_a_page_error_and_stops() {
        let mut h = WalkHarness::new(10);
        let seen = h.counter();
        let out = h
            .walker()
            .walk(
                0,
                PassOutcome::default(),
                None,
                &page_log(None),
                move |_cursor: Option<String>| {
                    let seen = Arc::clone(&seen);
                    async move {
                        seen.fetch_add(1, Ordering::Relaxed);
                        Err::<(Vec<u32>, _), _>(IgError::Api {
                            status: 500,
                            message: "boom".into(),
                        })
                    }
                },
                count_items,
            )
            .await;

        assert_eq!(h.calls(), 1);
        assert_eq!(out.failed, 1);
        assert_eq!(h.warnings(), 1);
    }
}
