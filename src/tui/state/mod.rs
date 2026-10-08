use std::time::{Duration, Instant};

use crate::events::LogLevel;

pub mod actions;
pub mod app;
pub mod archive;
pub mod events;
pub mod form;
pub mod palette;
pub mod run;
pub mod settings;
pub mod targets;
#[cfg(test)]
mod tests;

pub use app::AppState;
pub use archive::{ArchiveFocus, ArchiveState};
pub use form::Form;
pub use palette::PaletteState;
pub use run::{ChannelProgress, RunState, RunStatus};
pub use settings::SettingsState;
pub use targets::TargetsState;

const LOG_CAP: usize = 1200;
const SPARKLINE_CAP: usize = 60;
// toasts linger long enough to read after the action that raised them
const TOAST_TTL: Duration = Duration::from_secs(6);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Connection {
    #[default]
    Idle,
    Connecting,
    Ready,
    Failed,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub text: String,
    pub level: LogLevel,
    pub expires_at: Instant,
}

impl Toast {
    pub fn new(text: impl Into<String>, level: LogLevel) -> Self {
        Self {
            text: text.into(),
            level,
            expires_at: Instant::now() + TOAST_TTL,
        }
    }

    pub fn is_expired(&self, now: Instant) -> bool {
        now >= self.expires_at
    }
}

pub fn mask_sessionid(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return "(empty)".to_string();
    }
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= 6 {
        return "••••••".to_string();
    }
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("••••••••••••{tail}")
}
