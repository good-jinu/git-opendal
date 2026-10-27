//! Core remote helper state machine.
//!
//! **Fetch flow** (git clone / git fetch):
//!   1. git sends `capabilities` → we reply `import\npush\nrefspec ...\n\n`
//!   2. git sends `list`         → we reply with our stored refs
//!   3. git sends `import <ref>` batches → we apply stored bundles via
//!      `git bundle unbundle` and emit a fast-import stream back to git.
//!
//! **Push flow** (git push):
//!   1. git sends `capabilities` → same
//!   2. git sends `list for-push` → we reply with stored refs
//!   3. git sends `push <src>:<dst>` (one per ref, terminated by blank line)
//!      → we create a git bundle from the local refs, upload it, update refs.json.

use crate::config::RemoteConfig;
use crate::protocol::{self, Command};
use crate::storage::{RefsStore, Storage};
use anyhow::{Context, Result, bail};
use opendal::Operator;
use std::io;
use std::process::{Command as SysCommand, Stdio};
use tracing::{debug, info, warn};

/// Remote namespaces mapped into the private namespace by our refspecs.
const MAPPED_NAMESPACES: [&str; 2] = ["heads", "tags"];

pub struct RemoteHelper {
    storage: Storage,
    #[allow(dead_code)]
    cfg: RemoteConfig,
    /// Private ref namespace that `import` writes to, e.g. `refs/opendal/origin`.
    ///
    /// Git reads imported tips from here and then applies the user's own fetch
    /// refspec. Writing straight into `refs/heads/*` instead would move local
    /// branches behind Git's back and break `git clone`.
    private_prefix: String,
}

impl RemoteHelper {
    pub fn new(op: Operator, cfg: RemoteConfig, remote_name: &str) -> Self {
        Self {
            storage: Storage::new(op),
            cfg,
            private_prefix: private_prefix(remote_name),
        }
    }

    /// Main protocol loop — reads commands from git until EOF.
    pub async fn run(&mut self) -> Result<()> {
        let stdin = io::stdin();
        let mut stdin = stdin.lock();

        loop {
            let cmd = protocol::read_command(&mut stdin)?;
            match cmd {
                None => {
                    debug!("stdin closed — exiting");
                    break;
                }
                Some(Command::Blank) => {
                    debug!("received unexpected blank line — ignoring");
                }
                Some(Command::Capabilities) => {
                    self.handle_capabilities()?;
                }
                Some(Command::List) => {
                    self.handle_list(false).await?;
                }
                Some(Command::ListForPush) => {
                    self.handle_list(true).await?;
                }
                Some(Command::Import(first_ref)) => {
                    let mut refs = first_ref;
                    loop {
                        match protocol::read_command(&mut stdin)? {
                            Some(Command::Import(r)) => refs.extend(r),
                            Some(Command::Blank) => break,
                            Some(other) => {
                                bail!("Unexpected command during import batch: {:?}", other)
                            }
                            None => break,
                        }
                    }
                    self.handle_import(&refs).await?;
                }
                Some(Command::Push { src, dst, force }) => {
                    let mut pushes = vec![(src, dst, force)];
                    loop {
                        match protocol::read_command(&mut stdin)? {
                            Some(Command::Push { src, dst, force }) => {
                                pushes.push((src, dst, force))
                            }
                            Some(Command::Blank) | None => break,
                            Some(other) => {
                                bail!("Unexpected command during push batch: {:?}", other)
                            }
                        }
                    }
                    self.handle_push(&pushes).await?;
                }
                Some(Command::Option(key, val)) => {
                    self.handle_option(&key, &val)?;
                }
                Some(Command::Unknown(cmd)) => {
                    warn!("Unknown command from git: '{}' — ignoring", cmd);
                }
            }
        }

        Ok(())
    }

    // ─── Capabilities ────────────────────────────────────────────────────────

    fn handle_capabilities(&self) -> Result<()> {
        protocol::write_line("import")?;
        protocol::write_line("push")?;
        protocol::write_line("option")?;
        for ns in MAPPED_NAMESPACES {
            protocol::write_line(&format!(
                "refspec refs/{ns}/*:{}/{ns}/*",
                self.private_prefix
            ))?;
        }
        protocol::write_blank()?;
        Ok(())
    }

    // ─── List refs ───────────────────────────────────────────────────────────

    async fn handle_list(&self, _for_push: bool) -> Result<()> {
        let store = self
            .storage
            .load_refs()
            .await
            .context("Failed to load refs for list")?;

        if store.refs.is_empty() {
            protocol::write_blank()?;
            return Ok(());
        }

        let head_target = Self::determine_head(&store);

        for (ref_name, sha) in &store.refs {
            protocol::write_line(&format!("{} {}", sha, ref_name))?;
        }

        if let Some(head) = head_target {
            protocol::write_line(&format!("@{} HEAD", head))?;
        }

        protocol::write_blank()?;
        Ok(())
    }

