use std::collections::HashSet;

use tokio::sync::mpsc;

use crate::client::{self, InstagramClient};
use crate::config::DmKind;
use crate::error::IgResult;
use crate::events::{LogLevel, PrunerEvent};
use crate::model::DirectThread;

use super::filters::{first_protected_user, thread_matches_kind};
use super::TargetResolution;

const MAX_INBOX_THREADS: usize = 2_000;

#[derive(Debug, Default)]
pub(crate) struct Skips {
    pub kind: usize,
    pub protected: usize,
    pub protected_names: Vec<String>,
}

// Single admission gate for every target kind: wrong dm_kind, already requested,
// or guarded by the protect list. `apply_kind` is off for explicitly named threads.
pub(crate) fn admit_thread(
    thread: &DirectThread,
    dm_kind: DmKind,
    apply_kind: bool,
    protect: &HashSet<&str>,
    seen: &mut HashSet<String>,
    skips: &mut Skips,
) -> bool {
    if apply_kind && !thread_matches_kind(thread, dm_kind) {
        skips.kind += 1;
        return false;
    }
    let id = thread.id();
    if id.is_empty() || !seen.insert(id) {
        return false;
    }
    if let Some(name) = first_protected_user(thread, protect) {
        skips.protected += 1;
        skips.protected_names.push(name);
        return false;
    }
    true
}

#[derive(Debug, Default)]
pub(crate) struct ResolvedTargets {
    pub posts: bool,
    pub threads: Vec<DirectThread>,
    pub content: super::ContentFlags,
}

pub(crate) async fn resolve_targets(
    client: &InstagramClient,
    targets: &[TargetResolution],
    content: super::ContentFlags,
    protect_users: &[String],
    dm_kind: DmKind,
    tx: &mpsc::Sender<PrunerEvent>,
) -> IgResult<ResolvedTargets> {
    let protect: HashSet<&str> = protect_users.iter().map(|s| s.as_str()).collect();
    let mut out = ResolvedTargets {
        content,
        ..ResolvedTargets::default()
    };
    let mut seen_thread_ids: HashSet<String> = HashSet::new();
    let mut skips = Skips::default();

    for target in targets {
        match target {
            TargetResolution::Posts => {
                out.posts = true;
            }
            TargetResolution::AllDms => {
                let threads = client::list_all_threads(client, MAX_INBOX_THREADS).await?;
                if threads.len() >= MAX_INBOX_THREADS {
                    let _ = tx
                        .send(PrunerEvent::Log {
                            level: LogLevel::Warning,
                            message: format!(
                                "inbox walk capped at {MAX_INBOX_THREADS} threads, remaining not scanned"
                            ),
                        })
                        .await;
                }
                for t in threads {
                    if admit_thread(
                        &t,
                        dm_kind,
                        true,
                        &protect,
                        &mut seen_thread_ids,
                        &mut skips,
                    ) {
                        out.threads.push(t);
                    }
                }
            }
            TargetResolution::Threads(ids) => {
                for id in ids {
                    let id = id.trim();
                    if id.is_empty() || seen_thread_ids.contains(id) {
                        continue;
                    }
                    let _ = tx
                        .send(PrunerEvent::Log {
                            level: LogLevel::Info,
                            message: format!("loading thread {id}…"),
                        })
                        .await;
                    let thread = match client.direct_thread(id, 50, None).await {
                        Ok(t) => t,
                        Err(e) => {
                            let _ = tx
                                .send(PrunerEvent::Log {
                                    level: LogLevel::Warning,
                                    message: format!("thread {id}: {e}"),
                                })
                                .await;
                            continue;
                        }
                    };
                    if admit_thread(
                        &thread,
                        dm_kind,
                        false,
                        &protect,
                        &mut seen_thread_ids,
                        &mut skips,
                    ) {
                        out.threads.push(thread);
                    }
                }
            }
            TargetResolution::ThreadObjects(threads) => {
                for thread in threads {
                    if admit_thread(
                        thread,
                        dm_kind,
                        false,
                        &protect,
                        &mut seen_thread_ids,
                        &mut skips,
                    ) {
                        out.threads.push(thread.clone());
                    }
                }
            }
        }
    }

    if skips.kind > 0 {
        let _ = tx
            .send(PrunerEvent::Log {
                level: LogLevel::Info,
                message: format!(
                    "skipped {} thread(s) not matching dm_kind={}",
                    skips.kind,
                    dm_kind.label()
                ),
            })
            .await;
    }

    if skips.protected > 0 {
        skips.protected_names.sort();
        skips.protected_names.dedup();
        let _ = tx
            .send(PrunerEvent::Log {
                level: LogLevel::Warning,
                message: format!(
                    "skipped {} protected thread(s): {}",
                    skips.protected,
                    skips.protected_names.join(", ")
                ),
            })
            .await;
    }

    Ok(out)
}

pub(crate) fn target_count(resolved: &ResolvedTargets) -> usize {
    resolved.threads.len() + usize::from(resolved.posts) + resolved.content.count()
}
