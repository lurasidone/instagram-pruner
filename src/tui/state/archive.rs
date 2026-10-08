use chrono::NaiveDate;

use crate::history::{ChannelRef, DeletedRecord, HistoryScope, HistoryStore};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArchiveFocus {
    #[default]
    Days,
    Channels,
    Records,
}

impl ArchiveFocus {
    const ALL: [ArchiveFocus; 3] = [
        ArchiveFocus::Days,
        ArchiveFocus::Channels,
        ArchiveFocus::Records,
    ];

    fn step(self, forward: bool) -> Self {
        let index = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        let len = Self::ALL.len();
        Self::ALL[if forward {
            (index + 1) % len
        } else {
            (index + len - 1) % len
        }]
    }

    pub fn next(self) -> Self {
        self.step(true)
    }

    pub fn prev(self) -> Self {
        self.step(false)
    }
}

#[derive(Debug, Clone)]
pub struct ArchiveState {
    pub store: HistoryStore,
    pub scope: HistoryScope,
    pub focus: ArchiveFocus,
    pub days: Vec<NaiveDate>,
    pub day_cursor: usize,
    pub channels: Vec<ChannelRef>,
    pub channel_cursor: usize,
    pub records: Vec<DeletedRecord>,
    pub record_cursor: usize,
}

impl Default for ArchiveState {
    fn default() -> Self {
        let store = HistoryStore::with_root("./history");
        let days = store.list_days(HistoryScope::All).unwrap_or_default();
        let channels = days
            .first()
            .and_then(|d| store.list_channels(*d, HistoryScope::All).ok())
            .unwrap_or_default();
        let records = match (days.first(), channels.first()) {
            (Some(d), Some(c)) => store
                .load_day(*d, HistoryScope::All, Some(&c.id))
                .unwrap_or_default(),
            (Some(d), None) => store
                .load_day(*d, HistoryScope::All, None)
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        Self {
            store,
            scope: HistoryScope::All,
            focus: ArchiveFocus::Days,
            days,
            day_cursor: 0,
            channels,
            channel_cursor: 0,
            records,
            record_cursor: 0,
        }
    }
}

impl ArchiveState {
    pub fn reload(&mut self) {
        self.days = self.store.list_days(self.scope).unwrap_or_default();
        self.day_cursor = self.day_cursor.min(self.days.len().saturating_sub(1));
        self.reload_channels();
    }

    pub fn reload_channels(&mut self) {
        if let Some(&day) = self.days.get(self.day_cursor) {
            self.channels = self
                .store
                .list_channels(day, self.scope)
                .unwrap_or_default();
        } else {
            self.channels.clear();
        }
        self.channel_cursor = self
            .channel_cursor
            .min(self.channels.len().saturating_sub(1));
        self.reload_records();
    }

    pub fn reload_records(&mut self) {
        if let Some(&day) = self.days.get(self.day_cursor) {
            if let Some(ch) = self.channels.get(self.channel_cursor) {
                self.records = self
                    .store
                    .load_day(day, self.scope, Some(&ch.id))
                    .unwrap_or_default();
            } else {
                self.records = self
                    .store
                    .load_day(day, self.scope, None)
                    .unwrap_or_default();
            }
        } else {
            self.records.clear();
        }
        self.record_cursor = self.record_cursor.min(self.records.len().saturating_sub(1));
    }

    pub fn cycle_scope(&mut self) {
        self.scope = self.scope.next();
        self.reload();
    }
}
