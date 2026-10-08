use super::super::action;

#[derive(Debug, Clone)]
pub struct PaletteState {
    pub query: String,
    pub matches: Vec<usize>,
    pub cursor: usize,
}

impl Default for PaletteState {
    fn default() -> Self {
        Self {
            query: String::new(),
            matches: (0..action::commands().len()).collect(),
            cursor: 0,
        }
    }
}

impl PaletteState {
    pub fn update_matches(&mut self) {
        let q = self.query.trim().to_ascii_lowercase();
        self.matches.clear();
        for (i, cmd) in action::commands().iter().enumerate() {
            if q.is_empty()
                || cmd.label.to_ascii_lowercase().contains(&q)
                || cmd.hint.to_ascii_lowercase().contains(&q)
                || cmd.id.to_ascii_lowercase().contains(&q)
            {
                self.matches.push(i);
            }
        }
        self.cursor = self.cursor.min(self.matches.len().saturating_sub(1));
    }
}