    fn determine_head(store: &RefsStore) -> Option<String> {
        let preferred = ["refs/heads/main", "refs/heads/master"];
        for p in &preferred {
            if store.refs.contains_key(*p) {
                return Some(p.to_string());
            }
        }
        store.refs.keys().next().cloned()
    }

    // ─── Import (fetch) ──────────────────────────────────────────────────────

    async fn handle_import(&self, refs: &[String]) -> Result<()> {
        info!("import requested for refs: {:?}", refs);

        let store = self.storage.load_refs().await?;

        if store.bundles.is_empty() {
            self.emit_empty_fast_import()?;
            return Ok(());
        }

        let tmp_dir = tempfile::tempdir().context("Failed to create temp directory")?;

        for bundle_key in &store.bundles {
            let data = self
                .storage
                .download_bundle(bundle_key)
                .await
                .with_context(|| format!("Failed to download bundle: {}", bundle_key))?;

            let bundle_path = tmp_dir.path().join("current.bundle");
            std::fs::write(&bundle_path, &data)?;

            let status = SysCommand::new("git")
                .args(["bundle", "unbundle", bundle_path.to_str().unwrap()])
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .status()
                .context("Failed to run 'git bundle unbundle'")?;

            if !status.success() {
                bail!("'git bundle unbundle' failed with status {:?}", status);
            }
        }

        protocol::write_raw(b"feature done\n")?;
        // The private namespace mirrors the remote exactly, so a remote
        // force-push must be allowed to rewind it. Git still applies its own
        // fast-forward rules when copying into refs/remotes/*.
        protocol::write_raw(b"feature force\n")?;

        for ref_name in refs {
            let Some(private_ref) = private_ref(&self.private_prefix, ref_name) else {
                warn!(
                    "Requested ref '{}' is outside the mapped namespaces",
                    ref_name
                );
                continue;
            };
            if let Some(sha) = store.refs.get(ref_name) {
                protocol::write_raw(format!("reset {}\nfrom {}\n\n", private_ref, sha).as_bytes())?;
            } else {
                warn!("Requested ref '{}' not found in remote store", ref_name);
            }
        }

        protocol::write_raw(b"done\n")?;

        Ok(())
    }

    fn emit_empty_fast_import(&self) -> Result<()> {
        protocol::write_raw(b"feature done\n")?;
        protocol::write_raw(b"done\n")?;
        Ok(())
    }

    // ─── Push ────────────────────────────────────────────────────────────────

    /// Handle a batch of `push <src>:<dst>` commands.
    ///
    /// For each ref being pushed, resolves the local SHA, creates a git bundle
    /// containing only the new commits (excluding anything already on the
    /// remote), uploads it, and updates refs.json.
    async fn handle_push(&mut self, pushes: &[(String, String, bool)]) -> Result<()> {
        info!("push: {} ref(s)", pushes.len());

        let mut store = self.storage.load_refs().await?;
        let mut updated_refs: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut push_details: Vec<(String, String, String, bool)> = Vec::new();

        for (src, dst, force) in pushes {
            if src.is_empty() {
                // Deletion: `push :dst` — not yet supported.
                warn!("push: deletion of '{}' not supported", dst);
                protocol::write_line(&format!("error {} deletion not supported", dst))?;
                continue;
            }

            let sha = self.resolve_ref(src)?;

            // Non-fast-forward check (Issue 4)
            if let Some(old_sha) = store.refs.get(dst) {
                if !*force && !self.is_fast_forward(old_sha, &sha)? {
                    warn!("push: non-fast-forward for '{}' rejected", dst);
                    protocol::write_line(&format!("error {} non-fast-forward", dst))?;
                    continue;
                }
            }

            updated_refs.insert(dst.clone(), sha.clone());
            push_details.push((src.clone(), dst.clone(), sha, *force));
        }

        if updated_refs.is_empty() {
            protocol::write_blank()?;
            return Ok(());
        }

        // Use SRC ref names for bundle creation (Issue 2)
        let src_refs: Vec<&str> = push_details.iter().map(|(s, ..)| s.as_str()).collect();
        // Exclude commits the remote already has so bundles stay incremental.
        let exclusions: Vec<String> = store.refs.values().map(|sha| format!("^{sha}")).collect();

        // A ref pointing at objects the remote already has (e.g. a new branch
        // at an existing commit) needs no bundle, and `git bundle create`
        // refuses to write an empty one.
        if self.has_new_objects(&src_refs, &exclusions)? {
            // Use UUID to avoid collisions (Issue 1)
            let bundle_name = uuid::Uuid::new_v4().to_string();
            let tmp_bundle =
                tempfile::NamedTempFile::new().context("Failed to create temp bundle file")?;

            let status = SysCommand::new("git")
                .arg("bundle")
                .arg("create")
                .arg(tmp_bundle.path())
                .args(&src_refs)
                .args(&exclusions)
                .stderr(Stdio::inherit())
                .status()
                .context("Failed to run 'git bundle create'")?;

            if !status.success() {
                bail!("'git bundle create' failed");
            }

            let bundle_data = std::fs::read(tmp_bundle.path()).context("Failed to read bundle")?;
            // tmp_bundle will be deleted when dropped (Issue 7)

            let bundle_key = self
                .storage
                .upload_bundle(&bundle_name, bundle_data)
                .await?;
            store.bundles.push(bundle_key);
        } else {
            info!("push: remote already has every object, skipping bundle upload");
        }
        for (dst, sha) in &updated_refs {
            store.refs.insert(dst.clone(), sha.clone());
        }
        self.storage.save_refs(&store).await?;

        for dst in updated_refs.keys() {
            protocol::write_line(&format!("ok {}", dst))?;
        }
        protocol::write_blank()?;

        info!("push: {} ref(s) updated", updated_refs.len());
        Ok(())
    }

