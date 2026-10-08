use std::collections::HashMap;

use crate::config::ScopeKind;

use super::super::action::Field;

#[derive(Debug, Clone, Default)]
pub struct SettingsState {
    pub cursor: usize,
    pub editing: bool,
    pub buffer: String,
    pub buffer_cursor: usize,
    pub errors: HashMap<Field, String>,
}

impl SettingsState {
    pub fn field(&self) -> Field {
        Field::ALL[self.cursor.min(Field::ALL.len() - 1)]
    }

    pub fn move_visible(&mut self, scope: ScopeKind, forward: bool) {
        let visible: Vec<usize> = Field::ALL
            .iter()
            .enumerate()
            .filter(|(_, field)| **field != Field::Payload || scope.has_payload())
            .map(|(index, _)| index)
            .collect();
        let current = visible
            .iter()
            .position(|index| *index == self.cursor)
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % visible.len()
        } else {
            (current + visible.len() - 1) % visible.len()
        };
        self.cursor = visible[next];
    }

    pub fn start_edit(&mut self, current_value: &str) {
        self.editing = true;
        self.buffer = current_value.to_string();
        self.buffer_cursor = self.buffer.chars().count();
    }

    pub fn commit_edit(&mut self) -> String {
        self.editing = false;
        std::mem::take(&mut self.buffer)
    }

    pub fn cancel_edit(&mut self) {
        self.editing = false;
        self.buffer.clear();
        self.buffer_cursor = 0;
    }

    pub fn input(&mut self, c: char) {
        let mut chars: Vec<char> = self.buffer.chars().collect();
        let idx = self.buffer_cursor.min(chars.len());
        chars.insert(idx, c);
        self.buffer = chars.into_iter().collect();
        self.buffer_cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.buffer_cursor > 0 {
            let mut chars: Vec<char> = self.buffer.chars().collect();
            chars.remove(self.buffer_cursor - 1);
            self.buffer = chars.into_iter().collect();
            self.buffer_cursor -= 1;
        }
    }

    pub fn delete(&mut self) {
        let mut chars: Vec<char> = self.buffer.chars().collect();
        if self.buffer_cursor < chars.len() {
            chars.remove(self.buffer_cursor);
            self.buffer = chars.into_iter().collect();
        }
    }
}
