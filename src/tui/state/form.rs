use std::collections::HashMap;

use crate::config::{thread_id_list, Config, ContentFlags, DmKind, ScopeForm, ScopeKind};
use crate::pruner::{dates, PrunerConfig, TargetResolution};

use super::super::action::Field;

#[derive(Debug, Clone)]
pub struct Form {
    pub sessionid: String,
    pub scope: ScopeForm,
    pub before: String,
    pub after: String,
    pub limit: String,
    pub throttle_ms: String,
    pub scan_throttle_ms: String,
    pub concurrency: String,
    pub dm_kind: DmKind,
    pub dry_run: bool,
    pub hide_threads: bool,
    pub content: ContentFlags,
    pub protect_users: String,
}

impl Form {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            sessionid: cfg.sessionid.clone(),
            scope: cfg.scope_form.clone(),
            before: cfg.before.clone(),
            after: cfg.after.clone(),
            limit: cfg.limit.clone(),
            throttle_ms: cfg.throttle_ms.clone(),
            scan_throttle_ms: cfg.scan_throttle_ms.clone(),
            concurrency: cfg.concurrency.clone(),
            dm_kind: cfg.dm_kind,
            dry_run: cfg.dry_run,
            hide_threads: cfg.hide_threads,
            content: cfg.content,
            protect_users: cfg.protect_users.join(", "),
        }
    }

    pub fn get_text(&self, field: Field) -> String {
        match field {
            Field::SessionId => self.sessionid.clone(),
            Field::Payload => self.scope.threads.clone(),
            Field::Before => self.before.clone(),
            Field::After => self.after.clone(),
            Field::Limit => self.limit.clone(),
            Field::Throttle => self.throttle_ms.clone(),
            Field::ScanThrottle => self.scan_throttle_ms.clone(),
            Field::Concurrency => self.concurrency.clone(),
            Field::Protect => self.protect_users.clone(),
            _ => String::new(),
        }
    }

    pub fn set_text(&mut self, field: Field, value: String) {
        match field {
            Field::SessionId => self.sessionid = value.trim().to_string(),
            Field::Payload => self.scope.threads = value,
            Field::Before => self.before = value.trim().to_string(),
            Field::After => self.after = value.trim().to_string(),
            Field::Limit => self.limit = value.trim().to_string(),
            Field::Throttle => self.throttle_ms = value.trim().to_string(),
            Field::ScanThrottle => self.scan_throttle_ms = value.trim().to_string(),
            Field::Concurrency => self.concurrency = value.trim().to_string(),
            Field::Protect => self.protect_users = value,
            _ => {}
        }
    }

    pub fn toggle(&mut self, field: Field) {
        self.toggle_flag(field);
    }

    pub fn cycle_next(&mut self, field: Field) {
        self.cycle(field, true);
    }

    pub fn cycle_prev(&mut self, field: Field) {
        self.cycle(field, false);
    }

    fn cycle(&mut self, field: Field, forward: bool) {
        match field {
            Field::Scope => {
                if forward {
                    self.scope.next();
                } else {
                    self.scope.prev();
                }
            }
            Field::DmKind => {
                self.dm_kind = if forward {
                    self.dm_kind.next()
                } else {
                    self.dm_kind.prev()
                };
            }
            _ => {
                self.toggle_flag(field);
            }
        }
    }

    // dry-run, hide-threads and the five content classes all behave as plain switches
    fn toggle_flag(&mut self, field: Field) {
        match field {
            Field::DryRun => self.dry_run = !self.dry_run,
            Field::HideThreads => self.hide_threads = !self.hide_threads,
            Field::ContentLikes => self.content.likes = !self.content.likes,
            Field::ContentSaved => self.content.saved = !self.content.saved,
            Field::ContentComments => self.content.comments = !self.content.comments,
            Field::ContentArchived => self.content.archived = !self.content.archived,
            Field::ContentReposts => self.content.reposts = !self.content.reposts,
            _ => {}
        }
    }

    pub fn validate(&self) -> HashMap<Field, String> {
        let mut errs = HashMap::new();
        if self.sessionid.trim().is_empty() {
            errs.insert(Field::SessionId, "sessionid cookie is required".to_string());
        }
        if self.scope.kind == ScopeKind::Threads && thread_id_list(&self.scope.threads).is_empty() {
            errs.insert(
                Field::Payload,
                "at least one thread ID is required".to_string(),
            );
        }
        if self.scope.kind == ScopeKind::None && !self.content.any() {
            errs.insert(
                Field::Scope,
                "scope 'none' needs at least one content class enabled".to_string(),
            );
        }
        if !self.before.trim().is_empty() && dates::validate_date_input(&self.before).is_err() {
            errs.insert(
                Field::Before,
                "invalid date: expected YYYY-MM-DD or RFC3339".to_string(),
            );
        }
        if !self.after.trim().is_empty() && dates::validate_date_input(&self.after).is_err() {
            errs.insert(
                Field::After,
                "invalid date: expected YYYY-MM-DD or RFC3339".to_string(),
            );
        }
        if self.limit.trim().parse::<u64>().is_err() {
            errs.insert(Field::Limit, "must be a non-negative integer".to_string());
        }
        if let Ok(throttle) = self.throttle_ms.trim().parse::<u64>() {
            if throttle < 200 {
                errs.insert(Field::Throttle, "must be at least 200ms".to_string());
            }
        } else {
            errs.insert(Field::Throttle, "must be a positive integer".to_string());
        }
        if let Ok(scan) = self.scan_throttle_ms.trim().parse::<u64>() {
            if scan < 100 {
                errs.insert(Field::ScanThrottle, "must be at least 100ms".to_string());
            }
        } else {
            errs.insert(
                Field::ScanThrottle,
                "must be a positive integer".to_string(),
            );
        }
        match self.concurrency.trim().parse::<u64>() {
            Ok(n) if (1..=u64::from(crate::config::MAX_SCAN_CONCURRENCY)).contains(&n) => {}
            Ok(_) => {
                errs.insert(
                    Field::Concurrency,
                    format!("must be 1-{}", crate::config::MAX_SCAN_CONCURRENCY),
                );
            }
            Err(_) => {
                errs.insert(Field::Concurrency, "must be a positive integer".to_string());
            }
        }
        errs
    }

    pub fn to_pruner_config(&self) -> Result<PrunerConfig, String> {
        let targets = match self.scope.kind {
            ScopeKind::None => Vec::new(),
            ScopeKind::Posts => vec![TargetResolution::Posts],
            ScopeKind::Dms => vec![TargetResolution::AllDms],
            ScopeKind::All => vec![TargetResolution::Posts, TargetResolution::AllDms],
            ScopeKind::Threads => {
                let ids = thread_id_list(&self.scope.threads);
                if ids.is_empty() {
                    return Err("threads scope requires at least one thread ID".to_string());
                }
                vec![TargetResolution::Threads(ids)]
            }
        };

        let before = if self.before.trim().is_empty() {
            None
        } else {
            Some(self.before.trim().to_string())
        };
        let after = if self.after.trim().is_empty() {
            None
        } else {
            Some(self.after.trim().to_string())
        };

        let limit = self
            .limit
            .trim()
            .parse::<usize>()
            .map_err(|_| "invalid limit".to_string())?;
        let throttle_ms = self
            .throttle_ms
            .trim()
            .parse::<u64>()
            .map_err(|_| "invalid throttle".to_string())?;
        let scan_throttle_ms = self
            .scan_throttle_ms
            .trim()
            .parse::<u64>()
            .map_err(|_| "invalid scan throttle".to_string())?;
        let concurrency = crate::config::clamp_scan_concurrency(
            self.concurrency
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid scan concurrency".to_string())?,
        );
        let protect_users = thread_id_list(&self.protect_users);

        Ok(PrunerConfig {
            targets,
            content: self.content,
            before,
            after,
            limit,
            dry_run: self.dry_run,
            throttle_ms,
            scan_throttle_ms,
            concurrency,
            hide_threads: self.hide_threads,
            protect_users,
            dm_kind: self.dm_kind,
        })
    }
}
