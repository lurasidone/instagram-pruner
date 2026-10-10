mod clipboard;
mod config;
mod events;
mod history;
mod instagram;
mod pruner;
mod tui;
mod util;

pub(crate) use instagram::{client, device, error, model, ratelimit};

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use clap::Parser;

use crate::client::InstagramClient;
use crate::config::{Config, ScopeForm};

#[derive(Parser, Debug)]
#[command(
    name = "instagram-pruner",
    version,
    about = "Instagram content pruner (posts, DMs, likes, saved, comments, archived) via private mobile API"
)]
struct CliArgs {
    /// Connect, print what the current settings would scan, delete nothing
    #[arg(long)]
    smoke: bool,

    /// Print one DM thread's parsed items (read-only)
    #[arg(long, value_name = "THREAD_ID")]
    probe_thread: Option<String>,

    /// Print the shape of the activity feeds (likes/saved/archived/comments)
    #[arg(long)]
    probe_activity: bool,

    /// Browser `sessionid` cookie value (overrides INSTAGRAM_SESSIONID)
    #[arg(long)]
    sessionid: Option<String>,

    /// posts, dms, all, none or threads:<id>,<id>
    #[arg(long)]
    scope: Option<String>,

    /// Only items older than this date (YYYY-MM-DD or RFC3339, UTC)
    #[arg(long)]
    before: Option<String>,

    /// Only items newer than this date (YYYY-MM-DD or RFC3339, UTC)
    #[arg(long)]
    after: Option<String>,

    /// Stop after this many planned items (0 = no limit)
    #[arg(long)]
    limit: Option<String>,

    /// Delay between delete requests, in milliseconds
    #[arg(long)]
    throttle_ms: Option<String>,

    /// Delay between scan requests, in milliseconds
    #[arg(long)]
    scan_throttle_ms: Option<String>,

    /// all, 1on1 or group
    #[arg(long)]
    dm_kind: Option<String>,

    /// Plan and archive without deleting
    #[arg(long)]
    dry_run: bool,

    /// Activity classes: likes, saved, comments, archived, reposts, all, none
    #[arg(long, value_name = "CLASSES")]
    content: Option<String>,

    /// Hide DM threads after a successful live run
    #[arg(long)]
    hide_threads: bool,

    /// Never touch these threads/users
    #[arg(long, value_name = "IDS|FILE")]
    protect: Option<String>,
}

enum Mode {
    Tui,
    Smoke,
    ProbeThread(String),
    ProbeActivity,
}

fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    init_tracing();

    let args = CliArgs::parse();
    let mut cfg = Config::from_env();
    cfg.absorb_cli(&args)?;

    let smoke = args.smoke;
    let probe_activity = args.probe_activity;
    let mode = if smoke {
        Mode::Smoke
    } else if let Some(thread_id) = args.probe_thread {
        Mode::ProbeThread(thread_id)
    } else if probe_activity {
        Mode::ProbeActivity
    } else {
        Mode::Tui
    };

    let runtime = tokio::runtime::Runtime::new()?;
    match mode {
        Mode::Tui => runtime.block_on(tui::run(cfg)),
        Mode::Smoke => runtime.block_on(run_smoke(&cfg)),
        Mode::ProbeThread(thread_id) => runtime.block_on(run_probe_thread(&cfg, &thread_id)),
        Mode::ProbeActivity => runtime.block_on(run_probe_activity(&cfg)),
    }
}

impl Config {
    fn absorb_cli(&mut self, args: &CliArgs) -> Result<()> {
        if let Some(v) = &args.sessionid {
            self.sessionid = v.clone();
        }
        if let Some(v) = &args.scope {
            let mode = config::parse_scope(v).map_err(anyhow::Error::msg)?;
            self.scope_form = ScopeForm::from_scope_mode(&mode);
            self.scope = mode;
        }
        if let Some(v) = &args.before {
            self.before = v.clone();
        }
        if let Some(v) = &args.after {
            self.after = v.clone();
        }
        if let Some(v) = &args.limit {
            self.limit = v.clone();
        }
        if let Some(v) = &args.throttle_ms {
            self.throttle_ms = v.clone();
        }
        if let Some(v) = &args.scan_throttle_ms {
            self.scan_throttle_ms = v.clone();
        }
        if let Some(v) = &args.dm_kind {
            self.dm_kind = config::parse_dm_kind_value(v).map_err(anyhow::Error::msg)?;
        }
        if let Some(v) = &args.content {
            self.content = config::parse_content_value(v).map_err(anyhow::Error::msg)?;
        }
        if args.dry_run {
            self.dry_run = true;
        }
        if args.hide_threads {
            self.hide_threads = true;
        }
        if let Some(spec) = &args.protect {
            self.protect_users = parse_protect(spec)?;
        }
        Ok(())
    }
}

