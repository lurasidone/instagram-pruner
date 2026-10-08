use std::collections::HashSet;

use crate::config::DmKind;
use crate::model::DirectThread;

#[derive(Debug, Clone, Default)]
pub struct TargetsState {
    pub threads: Vec<DirectThread>,
    pub selected_threads: HashSet<String>,
    pub protected_users: HashSet<String>,
    pub thread_cursor: usize,
    pub loading: bool,
}

impl TargetsState {
    pub fn filtered_threads(&self, kind: DmKind) -> impl Iterator<Item = &DirectThread> {
        self.threads
            .iter()
            .filter(move |t| kind.matches(t.is_group))
    }

    pub fn is_protected(&self, thread: &DirectThread) -> bool {
        if self.protected_users.contains(&thread.thread_id) {
            return true;
        }
        thread.users.iter().any(|u| {
            self.protected_users.contains(&u.pk.to_string())
                || self.protected_users.contains(&u.username)
        })
    }
}
