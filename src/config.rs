use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub sessionid: String,
    pub base_url: String,
    pub session_path: String,
    pub scope: ScopeMode,
    pub scope_form: ScopeForm,
    pub before: String,
    pub after: String,
    pub limit: String,
    pub throttle_ms: String,
    pub scan_throttle_ms: String,
    pub concurrency: String,
    pub dry_run: bool,
    pub hide_threads: bool,
    pub content: ContentFlags,
    pub protect_users: Vec<String>,
    pub dm_kind: DmKind,
    pub history_dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeMode {
    None,
    Posts,
    Dms,
    All,
    Threads(String),
}

impl ScopeMode {
    pub fn kind(&self) -> ScopeKind {
        match self {
            Self::None => ScopeKind::None,
            Self::Posts => ScopeKind::Posts,
            Self::Dms => ScopeKind::Dms,
            Self::All => ScopeKind::All,
            Self::Threads(_) => ScopeKind::Threads,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScopeKind {
    None,
    #[default]
    Posts,
    Dms,
    All,
    Threads,
}

impl ScopeKind {
    pub const ALL: [ScopeKind; 5] = [
        ScopeKind::None,
        ScopeKind::Posts,
        ScopeKind::Dms,
        ScopeKind::All,
        ScopeKind::Threads,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ScopeKind::None => "none (content only)",
            ScopeKind::Posts => "posts",
            ScopeKind::Dms => "all DMs",
            ScopeKind::All => "all (posts + DMs)",
            ScopeKind::Threads => "threads by ID",
        }
    }

    pub fn has_payload(self) -> bool {
        self == ScopeKind::Threads
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }

    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeForm {
    pub kind: ScopeKind,
    pub threads: String,
}

impl ScopeForm {
    pub fn from_scope_mode(mode: &ScopeMode) -> Self {
        Self {
            kind: mode.kind(),
            threads: match mode {
                ScopeMode::Threads(ids) => ids.clone(),
                _ => String::new(),
            },
        }
    }

    pub fn summary(&self) -> String {
        match self.kind {
            ScopeKind::None => "content only".to_string(),
            ScopeKind::Posts => "posts".to_string(),
            ScopeKind::Dms => "DMs".to_string(),
            ScopeKind::All => "all".to_string(),
            ScopeKind::Threads => {
                let trimmed = self.threads.trim();
                if trimmed.is_empty() {
                    "threads: (none)".to_string()
                } else {
                    format!("threads: {trimmed}")
                }
            }
        }
    }

    pub fn next(&mut self) {
        self.kind = self.kind.next();
    }

    pub fn prev(&mut self) {
        self.kind = self.kind.prev();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContentFlags {
    pub likes: bool,
    pub saved: bool,
    pub comments: bool,
    pub archived: bool,
    pub reposts: bool,
}

impl ContentFlags {
    /// Order used by `label()` and by the settings screen.
    const NAMES: [&'static str; 5] = ["likes", "saved", "comments", "archived", "reposts"];

    const ALL: Self = Self {
        likes: true,
        saved: true,
        comments: true,
        archived: true,
        reposts: true,
    };

    fn as_array(self) -> [bool; 5] {
        [
            self.likes,
            self.saved,
            self.comments,
            self.archived,
            self.reposts,
        ]
    }

    /// Enables one class; false if the token is not a known class name.
    fn enable(&mut self, token: &str) -> bool {
        match token {
            "likes" | "like" => self.likes = true,
            "saved" | "save" => self.saved = true,
            "comments" | "comment" => self.comments = true,
            "archived" | "archive" => self.archived = true,
            "reposts" | "repost" => self.reposts = true,
            "all" => *self = Self::ALL,
            "none" => *self = Self::default(),
            _ => return false,
        }
        true
    }

    pub fn any(self) -> bool {
        self.as_array().iter().any(|on| *on)
    }

    pub fn count(self) -> usize {
        self.as_array().iter().filter(|on| **on).count()
    }

    pub fn label(self) -> String {
        let on: Vec<&str> = Self::NAMES
            .iter()
            .zip(self.as_array())
            .filter(|(_, enabled)| *enabled)
            .map(|(name, _)| *name)
            .collect();
        if on.is_empty() {
            "none".to_string()
        } else {
            on.join("+")
        }
    }
}

pub fn thread_id_list(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in raw.split([',', ' ', '\t', '\n']) {
        let trimmed = part.trim();
        if !trimmed.is_empty() && !out.iter().any(|kept| kept == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DmKind {
    #[default]
    All,
    OneOnOne,
    Group,
}

impl DmKind {
    pub fn matches(&self, is_group: bool) -> bool {
        match self {
            Self::All => true,
            Self::OneOnOne => !is_group,
            Self::Group => is_group,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::OneOnOne => "1on1",
            Self::Group => "group",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::All => Self::OneOnOne,
            Self::OneOnOne => Self::Group,
            Self::Group => Self::All,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::All => Self::Group,
            Self::OneOnOne => Self::All,
            Self::Group => Self::OneOnOne,
        }
    }
}

impl Config {
    pub fn from_env() -> Self {
        let scope = parse_scope_mode("INSTAGRAM_SCOPE");
        let scope_form = ScopeForm::from_scope_mode(&scope);
        Self {
            sessionid: env::var("INSTAGRAM_SESSIONID").unwrap_or_default(),
            base_url: trimmed_env("INSTAGRAM_BASE_URL")
                .map(|url| url.trim_end_matches('/').to_string())
                .unwrap_or_else(|| crate::device::API_BASE.to_string()),
            session_path: trimmed_env("INSTAGRAM_SESSION_PATH")
                .unwrap_or_else(|| crate::device::DEFAULT_SESSION_PATH.to_string()),
            scope,
            scope_form,
            before: env::var("INSTAGRAM_BEFORE")
                .unwrap_or_default()
                .trim()
                .to_string(),
            after: env::var("INSTAGRAM_AFTER")
                .unwrap_or_default()
                .trim()
                .to_string(),
            limit: parse_string_int("INSTAGRAM_LIMIT", "0"),
            throttle_ms: parse_string_int("INSTAGRAM_THROTTLE_MS", "2000"),
            scan_throttle_ms: parse_string_int("INSTAGRAM_SCAN_THROTTLE_MS", "350"),
            concurrency: parse_string_int("INSTAGRAM_SCAN_CONCURRENCY", "4"),
            dry_run: parse_bool("INSTAGRAM_DRY_RUN", true),
            hide_threads: parse_bool("INSTAGRAM_HIDE_THREADS", false),
            content: parse_content_flags(),
            protect_users: Vec::new(),
            dm_kind: parse_dm_kind(),
            history_dir: env::var("PRUNER_HISTORY_DIR").unwrap_or_else(|_| "./history".to_string()),
        }
    }
}

/// Environment value with whitespace stripped; `None` when unset or blank.
fn trimmed_env(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn content_tokens(raw: &str) -> impl Iterator<Item = String> + '_ {
    raw.split([',', ' ', '\t', '\n'])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
}

pub(crate) fn parse_content_flags() -> ContentFlags {
    let Ok(raw) = env::var("INSTAGRAM_CONTENT") else {
        return ContentFlags::default();
    };
    let mut flags = ContentFlags::default();
    for token in content_tokens(&raw) {
        if !flags.enable(&token) {
            warn(
                "INSTAGRAM_CONTENT",
                &token,
                "expected likes|saved|comments|archived|reposts|all|none; ignoring",
            );
        }
    }
    flags
}

pub(crate) fn parse_content_value(raw: &str) -> Result<ContentFlags, String> {
    let mut flags = ContentFlags::default();
    for token in content_tokens(raw) {
        if !flags.enable(&token) {
            return Err(format!(
                "invalid --content '{token}': expected likes, saved, comments, archived, reposts, \
                 all, or none"
            ));
        }
    }
    Ok(flags)
}

fn dm_kind_from_str(raw: &str) -> Option<DmKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "all" => Some(DmKind::All),
        "1on1" | "1-on-1" | "dm" => Some(DmKind::OneOnOne),
        "group" | "grp" => Some(DmKind::Group),
        _ => None,
    }
}

pub(crate) fn parse_dm_kind() -> DmKind {
    let Ok(raw) = env::var("INSTAGRAM_DM_KIND") else {
        return DmKind::All;
    };
    dm_kind_from_str(&raw).unwrap_or_else(|| {
        warn(
            "INSTAGRAM_DM_KIND",
            &raw,
            "expected all|1on1|group; using all",
        );
        DmKind::All
    })
}

pub(crate) fn parse_dm_kind_value(raw: &str) -> Result<DmKind, String> {
    dm_kind_from_str(raw)
        .ok_or_else(|| format!("invalid --dm-kind '{raw}': expected 'all', '1on1', or 'group'"))
}

pub(crate) fn parse_scope(raw: &str) -> Result<ScopeMode, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("posts") {
        return Ok(ScopeMode::Posts);
    }
    if trimmed.eq_ignore_ascii_case("none") || trimmed.eq_ignore_ascii_case("content") {
        return Ok(ScopeMode::None);
    }
    if trimmed.eq_ignore_ascii_case("dms") || trimmed.eq_ignore_ascii_case("all_dms") {
        return Ok(ScopeMode::Dms);
    }
    if trimmed.eq_ignore_ascii_case("all") {
        return Ok(ScopeMode::All);
    }
    for prefix in ["threads:", "threads "] {
        if let Some(rest) = stripped_prefix_ci(trimmed, prefix) {
            let ids = thread_id_list(rest);
            if ids.is_empty() {
                return Err(format!("scope '{prefix}' requires at least one thread id"));
            }
            return Ok(ScopeMode::Threads(ids.join(",")));
        }
    }
    let ids = thread_id_list(trimmed);
    if !ids.is_empty() && ids.iter().all(|id| id.chars().all(|c| c.is_ascii_digit())) {
        return Ok(ScopeMode::Threads(ids.join(",")));
    }
    Err(format!(
        "unrecognised scope '{raw}': expected posts, dms, all, or threads:<id>,<id>"
    ))
}

fn parse_scope_mode(key: &str) -> ScopeMode {
    let Ok(raw) = env::var(key) else {
        return ScopeMode::Posts;
    };
    match parse_scope(&raw) {
        Ok(mode) => mode,
        Err(_) => {
            warn(key, &raw, "unrecognised scope; using 'posts'");
            ScopeMode::Posts
        }
    }
}

fn stripped_prefix_ci<'a>(haystack: &'a str, prefix: &str) -> Option<&'a str> {
    haystack
        .get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .and_then(|_| haystack.get(prefix.len()..))
}

pub(crate) fn parse_bool(key: &str, default: bool) -> bool {
    let Ok(raw) = env::var(key) else {
        return default;
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" | "" => false,
        _ => {
            warn(
                key,
                &raw,
                &format!("expected bool (true/false); using {default}"),
            );
            default
        }
    }
}

pub const MAX_SCAN_CONCURRENCY: u32 = 8;

pub fn clamp_scan_concurrency(raw: u64) -> u32 {
    raw.clamp(1, u64::from(MAX_SCAN_CONCURRENCY)) as u32
}

pub(crate) fn parse_string_int(key: &str, default: &str) -> String {
    let Ok(raw) = env::var(key) else {
        return default.to_string();
    };
    let trimmed = raw.trim();
    if trimmed.parse::<u64>().is_ok() {
        return trimmed.to_string();
    }
    warn(
        key,
        &raw,
        &format!("expected non-negative integer; using {default}"),
    );
    default.to_string()
}

fn warn(key: &str, value: &str, hint: &str) {
    eprintln!("[instagram-pruner] {key}={value:?}: {hint}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_key(prefix: &str) -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        format!("PRUNER_TEST_{prefix}_{}", N.fetch_add(1, Ordering::Relaxed))
    }

    #[test]
    fn scope_posts_aliases() {
        for v in ["", "posts", "POSTS"] {
            let k = unique_key("scope");
            env::set_var(&k, v);
            assert!(matches!(parse_scope_mode(&k), ScopeMode::Posts));
            env::remove_var(&k);
        }
    }

    #[test]
    fn scope_dms_and_all() {
        let k = unique_key("scope");
        env::set_var(&k, "dms");
        assert!(matches!(parse_scope_mode(&k), ScopeMode::Dms));
        env::set_var(&k, "all_dms");
        assert!(matches!(parse_scope_mode(&k), ScopeMode::Dms));
        env::set_var(&k, "all");
        assert!(matches!(parse_scope_mode(&k), ScopeMode::All));
        env::remove_var(&k);
    }

    #[test]
    fn scope_threads_prefix_and_bare() {
        let k = unique_key("scope");
        env::set_var(&k, "threads:111,222");
        assert!(matches!(&parse_scope_mode(&k), ScopeMode::Threads(s) if s == "111,222"));
        env::set_var(&k, "1,2,3");
        assert!(matches!(&parse_scope_mode(&k), ScopeMode::Threads(s) if s == "1,2,3"));
        env::set_var(&k, "987654321");
        assert!(matches!(&parse_scope_mode(&k), ScopeMode::Threads(s) if s == "987654321"));
        env::remove_var(&k);
    }

    #[test]
    fn scope_none_is_content_only() {
        let k = unique_key("scope");
        env::set_var(&k, "none");
        assert!(matches!(parse_scope_mode(&k), ScopeMode::None));
        env::set_var(&k, "content");
        assert!(matches!(parse_scope_mode(&k), ScopeMode::None));
        env::remove_var(&k);
    }

    #[test]
    fn content_label_names_every_enabled_class() {
        assert_eq!(ContentFlags::default().label(), "none");
        let mut flags = ContentFlags {
            likes: true,
            ..Default::default()
        };
        assert_eq!(flags.label(), "likes");
        flags.saved = true;
        assert_eq!(flags.label(), "likes+saved");
        flags.comments = true;
        flags.archived = true;
        flags.reposts = true;
        assert_eq!(flags.label(), "likes+saved+comments+archived+reposts");
        assert_eq!(flags.count(), 5);
    }

    #[test]
    fn content_flags_parse_and_reject_typos() {
        assert_eq!(parse_content_value("").unwrap(), ContentFlags::default());
        assert_eq!(
            parse_content_value("none").unwrap(),
            ContentFlags::default()
        );

        let likes = parse_content_value("likes").unwrap();
        assert!(likes.likes && !likes.saved && likes.count() == 1);

        let both = parse_content_value("likes, saved").unwrap();
        assert!(both.likes && both.saved && both.count() == 2);

        let all = parse_content_value("all").unwrap();
        assert!(all.any() && all.count() == 5);

        assert!(parse_content_value("likez").is_err());
    }

    #[test]
    fn scan_concurrency_clamps_into_supported_range() {
        assert_eq!(clamp_scan_concurrency(0), 1);
        assert_eq!(clamp_scan_concurrency(1), 1);
        assert_eq!(clamp_scan_concurrency(4), 4);
        assert_eq!(
            clamp_scan_concurrency(u64::from(MAX_SCAN_CONCURRENCY)),
            MAX_SCAN_CONCURRENCY
        );
        assert_eq!(clamp_scan_concurrency(999), MAX_SCAN_CONCURRENCY);
    }

    #[test]
    fn dry_run_defaults_to_enabled() {
        let key = unique_key("dry_run");
        env::remove_var(&key);
        assert!(parse_bool(&key, true));
    }

    #[test]
    fn scope_form_cycling() {
        let mut form = ScopeForm::default();
        assert_eq!(form.kind, ScopeKind::Posts);
        form.next();
        assert_eq!(form.kind, ScopeKind::Dms);
        form.next();
        assert_eq!(form.kind, ScopeKind::All);
        form.next();
        assert_eq!(form.kind, ScopeKind::Threads);
        form.next();
        assert_eq!(form.kind, ScopeKind::None);
        form.next();
        assert_eq!(form.kind, ScopeKind::Posts);
        form.prev();
        assert_eq!(form.kind, ScopeKind::None);
        form.prev();
        assert_eq!(form.kind, ScopeKind::Threads);
    }

    #[test]
    fn scope_form_mirrors_scope_mode() {
        let form = ScopeForm::from_scope_mode(&ScopeMode::Posts);
        assert_eq!(form.kind, ScopeKind::Posts);
        assert!(form.threads.is_empty());

        let form = ScopeForm::from_scope_mode(&ScopeMode::Threads("7,8".to_string()));
        assert_eq!(form.kind, ScopeKind::Threads);
        assert_eq!(form.threads, "7,8");
        assert_eq!(form.summary(), "threads: 7,8");
    }

    #[test]
    fn parse_dm_kind_variants() {
        assert_eq!(parse_dm_kind_value("all").unwrap(), DmKind::All);
        assert_eq!(parse_dm_kind_value("1on1").unwrap(), DmKind::OneOnOne);
        assert_eq!(parse_dm_kind_value("group").unwrap(), DmKind::Group);
        assert!(parse_dm_kind_value("invalid").is_err());
    }

    #[test]
    fn none_scope_summary_is_content_only() {
        let form = ScopeForm::from_scope_mode(&ScopeMode::None);
        assert_eq!(form.summary(), "content only");
        assert!(!form.kind.has_payload());
    }
}
