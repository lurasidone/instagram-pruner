use crate::config::{Config, ScopeKind};
use crate::events::PrunerEvent;
use crate::model::DirectThread;
use crate::pruner::TargetResolution;

use super::super::action::{self, Action, Effect, Field, Overlay, Screen};
use super::{mask_sessionid, AppState, Form, RunStatus};

#[test]
fn form_validation_checks_sessionid_and_dates() {
    let mut form = Form::from_config(&Config::from_env());
    form.sessionid = String::new();
    let errs = form.validate();
    assert!(errs.contains_key(&Field::SessionId));

    form.sessionid = "fake_sessionid".to_string();
    form.before = "invalid_date".to_string();
    let errs = form.validate();
    assert!(errs.contains_key(&Field::Before));

    form.before = "2026-01-01".to_string();
    let errs = form.validate();
    assert!(!errs.contains_key(&Field::Before));
}

#[test]
fn settings_navigation_skips_hidden_thread_payload() {
    let mut app = AppState::new(Config::from_env());
    app.screen = Screen::Settings;
    app.settings.cursor = Field::ALL
        .iter()
        .position(|field| *field == Field::Scope)
        .unwrap();

    app.apply_action(Action::Down);
    assert_eq!(app.settings.field(), Field::Before);

    app.form.scope.kind = ScopeKind::Threads;
    app.settings.cursor = Field::ALL
        .iter()
        .position(|field| *field == Field::Scope)
        .unwrap();
    app.apply_action(Action::Down);
    assert_eq!(app.settings.field(), Field::Payload);
}

#[test]
fn cycling_away_from_thread_scope_moves_focus_off_hidden_payload() {
    let mut app = AppState::new(Config::from_env());
    app.screen = Screen::Settings;
    app.form.scope.kind = ScopeKind::Threads;
    app.settings.cursor = Field::ALL
        .iter()
        .position(|field| *field == Field::Scope)
        .unwrap();
    app.apply_action(Action::CycleNext);
    assert_eq!(app.form.scope.kind, ScopeKind::None);
    assert_eq!(app.settings.field(), Field::Scope);
}

#[test]
fn target_promotion_sets_threads_scope() {
    let mut app = AppState::new(Config::from_env());
    app.screen = Screen::Targets;
    app.targets.threads = vec![thread("thread_456"), thread("thread_123")];
    app.targets
        .selected_threads
        .insert("thread_123".to_string());
    app.targets
        .selected_threads
        .insert("thread_456".to_string());

    app.apply_action(Action::Activate);

    assert_eq!(app.form.scope.kind, ScopeKind::Threads);
    assert_eq!(app.form.scope.threads, "thread_456, thread_123");
    assert_eq!(app.screen, Screen::Settings);
}

#[test]
fn selecting_quit_from_command_palette_returns_quit_effect() {
    let mut app = AppState::new(Config::from_env());
    app.overlay = Overlay::Palette;
    app.palette.matches = action::commands()
        .iter()
        .enumerate()
        .filter(|(_, command)| command.id == "quit")
        .map(|(index, _)| index)
        .collect();
    app.palette.cursor = 0;

    assert!(matches!(
        app.apply_action(Action::PaletteSelect).as_slice(),
        [Effect::Quit]
    ));
}

#[test]
fn active_run_cannot_be_started_again() {
    let mut app = AppState::new(Config::from_env());
    app.form.sessionid = "fake_sessionid".into();
    app.form.dry_run = true;
    app.run.status = RunStatus::Running;

    assert!(app.apply_action(Action::Activate).is_empty());
    assert!(app
        .toast
        .as_ref()
        .is_some_and(|toast| toast.text.contains("already active")));
}

fn thread(id: &str) -> DirectThread {
    DirectThread {
        thread_id: id.to_string(),
        pk: None,
        thread_title: None,
        is_group: false,
        users: Vec::new(),
        items: Vec::new(),
        has_older: false,
        prev_cursor: None,
        oldest_cursor: None,
    }
}

fn run_targets(app: &mut AppState) -> Vec<TargetResolution> {
    app.form.sessionid = "fake_sessionid".to_string();
    app.form.dry_run = true;
    app.form.limit = "0".to_string();
    app.form.throttle_ms = "2000".to_string();
    app.form.scan_throttle_ms = "350".to_string();
    app.screen = Screen::Settings;
    let effects = app.apply_action(Action::Activate);
    match effects.into_iter().next() {
        Some(Effect::StartRun(cfg)) => cfg.targets,
        other => panic!("expected StartRun, got {other:?}"),
    }
}

