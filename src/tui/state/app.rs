use std::time::Instant;

use crate::config::{Config, ScopeKind};
use crate::events::{LogLevel, Phase, RunStats};
use crate::history::HistoryStore;
use crate::model::DirectThread;
use crate::pruner::TargetResolution;

use super::super::action::{Effect, Overlay, Screen};
use super::super::keys::InputMode;
use super::{
    ArchiveState, Connection, Form, PaletteState, RunState, RunStatus, SettingsState, TargetsState,
    Toast,
};

#[derive(Debug)]
pub struct AppState {
    pub screen: Screen,
    pub overlay: Overlay,
    pub connection: Connection,
    pub identity: Option<String>,
    pub toast: Option<Toast>,
    pub form: Form,
    pub settings: SettingsState,
    pub targets: TargetsState,
    pub run: RunState,
    pub archive: ArchiveState,
    pub palette: PaletteState,
    pub config: Config,
}

impl AppState {
    pub fn new(cfg: Config) -> Self {
        let form = Form::from_config(&cfg);
        let mut archive = ArchiveState {
            store: HistoryStore::with_root(&cfg.history_dir),
            ..ArchiveState::default()
        };
        archive.reload();

        let mut targets = TargetsState::default();
        for u in &cfg.protect_users {
            targets.protected_users.insert(u.clone());
        }

        Self {
            screen: Screen::Settings,
            overlay: Overlay::None,
            connection: Connection::Idle,
            identity: None,
            toast: None,
            form,
            settings: SettingsState::default(),
            targets,
            run: RunState::default(),
            archive,
            palette: PaletteState::default(),
            config: cfg,
        }
    }

    pub fn input_mode(&self) -> InputMode {
        match self.overlay {
            Overlay::Help => InputMode::Help,
            Overlay::Palette => InputMode::Palette,
            Overlay::Confirm => InputMode::Confirm,
            Overlay::None => {
                if self.settings.editing {
                    InputMode::Text
                } else {
                    InputMode::Normal
                }
            }
        }
    }

    pub fn set_toast(&mut self, text: impl Into<String>, level: LogLevel) {
        self.toast = Some(Toast::new(text, level));
    }

    pub fn picked_threads_len(&self) -> usize {
        self.targets
            .threads
            .iter()
            .filter(|t| self.targets.selected_threads.contains(&t.thread_id))
            .count()
    }

    // picks only narrow the blanket scopes; typed thread ids always win
    pub fn has_picker_override(&self) -> bool {
        matches!(self.form.scope.kind, ScopeKind::Dms | ScopeKind::All)
            && self.picked_threads_len() > 0
    }

    pub(super) fn picked_threads(&self) -> Vec<DirectThread> {
        self.targets
            .threads
            .iter()
            .filter(|t| self.targets.selected_threads.contains(&t.thread_id))
            .cloned()
            .collect()
    }

    fn picker_override(&self) -> Option<Vec<DirectThread>> {
        if self.has_picker_override() {
            Some(self.picked_threads())
        } else {
            None
        }
    }

    pub(super) fn start_run_effects(&mut self) -> Vec<Effect> {
        match self.form.to_pruner_config() {
            Ok(mut pruner_cfg) => {
                self.run.log.clear();
                self.run.log_scroll = 0;
                if let Some(picked) = self.picker_override() {
                    let count = picked.len();
                    pruner_cfg.targets = vec![TargetResolution::ThreadObjects(picked)];
                    self.run.push_log(
                        LogLevel::Info,
                        format!(
                            "using {count} picked thread(s); scope {} ignored",
                            self.form.scope.kind.label()
                        ),
                    );
                }
                self.run.status = RunStatus::Running;
                self.run.phase = Phase::Scan;
                self.run.started_at = Some(Instant::now());
                self.run.finished_at = None;
                self.run.plan_total = 0;
                self.run.plan_done = 0;
                self.run.stats = RunStats::default();
                self.run.channels.clear();
                self.run.current_progress = None;
                self.run.log_scroll = 0;

                if pruner_cfg.content.any() {
                    self.run.push_log(
                        LogLevel::Info,
                        format!("content classes: {}", pruner_cfg.content.label()),
                    );
                }
                self.screen = Screen::Run;
                vec![Effect::StartRun(pruner_cfg)]
            }
            Err(e) => {
                self.set_toast(format!("config error: {e}"), LogLevel::Error);
                Vec::new()
            }
        }
    }
}
