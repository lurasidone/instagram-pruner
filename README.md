# instagram-pruner

A terminal app for deleting your own Instagram content: posts, direct messages,
liked and saved media, comments, archived posts and repost collections.

It talks to the private mobile API the app itself uses, so there is no GraphQL
portal, no scraping and no browser automation involved. Everything is driven
from a TUI, with a dry run on by default.

> Instagram can change this API whenever it likes, and automated requests are
> one of the things that triggers account checks. Run a dry run first, keep the
> request rates conservative, and expect the client to break one day.

## What it does

- Scans your posts, DMs (1:1 or group), likes, saved items, comments, archived
  posts and repost collections, and builds a plan of what would go.
- Filters by date window, scope, item count, DM kind and protected threads.
- Deletes serially with a delay between writes; scans run read-only and in
  parallel (4 requests at a time by default).
- Records every planned and completed deletion as JSONL, split per day and per
  target.
- Writes `session.json` (device identity + session tokens) with `0600`, and
  refreshes tokens from response headers as it goes.

A run never deletes the same item twice: the plan is deduplicated on
`(target, item)`, deletes keep that order, and the result set is remembered
across verification passes. Live runs then re-scan up to three times for
leftovers and retry whatever is still there.

## Requirements

- Rust stable plus a C/C++ toolchain and `cmake` (the emulated TLS fingerprint
  needs BoringSSL, built through `btls-sys`).
- A session cookie from a browser where you are logged in. Password login is not
  implemented, deliberately.

## Build and run

```sh
cargo build --release
cp .env.example .env
```

Put the value after `sessionid=` from your browser cookies into
`INSTAGRAM_SESSIONID`, then:

```sh
cargo run --release
```

In **Settings**, keep **Dry run** on for the first run and go through the
selected scope, filters and targets before letting it delete anything.

Read-only diagnostics:

```sh
cargo run --release -- --smoke                    # authenticate and list what would be scanned
cargo run --release -- --probe-thread <THREAD_ID> # dump one DM thread
cargo run --release -- --probe-activity           # dump likes/saved/archived/comments shape
cargo run --release -- --help
```

Everything the TUI can set is also a CLI flag or an environment variable, so
these probes respect the same overrides as a real run.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `INSTAGRAM_SESSIONID` | | Browser session cookie value |
| `INSTAGRAM_BASE_URL` | `https://i.instagram.com/api/v1` | API base URL |
| `INSTAGRAM_SESSION_PATH` | `session.json` | Private device and session state |
| `INSTAGRAM_SCOPE` | `posts` | `none`, `posts`, `dms`, `all` or `threads:<id>,<id>` |
| `INSTAGRAM_CONTENT` | | Activity classes: `likes`, `saved`, `comments`, `archived`, `reposts`, plus `all` / `none` |
| `INSTAGRAM_DM_KIND` | `all` | DM filter: `all`, `1on1` or `group` |
| `INSTAGRAM_BEFORE` / `INSTAGRAM_AFTER` | | Date bounds, `YYYY-MM-DD` or RFC 3339, UTC, both exclusive |
| `INSTAGRAM_LIMIT` | `0` | Stop after this many planned items; `0` means no limit |
| `INSTAGRAM_THROTTLE_MS` | `2000` | Delay between delete requests |
| `INSTAGRAM_SCAN_THROTTLE_MS` | `350` | Delay between scan requests |
| `INSTAGRAM_SCAN_CONCURRENCY` | `4` | Parallel scan requests, clamped to 1-8 |
| `INSTAGRAM_DRY_RUN` | `true` | Plan and archive without sending deletes |
| `INSTAGRAM_HIDE_THREADS` | `false` | Hide DM threads after a successful live run |
| `PRUNER_HISTORY_DIR` | `history` | Where the JSONL record goes |

`INSTAGRAM_CONTENT` is applied on top of the selected scope, and on its own when
the scope is `none`. The Targets screen (or `--protect`) keeps threads out of a
run. `RUST_LOG` turns on tracing and the HTTP client honours the usual proxy
variables such as `HTTPS_PROXY`.

## Data and privacy

`session.json`, the `history/` tree and `.env` are private account data. They are
git-ignored, and they should stay that way: never commit them, and treat the
record as an audit log rather than a restore mechanism.

## Known limitations

- Repost collections have no removal endpoint in this API. The cleanup path only
  looks at saved collections that are named or typed like reposts, and it cannot
  undo anything else.
- Comment cleanup uses a private bulk-delete endpoint that has not been verified
  against a disposable comment. Leave it off until you can check it safely.
- The OkHttp 5 profile that ships with `wreq-util` claims to be an unrelated app
  on an alpha build, so this project pins the Android-compatible OkHttp 4.12
  profile instead.

## Development

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## License

GPL-3.0, see [LICENSE](LICENSE).