#[test]
fn picked_threads_override_blanket_scope() {
    let mut app = AppState::new(Config::from_env());
    app.form.scope.kind = ScopeKind::All;
    app.targets.threads = vec![thread("t1"), thread("t2"), thread("t3")];
    app.targets.selected_threads.insert("t3".to_string());
    app.targets.selected_threads.insert("t1".to_string());

    match run_targets(&mut app).as_slice() {
        [TargetResolution::ThreadObjects(picked)] => {
            let ids: Vec<&str> = picked.iter().map(|t| t.thread_id.as_str()).collect();
            assert_eq!(ids, ["t1", "t3"]);
        }
        other => panic!("expected only picked threads, got {other:?}"),
    }
}

#[test]
fn empty_picker_keeps_scope_targets() {
    let mut app = AppState::new(Config::from_env());
    app.form.scope.kind = ScopeKind::All;
    app.targets.threads = vec![thread("t1")];

    let targets = run_targets(&mut app);
    assert!(matches!(
        targets.as_slice(),
        [TargetResolution::Posts, TargetResolution::AllDms]
    ));
}

#[test]
fn dms_scope_is_narrowed_by_picks() {
    let mut app = AppState::new(Config::from_env());
    app.form.scope.kind = ScopeKind::Dms;
    app.targets.threads = vec![thread("t1"), thread("t2")];
    app.targets.selected_threads.insert("t2".to_string());

    match run_targets(&mut app).as_slice() {
        [TargetResolution::ThreadObjects(picked)] => {
            assert_eq!(picked.len(), 1);
            assert_eq!(picked[0].thread_id, "t2");
        }
        other => panic!("expected only picked threads, got {other:?}"),
    }
}

#[test]
fn typed_thread_scope_beats_stale_picks() {
    let mut app = AppState::new(Config::from_env());
    app.form.scope.kind = ScopeKind::Threads;
    app.form.scope.threads = "typed_1, typed_2".to_string();
    app.targets.threads = vec![thread("t1")];
    app.targets.selected_threads.insert("t1".to_string());

    match run_targets(&mut app).as_slice() {
        [TargetResolution::Threads(ids)] => assert_eq!(ids, &["typed_1", "typed_2"]),
        other => panic!("expected the typed ids to win, got {other:?}"),
    }
}

#[test]
fn posts_scope_ignores_picks() {
    let mut app = AppState::new(Config::from_env());
    app.form.scope.kind = ScopeKind::Posts;
    app.targets.threads = vec![thread("t1")];
    app.targets.selected_threads.insert("t1".to_string());

    assert!(matches!(
        run_targets(&mut app).as_slice(),
        [TargetResolution::Posts]
    ));
}

#[test]
fn refreshed_inbox_drops_unresolvable_picks() {
    let mut app = AppState::new(Config::from_env());
    app.form.scope.kind = ScopeKind::All;
    app.targets.threads = vec![thread("t1"), thread("t2")];
    app.targets.selected_threads.insert("t1".to_string());
    app.targets.selected_threads.insert("t2".to_string());

    app.apply_event(PrunerEvent::DMsLoaded {
        channels: vec![thread("t2")],
    });

    assert_eq!(
        app.targets.selected_threads.iter().collect::<Vec<_>>(),
        vec!["t2"]
    );

    match run_targets(&mut app).as_slice() {
        [TargetResolution::ThreadObjects(picked)] => assert_eq!(picked.len(), 1),
        other => panic!("expected the surviving pick, got {other:?}"),
    }
}

#[test]
fn mask_sessionid_hides_secrets() {
    assert_eq!(mask_sessionid(""), "(empty)");
    assert_eq!(mask_sessionid("123"), "••••••");
    assert_eq!(mask_sessionid("1234567890abcdef"), "••••••••••••cdef");
    assert_eq!(mask_sessionid("1234567890🔐"), "••••••••••••890🔐");
}

#[test]
fn space_toggles_the_focused_content_class_into_the_run() {
    let mut app = AppState::new(Config::from_env());
    app.form.sessionid = "fake_sessionid".to_string();
    app.form.dry_run = true;
    app.form.scope.kind = ScopeKind::None;
    app.screen = Screen::Settings;

    assert!(!app.form.content.any());

    app.settings.cursor = Field::ALL
        .iter()
        .position(|f| *f == Field::ContentLikes)
        .expect("content field present");
    app.apply_action(Action::Toggle);

    assert!(app.form.content.likes);
    assert_eq!(app.form.content.label(), "likes");

    let cfg = app.form.to_pruner_config().expect("valid config");
    assert!(cfg.targets.is_empty());
    assert!(cfg.content.likes && cfg.content.count() == 1);

    app.apply_action(Action::Toggle);
    assert!(!app.form.content.any());
    assert!(app.form.validate().contains_key(&Field::Scope));
}
