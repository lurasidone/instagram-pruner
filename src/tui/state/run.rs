use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::events::{ChannelStats, LogEntry, LogLevel, Phase, RunStats};

use super::{LOG_CAP, SPARKLINE_CAP};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChannelProgress {
    pub channel_id: String,
    pub channel_name: String,
    pub deleted: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RateLimiterView {
    pub hits_429: u64,
    pub cooldown_remaining_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RunStatus {
    #[default]
    Idle,
    Running,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone)]
pub struct RunState {
    pub status: RunStatus,
    pub phase: Phase,
    pub started_at: Option<Instant>,
    pub finished_at: Option<Instant>,
    pub channels: Vec<(String, String, ChannelStats)>,
    pub current_progress: Option<ChannelProgress>,
    pub stats: RunStats,
    pub plan_total: usize,
    pub plan_done: usize,
    pub log: VecDeque<LogEntry>,
    pub log_scroll: usize,
    pub rate_limiter: RateLimiterView,
    pub sparkline_data: VecDeque<u64>,
}

impl Default for RunState {
    fn default() -> Self {
        Self {
            status: RunStatus::Idle,
            phase: Phase::Scan,
            started_at: None,
            finished_at: None,
            channels: Vec::new(),
            current_progress: None,
            stats: RunStats::default(),
            plan_total: 0,
            plan_done: 0,
            log: VecDeque::new(),
            log_scroll: 0,
            rate_limiter: RateLimiterView::default(),
            sparkline_data: VecDeque::new(),
        }
    }
}

impl RunState {
    pub fn is_running(&self) -> bool {
        self.status == RunStatus::Running
    }

    pub fn elapsed(&self) -> Duration {
        match (self.started_at, self.finished_at) {
            (Some(s), Some(f)) => f.duration_since(s),
            (Some(s), None) => Instant::now().duration_since(s),
            _ => Duration::ZERO,
        }
    }

    pub fn push_log(&mut self, level: LogLevel, message: impl Into<String>) {
        if self.log.len() >= LOG_CAP {
            self.log.pop_front();
        }
        self.log.push_back(LogEntry {
            level,
            message: message.into(),
            at: std::time::SystemTime::now(),
        });
    }

    pub fn push_rate(&mut self, ms: u64) {
        if self.sparkline_data.len() >= SPARKLINE_CAP {
            self.sparkline_data.pop_front();
        }
        self.sparkline_data.push_back(ms);
    }
}
