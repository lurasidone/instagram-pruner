#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Scan,
    Delete,
}

#[derive(Debug, Clone, Copy)]
pub enum LogLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ChannelStats {
    pub scanned: u64,
    pub deleted: u64,
    pub skipped: u64,
    pub failed: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RunStats {
    pub channel_count: usize,
    pub channels_processed: usize,
    pub messages_scanned: u64,
    pub messages_deleted: u64,
    pub messages_skipped: u64,
    pub messages_failed: u64,
    pub rate_limit_hits: u64,
    pub dm_messages_deleted: u64,
    pub plan_total: usize,
    pub was_cancelled: bool,
}

impl RunStats {
    pub fn aggregate(&mut self, cs: ChannelStats) {
        self.messages_scanned += cs.scanned;
        self.messages_deleted += cs.deleted;
        self.messages_skipped += cs.skipped;
        self.messages_failed += cs.failed;
    }
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub level: LogLevel,
    pub message: String,
    pub at: std::time::SystemTime,
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum PrunerEvent {
    Started {
        channel_count: usize,
    },
    ChannelStarted {
        channel_id: String,
        channel_name: String,
        index: usize,
        total: usize,
        phase: Phase,
    },
    MessageDeleted {
        preview: String,
        outcome: crate::history::DeleteOutcome,
    },
    MessageFailed {
        message_id: String,
        error: String,
    },
    ChannelDone {
        channel_id: String,
        stats: ChannelStats,
        phase: Phase,
    },
    PlanReady {
        total: usize,
    },
    RateLimited {
        retry_after_ms: u64,
    },
    Log {
        level: LogLevel,
        message: String,
    },
    Done {
        stats: RunStats,
    },
    Cancelled,
    Error {
        message: String,
    },
    DMsLoaded {
        channels: Vec<crate::model::DirectThread>,
    },
    DMsLoadFailed {
        message: String,
    },
    Identity {
        username: String,
    },
    ProbeFailed {
        message: String,
    },
}
