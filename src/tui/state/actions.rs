use std::collections::HashSet;

use crate::config::ScopeKind;
use crate::events::LogLevel;

use super::super::action::{self, Action, Effect, Field, Overlay, Screen};
use super::{AppState, ArchiveFocus, Connection};

impl AppState {
    pub fn apply_action(&mut self, action: Action) -> Vec<Effect> {
        match action {
            Action::Quit => vec![Effect::Quit],
            Action::Help => {
                self.overlay = if self.overlay == Overlay::Help {
                    Overlay::None
                } else {
                    Overlay::Help
                };
                Vec::new()
            }
            Action::Palette => {
                self.overlay = if self.overlay == Overlay::Palette {
                    Overlay::None
                } else {
                    self.palette.query.clear();
                    self.palette.update_matches();
                    self.palette.cursor = 0;
                    Overlay::Palette
                };
                Vec::new()
            }
            Action::PaletteDown => {
                if !self.palette.matches.is_empty() {
                    self.palette.cursor = (self.palette.cursor + 1) % self.palette.matches.len();
                }
                Vec::new()
            }
            Action::PaletteUp => {
                if !self.palette.matches.is_empty() {
                    self.palette.cursor = (self.palette.cursor + self.palette.matches.len() - 1)
                        % self.palette.matches.len();
                }
                Vec::new()
            }
            Action::PaletteSelect => {
                if let Some(&cmd_idx) = self.palette.matches.get(self.palette.cursor) {
                    let cmd = action::commands()[cmd_idx].action.clone();
                    self.overlay = Overlay::None;
                    self.apply_action(cmd)
                } else {
                    self.overlay = Overlay::None;
                    Vec::new()
                }
            }
            Action::Back => {
                if self.overlay != Overlay::None {
                    self.overlay = Overlay::None;
                    Vec::new()
                } else if self.settings.editing {
                    self.settings.cancel_edit();
                    Vec::new()
                } else if self.run.is_running() {
                    vec![Effect::CancelRun]
                } else {
                    Vec::new()
                }
            }
            Action::PrevScreen => {
                self.screen = self.screen.prev();
                Vec::new()
            }
            Action::NextScreen => {
                self.screen = self.screen.next();
                Vec::new()
            }
            Action::Goto(screen) => {
                self.screen = screen;
                Vec::new()
            }
            Action::NextField => {
                match self.screen {
                    Screen::Settings => self.settings.move_visible(self.form.scope.kind, true),
                    Screen::Archive => self.archive.focus = self.archive.focus.next(),
                    Screen::Targets | Screen::Run => {}
                }
                Vec::new()
            }
            Action::PrevField => {
                match self.screen {
                    Screen::Settings => self.settings.move_visible(self.form.scope.kind, false),
                    Screen::Archive => self.archive.focus = self.archive.focus.prev(),
                    Screen::Targets | Screen::Run => {}
                }
                Vec::new()
            }
            Action::Down => {
                match self.screen {
                    Screen::Settings => self.settings.move_visible(self.form.scope.kind, true),
                    Screen::Targets => {
                        let total = self.targets.filtered_threads(self.form.dm_kind).count();
                        if total > 0 {
                            self.targets.thread_cursor =
                                (self.targets.thread_cursor + 1).min(total - 1);
                        }
                    }
                    Screen::Run => {
                        self.run.log_scroll = self.run.log_scroll.saturating_sub(1);
                    }
                    Screen::Archive => match self.archive.focus {
                        ArchiveFocus::Days => {
                            if !self.archive.days.is_empty() {
                                self.archive.day_cursor =
                                    (self.archive.day_cursor + 1).min(self.archive.days.len() - 1);
                                self.archive.reload_channels();
                            }
                        }
                        ArchiveFocus::Channels => {
                            if !self.archive.channels.is_empty() {
                                self.archive.channel_cursor = (self.archive.channel_cursor + 1)
                                    .min(self.archive.channels.len() - 1);
                                self.archive.reload_records();
                            }
                        }
                        ArchiveFocus::Records => {
                            if !self.archive.records.is_empty() {
                                self.archive.record_cursor = (self.archive.record_cursor + 1)
                                    .min(self.archive.records.len() - 1);
                            }
                        }
                    },
                }
                Vec::new()
            }
            Action::Up => {
                match self.screen {
                    Screen::Settings => self.settings.move_visible(self.form.scope.kind, false),
                    Screen::Targets => {
                        self.targets.thread_cursor = self.targets.thread_cursor.saturating_sub(1);
                    }
                    Screen::Run => {
                        self.run.log_scroll = self.run.log_scroll.saturating_add(1);
                    }
                    Screen::Archive => match self.archive.focus {
                        ArchiveFocus::Days => {
                            self.archive.day_cursor = self.archive.day_cursor.saturating_sub(1);
                            self.archive.reload_channels();
                        }
                        ArchiveFocus::Channels => {
                            self.archive.channel_cursor =
                                self.archive.channel_cursor.saturating_sub(1);
                            self.archive.reload_records();
                        }
                        ArchiveFocus::Records => {
                            self.archive.record_cursor =
                                self.archive.record_cursor.saturating_sub(1);
                        }
                    },
                }
                Vec::new()
            }
            Action::CycleNext => {
                if self.screen == Screen::Settings {
                    self.cycle_focused(true);
                } else if self.screen == Screen::Archive {
                    self.archive.cycle_scope();
                }
                Vec::new()
            }
            Action::CyclePrev => {
                if self.screen == Screen::Settings {
                    self.cycle_focused(false);
                } else if self.screen == Screen::Archive {
                    self.archive.cycle_scope();
                }
                Vec::new()
            }
            Action::Toggle => {
                if self.screen == Screen::Settings {
                    let field = self.settings.field();
                    self.form.toggle(field);
                } else if self.screen == Screen::Targets {
                    if let Some(id) = self.focused_thread_id() {
                        if self.targets.selected_threads.contains(&id) {
                            self.targets.selected_threads.remove(&id);
                        } else {
                            self.targets.selected_threads.insert(id);
                        }
                    }
                }
                Vec::new()
            }
            Action::ToggleProtect => {
                if self.screen == Screen::Targets {
                    if let Some(id) = self.focused_thread_id() {
                        if self.targets.protected_users.contains(&id) {
                            self.targets.protected_users.remove(&id);
                            self.set_toast("unprotected thread", LogLevel::Info);
                        } else {
                            self.targets.protected_users.insert(id);
                            self.set_toast("protected thread", LogLevel::Success);
                        }
                        self.form.protect_users = self
                            .targets
                            .protected_users
                            .iter()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ");
                    }
                }
                Vec::new()
            }
            Action::ToggleDryRun => {
                self.form.dry_run = !self.form.dry_run;
                let state = if self.form.dry_run { "ON" } else { "OFF" };
                self.set_toast(format!("dry run {state}"), LogLevel::Info);
                Vec::new()
            }
            Action::StartEdit => {
                if self.screen == Screen::Settings {
                    let field = self.settings.field();
                    if field.is_text() {
                        let val = self.form.get_text(field);
                        self.settings.start_edit(&val);
                    }
                }
                Vec::new()
            }
            Action::CommitEdit => {
                if self.settings.editing {
                    let field = self.settings.field();
                    let val = self.settings.commit_edit();
                    self.form.set_text(field, val);
                    self.settings.errors = self.form.validate();
                }
                Vec::new()
            }
            Action::CancelEdit => {
                self.settings.cancel_edit();
                Vec::new()
            }
            Action::Input(c) => {
                if self.overlay == Overlay::Palette {
                    self.palette.query.push(c);
                    self.palette.update_matches();
                } else if self.settings.editing {
                    self.settings.input(c);
                }
                Vec::new()
            }
            Action::InputBackspace => {
                if self.overlay == Overlay::Palette {
                    self.palette.query.pop();
                    self.palette.update_matches();
                } else if self.settings.editing {
                    self.settings.backspace();
                }
                Vec::new()
            }
            Action::InputDelete => {
                if self.settings.editing {
                    self.settings.delete();
                }
                Vec::new()
            }
            Action::InputLeft => {
                if self.settings.editing && self.settings.buffer_cursor > 0 {
                    self.settings.buffer_cursor -= 1;
                }
                Vec::new()
            }
            Action::InputRight => {
                if self.settings.editing
                    && self.settings.buffer_cursor < self.settings.buffer.chars().count()
                {
                    self.settings.buffer_cursor += 1;
                }
                Vec::new()
            }
            Action::InputHome => {
                if self.settings.editing {
                    self.settings.buffer_cursor = 0;
                }
                Vec::new()
            }
            Action::InputEnd => {
                if self.settings.editing {
                    self.settings.buffer_cursor = self.settings.buffer.chars().count();
                }
                Vec::new()
            }
            Action::InputClear => {
                if self.overlay == Overlay::Palette {
                    self.palette.query.clear();
                    self.palette.update_matches();
                } else if self.settings.editing {
                    self.settings.buffer.clear();
                    self.settings.buffer_cursor = 0;
                }
                Vec::new()
            }
            Action::SelectAll => {
                if self.screen == Screen::Targets {
                    let ids: Vec<String> = self
                        .targets
                        .filtered_threads(self.form.dm_kind)
                        .map(|t| t.thread_id.clone())
                        .collect();
                    for id in ids {
                        self.targets.selected_threads.insert(id);
                    }
                }
                Vec::new()
            }
            Action::SelectNone => {
                if self.screen == Screen::Targets {
                    self.targets.selected_threads.clear();
                }
                Vec::new()
            }
            Action::SelectInvert => {
                if self.screen == Screen::Targets {
                    let all_ids: Vec<String> = self
                        .targets
                        .filtered_threads(self.form.dm_kind)
                        .map(|t| t.thread_id.clone())
                        .collect();
                    let mut inverted = HashSet::new();
                    for id in all_ids {
                        if !self.targets.selected_threads.contains(&id) {
                            inverted.insert(id);
                        }
                    }
                    self.targets.selected_threads = inverted;
                }
                Vec::new()
            }
            Action::Activate => {
                if self.screen == Screen::Targets {
                    let picked = self.picked_threads();
                    if !picked.is_empty() {
                        let count = picked.len();
                        let ids: Vec<String> = picked.iter().map(|t| t.thread_id.clone()).collect();
                        self.form.scope.kind = ScopeKind::Threads;
                        self.form.scope.threads = ids.join(", ");
                        self.set_toast(
                            format!("copied {count} picked thread(s) into the threads scope"),
                            LogLevel::Success,
                        );
                        self.screen = Screen::Settings;
                    }
                    Vec::new()
                } else if self.screen == Screen::Settings || self.screen == Screen::Run {
                    if self.run.is_running() {
                        self.set_toast("a prune run is already active", LogLevel::Warning);
                        return Vec::new();
                    }
                    self.settings.errors = self.form.validate();
                    if !self.settings.errors.is_empty() {
                        self.set_toast("fix validation errors before running", LogLevel::Error);
                        self.screen = Screen::Settings;
                        return Vec::new();
                    }
                    if !self.form.dry_run && self.overlay != Overlay::Confirm {
                        self.overlay = Overlay::Confirm;
                        return Vec::new();
                    }
                    self.start_run_effects()
                } else {
                    Vec::new()
                }
            }
            Action::Confirm => {
                self.overlay = Overlay::None;
                self.start_run_effects()
            }
            Action::Decline => {
                self.overlay = Overlay::None;
                Vec::new()
            }
            Action::Probe => {
                self.connection = Connection::Connecting;
                self.targets.loading = true;
                vec![Effect::Probe]
            }
            Action::Refresh => {
                if self.screen == Screen::Archive {
                    self.archive.reload();
                    self.set_toast("archive reloaded", LogLevel::Info);
                    Vec::new()
                } else {
                    self.targets.loading = true;
                    vec![Effect::LoadTargets]
                }
            }
            Action::Yank => {
                if self.screen == Screen::Archive {
                    if let Some(record) = self.archive.records.get(self.archive.record_cursor) {
                        let json = serde_json::to_string_pretty(record).unwrap_or_default();
                        self.set_toast("copied record to clipboard", LogLevel::Success);
                        return vec![Effect::Yank(json)];
                    }
                }
                Vec::new()
            }
            Action::ClearLog => {
                self.run.log.clear();
                self.run.log_scroll = 0;
                Vec::new()
            }
        }
    }

    fn focused_thread_id(&self) -> Option<String> {
        self.targets
            .filtered_threads(self.form.dm_kind)
            .nth(self.targets.thread_cursor)
            .map(|t| t.thread_id.clone())
    }

    fn cycle_focused(&mut self, forward: bool) {
        let field = self.settings.field();
        if forward {
            self.form.cycle_next(field);
        } else {
            self.form.cycle_prev(field);
        }
        if field == Field::Scope
            && !self.form.scope.kind.has_payload()
            && self.settings.field() == Field::Payload
        {
            self.settings.cursor = Field::ALL
                .iter()
                .position(|field| *field == Field::Scope)
                .unwrap_or(0);
        }
    }
}