fn init_tracing() {
    let Ok(spec) = std::env::var("RUST_LOG") else {
        return;
    };
    if spec.trim().is_empty() {
        return;
    }
    let filter = tracing_subscriber::EnvFilter::try_new(&spec)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

/// Accepts a comma/whitespace separated list, a path, or `@path`.
fn parse_protect(spec: &str) -> Result<Vec<String>> {
    let spec = spec.trim();
    if spec.is_empty() {
        bail!("--protect is empty");
    }

    let raw = match spec.strip_prefix('@') {
        Some(path) if Path::new(path).is_file() => std::fs::read_to_string(path)
            .with_context(|| format!("--protect: cannot read file {path:?}"))?,
        _ if Path::new(spec).is_file() => std::fs::read_to_string(spec)
            .with_context(|| format!("--protect: cannot read {spec:?}"))?,
        _ => spec.to_string(),
    };

    let mut ids: Vec<String> = Vec::new();
    for id in config::thread_id_list(&raw) {
        let id = id.trim_start_matches('@');
        if !id.is_empty() && !ids.iter().any(|kept| kept == id) {
            ids.push(id.to_string());
        }
    }
    if ids.is_empty() {
        bail!("--protect: no user IDs found");
    }
    Ok(ids)
}

/// Read-only helpers share the conservative 350ms scan pacing.
async fn probe_client(cfg: &Config) -> Result<InstagramClient> {
    let cancel = Arc::new(AtomicBool::new(false));
    let limiter = Arc::new(ratelimit::RateLimiter::new(
        std::time::Duration::from_millis(350),
        cancel,
    ));
    InstagramClient::new(
        &cfg.sessionid,
        Some(&cfg.base_url),
        Some(&cfg.session_path),
        limiter,
    )
    .await
    .context("build instagram client")
}

fn require_session(cfg: &Config) -> Result<()> {
    if cfg.sessionid.trim().is_empty() {
        bail!("INSTAGRAM_SESSIONID is empty. Set it in .env or pass --sessionid");
    }
    Ok(())
}

async fn run_smoke(cfg: &Config) -> Result<()> {
    println!("[smoke] starting Instagram pruner smoke test");
    require_session(cfg)?;
    println!("[smoke] base url: {}", cfg.base_url);
    println!("[smoke] session file: {}", cfg.session_path);

    let client = probe_client(cfg).await?;

    println!("[smoke] authenticating with get_current_user()...");
    let user_id = client.user_id();
    let username = client.username();
    let full_name = client
        .fetch_user_info(&user_id)
        .await
        .map(|u| u.full_name)
        .unwrap_or_default();
    println!(
        "[smoke] OK: authenticated as @{} (pk={}, name={:?})",
        username, user_id, full_name
    );

    println!("[smoke] fetching first page of user posts...");
    match client.user_medias_page(&user_id, None, 12).await {
        Ok((items, next)) => {
            println!(
                "[smoke] OK: retrieved {} posts (more_available={})",
                items.len(),
                next.is_some()
            );
            for (i, post) in items.iter().take(5).enumerate() {
                let caption = util::truncate(&post.caption_text(), 50);
                println!("  post [{i}]: id={} caption={caption:?}", post.media_id());
            }
        }
        Err(e) => println!("[smoke] warning: fetching feed posts failed: {e}"),
    }

    println!("[smoke] fetching first page of direct messages inbox...");
    match client.direct_inbox_page(None, 20).await {
        Ok((threads, _)) => {
            println!("[smoke] OK: retrieved {} threads", threads.len());
            for (i, thread) in threads.iter().take(5).enumerate() {
                let title = thread.display_name();
                let last_msg = thread
                    .items
                    .first()
                    .and_then(|it| it.text.as_ref())
                    .map(|s| util::truncate(s, 40))
                    .unwrap_or_else(|| "(non-text item)".to_string());
                let protected = if cfg.protect_users.iter().any(|p| {
                    p == &thread.thread_id
                        || thread.users.iter().any(|u| &u.pk == p || &u.username == p)
                }) {
                    " [PROTECTED]"
                } else {
                    ""
                };
                println!(
                    "  thread [{i}]: id={} title={:?} last_msg={:?}{protected}",
                    thread.thread_id, title, last_msg
                );
            }
        }
        Err(e) => println!("[smoke] warning: fetching inbox failed: {e}"),
    }

    if !cfg.protect_users.is_empty() {
        println!(
            "[smoke] protect list contains {} user/thread IDs",
            cfg.protect_users.len()
        );
    }
    println!("[smoke] smoke test finished successfully (nothing deleted)");
    Ok(())
}

async fn run_probe_thread(cfg: &Config, thread_id: &str) -> Result<()> {
    println!("[probe-thread] probing thread: {thread_id}");
    let client = probe_client(cfg).await?;

    let thread = client
        .direct_thread(thread_id, 20, None)
        .await
        .context("direct_thread")?;
    println!("[probe-thread] thread_id: {}", thread.thread_id);
    println!("[probe-thread] title: {}", thread.display_name());
    println!("[probe-thread] is_group: {}", thread.is_group);
    println!(
        "[probe-thread] users: {}",
        thread
            .users
            .iter()
            .map(|u| format!("@{} (pk={})", u.username, u.pk))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "[probe-thread] retrieved {} items in page",
        thread.items.len()
    );
    for (i, item) in thread.items.iter().take(10).enumerate() {
        let txt = item.text.as_deref().unwrap_or("(non-text)");
        println!(
            "  item [{i}]: id={} user={:?} type={:?} text={:?}",
            item.item_id, item.user_id, item.item_type, txt
        );
    }
    Ok(())
}

async fn run_probe_activity(cfg: &Config) -> Result<()> {
    require_session(cfg)?;
    let client = probe_client(cfg).await?;
    println!(
        "[probe-activity] authenticated as @{} (pk={})",
        client.username(),
        client.user_id()
    );

    let types = r#"["ALL_MEDIA_AUTO_COLLECTION","PRODUCT_AUTO_COLLECTION","MEDIA"]"#;
    match client
        .raw_get_json("collections/list/", &[("collection_types", types)])
        .await
    {
        Ok(v) => {
            println!("\n[probe-activity] collections/list/");
            match v.get("items").and_then(|i| i.as_array()) {
                Some(items) => {
                    for item in items {
                        println!(
                            "  id={:?} name={:?} type={:?} count={:?}",
                            item.get("collection_id"),
                            item.get("collection_name"),
                            item.get("collection_type"),
                            item.get("collection_media_count"),
                        );
                    }
                }
                None => println!("  (no items array) {}", util::truncate(&v.to_string(), 400)),
            }
        }
        Err(e) => println!("\n[probe-activity] collections/list/ failed: {e}"),
    }

    for (label, path) in [
        ("liked", "feed/liked/"),
        ("saved", "feed/saved/posts/"),
        ("archived", "feed/only_me_feed/"),
    ] {
        match client
            .raw_get_json(path, &[("include_igtv_preview", "false")])
            .await
        {
            Ok(v) => {
                let items = v.get("items").and_then(|i| i.as_array());
                let n = items.map_or(0, |a| a.len());
                println!("\n[probe-activity] {path} → {n} item(s)");
                if let Some(first) = items.and_then(|a| a.first()) {
                    println!(
                        "  first item wrapped in `media` key: {}",
                        first.get("media").is_some()
                    );
                    println!(
                        "  keys: {:?}",
                        first.as_object().map(|o| o.keys().collect::<Vec<_>>())
                    );
                }
                println!(
                    "  cursors: next_max_id={:?} max_id={:?} more_available={:?}",
                    v.get("next_max_id"),
                    v.get("max_id"),
                    v.get("more_available")
                );
            }
            Err(e) => println!("\n[probe-activity] {label} ({path}) failed: {e}"),
        }
    }

    let own = client
        .user_medias_page(&client.user_id(), None, 3)
        .await
        .map(|(items, _)| items)
        .unwrap_or_default();
    let probe_media = match own.first() {
        Some(media) => Some((media.media_id(), "own newest post")),
        None => match client.saved_medias_page(None).await {
            Ok((items, _)) => items.first().map(|m| (m.media_id(), "first saved post")),
            Err(_) => None,
        },
    };

    match probe_media {
        Some((media_id, source)) => {
            println!("\n[probe-activity] media/{media_id}/comments/ ({source})");
            match client.media_comments_page(&media_id, None).await {
                Ok((comments, next)) => {
                    println!(
                        "  parsed OK: {} comment(s), next={next:?}, {} authored by me",
                        comments.len(),
                        comments
                            .iter()
                            .filter(|c| c.author_id() == client.user_id())
                            .count()
                    );
                    for c in comments.iter().take(3) {
                        println!(
                            "    pk={} by @{} at={} text={:?}",
                            c.pk,
                            c.author_username(),
                            c.created_at,
                            util::truncate(&c.text, 40)
                        );
                    }
                }
                Err(e) => println!("  failed: {e}"),
            }
        }
        None => println!("\n[probe-activity] no media available to probe comments on"),
    }

    println!("\n[probe-activity] done (nothing deleted)");
    Ok(())
}