    /// Whether `revs` reach any object (commit, tree, blob or tag) outside `exclusions`.
    fn has_new_objects(&self, revs: &[&str], exclusions: &[String]) -> Result<bool> {
        let output = SysCommand::new("git")
            .args(["rev-list", "--objects"])
            .args(revs)
            .args(exclusions)
            .stderr(Stdio::inherit())
            .output()
            .context("Failed to run 'git rev-list'")?;

        if !output.status.success() {
            bail!("'git rev-list --objects' failed");
        }

        Ok(!output.stdout.is_empty())
    }

    fn is_fast_forward(&self, old_sha: &str, new_sha: &str) -> Result<bool> {
        let status = SysCommand::new("git")
            .args(["merge-base", "--is-ancestor", old_sha, new_sha])
            .status()
            .context("Failed to run git merge-base")?;

        Ok(status.success())
    }

    fn resolve_ref(&self, refname: &str) -> Result<String> {
        let output = SysCommand::new("git")
            .args(["rev-parse", refname])
            .output()
            .context("Failed to run git rev-parse")?;

        if !output.status.success() {
            bail!("Failed to resolve ref '{}'", refname);
        }

        Ok(String::from_utf8(output.stdout)
            .context("git rev-parse output is not UTF-8")?
            .trim()
            .to_string())
    }

    // ─── Option ──────────────────────────────────────────────────────────────

    fn handle_option(&self, key: &str, val: &str) -> Result<()> {
        debug!("option: {}={}", key, val);
        protocol::write_line(option_response(key))?;
        Ok(())
    }
}

// ─── Utilities ───────────────────────────────────────────────────────────────

/// Build the private namespace for a remote. Git passes the URL itself as the
/// remote name for anonymous remotes, so anything outside `[A-Za-z0-9_-]` is
/// replaced to keep the result a valid ref name.
fn private_prefix(remote_name: &str) -> String {
    let safe: String = remote_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe = if safe.is_empty() {
        "_".to_string()
    } else {
        safe
    };
    format!("refs/opendal/{safe}")
}

/// Map a remote ref (`refs/heads/x`, `refs/tags/x`) into the private namespace.
fn private_ref(prefix: &str, remote_ref: &str) -> Option<String> {
    MAPPED_NAMESPACES.iter().find_map(|ns| {
        remote_ref
            .strip_prefix(&format!("refs/{ns}/"))
            .map(|rest| format!("{prefix}/{ns}/{rest}"))
    })
}

fn option_response(key: &str) -> &'static str {
    match key {
        "verbosity" => "ok",
        "progress" => "unsupported",
        _ => "unsupported",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_option_is_unsupported() {
        assert_eq!(option_response("progress"), "unsupported");
    }

    #[test]
    fn verbosity_option_is_supported() {
        assert_eq!(option_response("verbosity"), "ok");
    }

    #[test]
    fn private_prefix_uses_remote_name() {
        assert_eq!(private_prefix("origin"), "refs/opendal/origin");
    }

    #[test]
    fn private_prefix_sanitizes_url_remote_names() {
        assert_eq!(
            private_prefix("opendal://fs/tmp/my.repo"),
            "refs/opendal/opendal___fs_tmp_my_repo"
        );
        assert_eq!(private_prefix(""), "refs/opendal/_");
    }

    #[test]
    fn private_ref_maps_heads_and_tags() {
        let p = "refs/opendal/origin";
        assert_eq!(
            private_ref(p, "refs/heads/feature/x").as_deref(),
            Some("refs/opendal/origin/heads/feature/x")
        );
        assert_eq!(
            private_ref(p, "refs/tags/v1").as_deref(),
            Some("refs/opendal/origin/tags/v1")
        );
        assert_eq!(private_ref(p, "HEAD"), None);
        assert_eq!(private_ref(p, "refs/notes/x"), None);
    }
}
