use crate::events::{LogLevel, Phase, PrunerEvent};

use super::{AppState, ChannelProgress, Connection, RunStatus};

impl AppState {
    pub fn apply_event(&mut self, event: PrunerEvent) {
        match event {
            PrunerEvent::Started { channel_count } => {
                self.run.stats.channel_count = channel_count;
                self.run.push_log(
                    LogLevel::Info,
                    format!("started scan of {channel_count} target(s)"),
                );
            }
            PrunerEvent::ChannelStarted {
                channel_id,
                channel_name,
                index,
                total,
                phase,
            } => {
                self.run.phase = phase;
                self.run.current_progress = Some(ChannelProgress {
                    channel_id: channel_id.clone(),
                    channel_name: channel_name.clone(),
                    deleted: 0,
                });
                self.run.push_log(
                    LogLevel::Info,
                    format!("[{index}/{total}] starting {channel_name}"),
                );
            }
            PrunerEvent::MessageDeleted { preview, outcome } => {
                self.run.plan_done += 1;
                self.run.stats.messages_deleted += 1;
                if let Some(p) = &mut self.run.current_progress {
                    p.deleted += 1;
                }
                self.run.push_log(
                    LogLevel::Success,
                    format!("{}: {preview}", outcome.log_prefix()),
                );
            }
            PrunerEvent::MessageFailed { message_id, error } => {
                self.run.stats.messages_failed += 1;
                self.run
                    .push_log(LogLevel::Error, format!("failed {message_id}: {error}"));
            }
            PrunerEvent::ChannelDone {
                channel_id,
                stats,
                phase,
            } => {
                self.run.stats.aggregate(stats);
                let name = self
                    .run
                    .current_progress
                    .as_ref()
                    .map(|p| p.channel_name.clone())
                    .unwrap_or_else(|| channel_id.clone());
                self.run.channels.push((channel_id, name.clone(), stats));
                self.run.push_log(
                    LogLevel::Info,
                    format!("finished {name} ({phase:?}): deleted {}", stats.deleted),
                );
            }
            PrunerEvent::PlanReady { total } => {
                self.run.plan_total = total;
                self.run.phase = Phase::Delete;
                self.run.push_log(
                    LogLevel::Success,
                    format!("plan ready: {total} items to prune"),
                );
            }
            PrunerEvent::RateLimited { retry_after_ms } => {
                self.run.stats.rate_limit_hits += 1;
                self.run.rate_limiter.hits_429 += 1;
                self.run.rate_limiter.cooldown_remaining_ms = retry_after_ms;
                self.run.push_rate(retry_after_ms);
                self.run.push_log(
                    LogLevel::Warning,
                    format!("rate limited: backing off for {retry_after_ms}ms"),
                );
            }
            PrunerEvent::Log { level, message } => {
                self.run.push_log(level, message);
            }
            PrunerEvent::Done { stats } => {
                self.run.status = RunStatus::Completed;
                self.run.finished_at = Some(std::time::Instant::now());
                self.run.stats = stats;
                self.run.current_progress = None;
                let elapsed = self.run.elapsed();
                self.run.push_log(
                    LogLevel::Success,
                    format!(
                        "run complete: {} deleted, {} failed in {elapsed:?}",
                        stats.messages_deleted, stats.messages_failed
                    ),
                );
                self.archive.reload();
            }
            PrunerEvent::Cancelled => {
                self.run.status = RunStatus::Cancelled;
                self.run.finished_at = Some(std::time::Instant::now());
                self.run.current_progress = None;
                self.run.push_log(LogLevel::Warning, "run cancelled");
            }
            PrunerEvent::Error { message } => {
                self.run.status = RunStatus::Failed;
                self.run.finished_at = Some(std::time::Instant::now());
                self.run
                    .push_log(LogLevel::Error, format!("error: {message}"));
            }
            PrunerEvent::Identity { username } => {
                self.connection = Connection::Ready;
                self.identity = Some(username);
            }
            PrunerEvent::DMsLoaded { channels } => {
                self.targets.loading = false;
                let before = self.targets.selected_threads.len();
                self.targets
                    .selected_threads
                    .retain(|id| channels.iter().any(|t| &t.thread_id == id));
                let dropped = before - self.targets.selected_threads.len();
                self.targets.threads = channels;
                if dropped > 0 {
                    self.set_toast(
                        format!("{dropped} picked thread(s) left the inbox; deselected"),
                        LogLevel::Warning,
                    );
                }
            }
            PrunerEvent::DMsLoadFailed { message } => {
                self.targets.loading = false;
                self.set_toast(
                    format!("failed to load threads: {message}"),
                    LogLevel::Error,
                );
            }
            PrunerEvent::ProbeFailed { message } => {
                self.connection = Connection::Failed;
                self.targets.loading = false;
                self.set_toast(format!("probe failed: {message}"), LogLevel::Error);
            }
        }
    }
}
