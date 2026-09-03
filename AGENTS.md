## git-opendal

This project is a Git remote helper that stores repositories on any OpenDAL
backend (S3, GCS, Azure Blob, local fs, etc.). The goal of this file is to
capture immediately useful, discoverable facts for an AI so it can make
productive edits without guessing project conventions.

- Language: Rust (async, tokio). Binary invoked by Git as `git-remote-opendal`.
- Transport entry point: `src/main.rs` (parses args, builds operator, runs helper state machine).
- Workflow entry point: `src/bin/git-opendal.rs`, installed as `git-opendal` and invoked by Git as `git opendal ...`.

Core components (read across these files to understand flows):

- `src/main.rs` — CLI entry, tracing config (logs go to stderr), builds the
  OpenDAL operator, starts `helper::RemoteHelper`.
- `src/config.rs` — URL parsing and parameter resolution. Backend parameters
  come from environment variables (`OPENDAL_<SCHEME>_<KEY>`). Keys used inside
  code are lowercase-hyphenated (e.g. `access-key-id`, `credential-path`). See
  `collect_env_params` for exact behavior.
- `src/operator.rs` — constructs `opendal::Operator` per scheme. User-facing
  schemes: `s3`, `gcs`, `azblob`, `gdrive`, `fs`. The `memory` scheme exists
  only for unit tests / single-process debugging and should not be documented
  as a normal backend. To add a backend: 1) enable the service feature in
  `Cargo.toml`; 2) add `build_<scheme>`; 3) add an arm in `build_operator`;
  4) update `USER_SUPPORTED_SCHEMES` and README if it is user-facing.
- `src/storage.rs` — storage layout and helpers. Important layout:

  info/refs.json  — JSON { refs: {"refs/heads/...": "<sha>"}, bundles: ["objects/....bundle"] }
  objects/*.bundle — one bundle file per push

  RefsStore has fields `refs: HashMap<String,String>` and `bundles: Vec<String>`.

- `src/protocol.rs` — small parser/writer for Git remote helper line protocol.
  Use `write_raw` for fast-import data and `write_line` / `write_blank` for
  protocol lines. Note: diagnostics MUST go to stderr (see tracing setup).
- `src/helper.rs` — main protocol state machine implementing `import`/`push`.
  Push flow uses `git fast-import`/`git bundle create` and uploads bundles.
  Fetch flow downloads bundles and runs `git bundle unbundle` then emits a
  fast-import stream. Both flows rely on the `git` binary being present.

Developer workflows (commands discovered from README / files):

- Build (dev):
  cargo install --path .
  # or for local debugging
  cargo build --release

- Formatting:
  cargo fmt --check

- Tests: run unit tests (module tests exist in several files):
  cargo test

- Debug logs: set `GIT_REMOTE_OPENDAL_LOG` before invoking git to control
  tracing (goes to stderr):
  export GIT_REMOTE_OPENDAL_LOG=debug

- One installation, two Git extensions:

  cargo install --path .
  git opendal setup --backend fs --path /tmp/my-bare-repos/myrepo.git --push

- Manual smoke (local fs):
  git remote add origin opendal://fs/tmp/my-bare-repos/myrepo.git
  git push origin main
  git clone opendal://fs/tmp/my-bare-repos/myrepo.git

- Releases:
  Releases are managed via `release-plz` (`release-plz.toml` and
  `.github/workflows/release-plz.yml`). Conventional Commits (`feat:`, `fix:`,
  `chore:`) merged to `main` update a standing Release PR. Merging that PR tags
  the release, publishes to crates.io, compiles binary assets across 5 targets,
  and publishes the GitHub Release with changelog.

Project-specific conventions and gotchas:

- Config resolution: backend parameters come only from
  `OPENDAL_<SCHEME>_<KEY>` env vars. `config.rs` maps env var suffixes to
  lowercase-hyphenated keys internally.
- Logging: tracing is configured to write to stderr intentionally. Do not
  write protocol data to stderr — it will break Git's protocol (see main.rs).
- Capabilities: branch and tag refspecs are both advertised. If refspec
  behavior changes, update `handle_capabilities` and fetch/push behavior
  together.
- Options: `option progress` is intentionally reported as `unsupported` unless
  real progress messages are emitted on stdout according to the remote-helper
  protocol.
- Storage layout: `refs.json` contains the canonical list of refs and the
  ordered list of bundle keys. Concurrent pushes use a last-writer-wins
  strategy — there's no server-side locking implemented.
- Temporary files: imports/pushes use `tempfile` RAII cleanup plus `git bundle`
  tooling. The helper assumes `git` is available on PATH and that subprocess
  calls to `git` succeed; update `helper.rs` if you need platform-specific
  adjustments.
- Tests that mutate environment variables should use `serial_test`, because env
  vars are process-global and Rust tests run in parallel by default.

Extensibility notes (common edits you may be asked to make):

- Add a new backend: modify `Cargo.toml` to add opendal feature, then add
  a `build_<name>` in `src/operator.rs` and an arm in `build_operator`.
- Change ref storage shape: `src/storage.rs::RefsStore` centralises the
  format—update read/write logic and consider migration when changing it.
- Protocol changes: `src/protocol.rs` is the single place for parsing and
  writing lines. Modifying it affects `helper.rs` heavily.
- Workflow changes: `src/bin/git-opendal.rs` may validate tools and credentials,
  construct URLs, register remotes, bootstrap a push, and report health. It
  must not duplicate transport or storage behavior from the remote helper.

Where to look first for a change request
- If it's about config/env handling: `src/config.rs`.
- If it's about cloud client setup: `src/operator.rs` and Cargo features.
- If it's about wire protocol or helper behavior: `src/protocol.rs` and
  `src/helper.rs`.

If anything in this file is unclear or you'd like more examples (e.g., a
step-by-step local debug session or an integration test harness), tell me
which part to expand and I'll update this doc.
