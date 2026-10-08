use ratatui::layout::{Constraint, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use super::super::action::{self, Overlay};
use super::super::keys;
use super::super::state::AppState;
use super::super::theme;
use super::super::widgets::{centered_rect, panel, split_v};

pub fn draw(f: &mut Frame, app: &AppState, area: Rect) {
    match app.overlay {
        Overlay::None => {}
        Overlay::Palette => draw_palette(f, app, area),
        Overlay::Confirm => draw_confirm(f, app, area),
        Overlay::Help => draw_help(f, area),
    }
}

fn draw_palette(f: &mut Frame, app: &AppState, area: Rect) {
    let win = centered_rect(60, 45, area);
    f.render_widget(Clear, win);
    let block = panel(
        Line::from(vec![
            Span::styled(" Command palette ", theme::strong()),
            Span::styled("· type to filter · Enter run · Esc close ", theme::dim()),
        ]),
        true,
    );
    let inner = block.inner(win);
    f.render_widget(block, win);

    let prompt = Line::from(vec![
        Span::styled("> ", theme::accent().add_modifier(Modifier::BOLD)),
        Span::styled(app.palette.query.clone(), theme::strong()),
        Span::styled("▏", theme::accent().add_modifier(Modifier::SLOW_BLINK)),
    ]);
    let parts = split_v(inner, &[Constraint::Length(1), Constraint::Min(1)]);
    f.render_widget(Paragraph::new(prompt), parts[0]);

    let items: Vec<ListItem> = app
        .palette
        .matches
        .iter()
        .filter_map(|&i| action::commands().get(i))
        .map(|cmd| {
            let line = Line::from(vec![
                Span::styled(format!("{:<18}", cmd.label), theme::strong()),
                Span::styled(format!(" {}", cmd.hint), theme::dim()),
            ]);
            ListItem::new(line)
        })
        .collect();

    if items.is_empty() {
        let empty = Paragraph::new("No matching commands").style(theme::dim());
        f.render_widget(empty, parts[1]);
        return;
    }

    let mut state = ListState::default();
    state.select(Some(app.palette.cursor.min(items.len().saturating_sub(1))));
    f.render_stateful_widget(
        List::new(items)
            .style(theme::base())
            .highlight_style(theme::selected())
            .highlight_symbol("▶ "),
        parts[1],
        &mut state,
    );
}

fn draw_confirm(f: &mut Frame, app: &AppState, area: Rect) {
    let win = centered_rect(56, 32, area);
    f.render_widget(Clear, win);
    let block = panel(
        Span::styled(
            " Confirm Permanent Deletion ",
            theme::err().add_modifier(Modifier::BOLD),
        ),
        true,
    );
    let inner = block.inner(win);
    f.render_widget(block, win);

    let summary = app.form.scope.summary();
    let lines = vec![
        Line::from(vec![
            Span::styled("WARNING: ", theme::err().add_modifier(Modifier::BOLD)),
            Span::styled("You are about to launch a LIVE prune run.", theme::strong()),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Scope: ", theme::dim()),
            Span::styled(summary, theme::accent()),
        ]),
        Line::from(vec![
            Span::styled("Items will be ", theme::base()),
            Span::styled(
                "PERMANENTLY DELETED",
                theme::err().add_modifier(Modifier::BOLD),
            ),
            Span::styled(" from your Instagram account.", theme::base()),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Press ", theme::dim()),
            Span::styled("Enter", theme::key()),
            Span::styled(" or ", theme::dim()),
            Span::styled("y", theme::key()),
            Span::styled(" to proceed, or ", theme::dim()),
            Span::styled("Esc", theme::key()),
            Span::styled(" / ", theme::dim()),
            Span::styled("n", theme::key()),
            Span::styled(" to cancel.", theme::dim()),
        ]),
    ];

    f.render_widget(Paragraph::new(lines).style(theme::base()), inner);
}

fn draw_help(f: &mut Frame, area: Rect) {
    let win = centered_rect(68, 82, area);
    f.render_widget(Clear, win);
    let block = panel(" Keyboard Shortcuts · Esc/? close ", true);
    let inner = block.inner(win);
    f.render_widget(block, win);

    let width = inner.width as usize;
    let mut lines = Vec::new();
    for (title, items) in keys::HELP_SECTIONS {
        let prefix = format!("── {title} ");
        let rule = format!(
            "{prefix}{}",
            "─".repeat(width.saturating_sub(prefix.chars().count()))
        );
        lines.push(Line::from(Span::styled(
            rule,
            theme::accent().add_modifier(Modifier::BOLD),
        )));
        for (key, desc) in *items {
            lines.push(Line::from(vec![
                Span::styled(format!("  {key:<18}"), theme::key()),
                Span::styled(*desc, theme::base()),
            ]));
        }
        lines.push(Line::raw(""));
    }

    f.render_widget(Paragraph::new(lines).style(theme::base()), inner);
}
