//! End-to-end tests: real `git` driving the built `git-remote-opendal` binary
//! against the local `fs` backend.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// An isolated sandbox: its own HOME, no system git config, and the freshly
/// built helper first on PATH.
struct Sandbox {
    dir: tempfile::TempDir,
    path_env: String,
}

impl Sandbox {
    fn new() -> Self {
        let bin_dir = Path::new(env!("CARGO_BIN_EXE_git-remote-opendal"))
            .parent()
            .unwrap()
            .to_path_buf();
        let path_env = format!(
            "{}:{}",
            bin_dir.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Self {
            dir: tempfile::tempdir().unwrap(),
            path_env,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn remote_url(&self) -> String {
        format!("opendal://fs{}", self.path("remote").display())
    }

    fn git(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("PATH", &self.path_env)
            .env("HOME", self.dir.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .env("GIT_REMOTE_OPENDAL_LOG", "error")
            .output()
            .expect("failed to run git")
    }

    /// Run git and panic with its stderr if it fails.
    fn ok(&self, cwd: &Path, args: &[&str]) -> String {
        let out = self.git(cwd, args);
        assert!(
            out.status.success(),
            "git {:?} failed ({:?}):\n{}",
            args,
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Create a repo with one commit on `main` and an `origin` remote.
    fn init_repo(&self, name: &str) -> PathBuf {
        let repo = self.path(name);
        std::fs::create_dir(&repo).unwrap();
        self.ok(&repo, &["init", "-q"]);
        self.ok(&repo, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        self.commit(&repo, "f", "c1");
        self.ok(&repo, &["remote", "add", "origin", &self.remote_url()]);
        repo
    }

    fn clone(&self, name: &str) -> PathBuf {
        let target = self.path(name);
        self.ok(
            self.dir.path(),
            &["clone", "-q", &self.remote_url(), target.to_str().unwrap()],
        );
        target
    }

    fn commit(&self, repo: &Path, file: &str, msg: &str) {
        let path = repo.join(file);
        let mut body = std::fs::read_to_string(&path).unwrap_or_default();
        body.push_str(msg);
        body.push('\n');
        std::fs::write(&path, body).unwrap();
        self.ok(repo, &["add", file]);
        self.ok(repo, &["commit", "-q", "-m", msg]);
    }

    fn rev(&self, repo: &Path, rev: &str) -> String {
        self.ok(repo, &["rev-parse", rev])
    }
}

#[test]
fn clone_repo_with_tags() {
    let sb = Sandbox::new();
    let a = sb.init_repo("a");
    sb.ok(&a, &["tag", "light"]);
    sb.ok(&a, &["tag", "-a", "annotated", "-m", "annotated"]);
    sb.ok(&a, &["push", "-q", "origin", "main", "light", "annotated"]);

    let b = sb.clone("b");

    assert_eq!(sb.rev(&b, "HEAD"), sb.rev(&a, "main"));
    assert_eq!(sb.ok(&b, &["status", "--porcelain"]), "");
    let tags = sb.ok(&b, &["tag"]);
    assert!(
        tags.contains("light") && tags.contains("annotated"),
        "{tags}"
    );
}

#[test]
fn import_writes_only_the_private_namespace() {
    let sb = Sandbox::new();
    let a = sb.init_repo("a");
    sb.ok(&a, &["push", "-q", "origin", "main"]);
    let b = sb.clone("b");

    assert_eq!(
        sb.rev(&b, "refs/opendal/origin/heads/main"),
        sb.rev(&a, "main")
    );
}

#[test]
fn fetch_does_not_move_checked_out_branch() {
    let sb = Sandbox::new();
    let a = sb.init_repo("a");
    sb.ok(&a, &["push", "-q", "origin", "main"]);
    let b = sb.clone("b");
    let before = sb.rev(&b, "main");

    sb.commit(&a, "f", "c2");
    sb.ok(&a, &["push", "-q", "origin", "main"]);
    sb.ok(&b, &["fetch", "-q", "origin"]);

    assert_eq!(
        sb.rev(&b, "main"),
        before,
        "fetch must not touch local main"
    );
    assert_eq!(sb.rev(&b, "origin/main"), sb.rev(&a, "main"));
    assert_eq!(sb.ok(&b, &["status", "--porcelain"]), "");
}

#[test]
fn fetch_and_pull_with_unpushed_local_commit() {
    let sb = Sandbox::new();
    let a = sb.init_repo("a");
    sb.ok(&a, &["push", "-q", "origin", "main"]);
    let b = sb.clone("b");

    sb.commit(&a, "f", "remote-change");
    sb.ok(&a, &["push", "-q", "origin", "main"]);
    sb.commit(&b, "g", "local-change");
    let local = sb.rev(&b, "main");

    sb.ok(&b, &["fetch", "-q", "origin"]);
    assert_eq!(sb.rev(&b, "main"), local);

    sb.ok(&b, &["pull", "-q", "--rebase", "origin", "main"]);
    sb.ok(
        &b,
        &["merge-base", "--is-ancestor", &sb.rev(&a, "main"), "main"],
    );
}

#[test]
fn fetch_after_remote_force_push() {
    let sb = Sandbox::new();
    let a = sb.init_repo("a");
    sb.commit(&a, "f", "c2");
    sb.ok(&a, &["push", "-q", "origin", "main"]);
    let b = sb.clone("b");

    sb.ok(&a, &["reset", "-q", "--hard", "HEAD~1"]);
    sb.commit(&a, "f", "c2-rewritten");
    sb.ok(&a, &["push", "-q", "--force", "origin", "main"]);
    sb.ok(&b, &["fetch", "-q", "origin"]);

    assert_eq!(sb.rev(&b, "origin/main"), sb.rev(&a, "main"));
}

#[test]
fn push_branch_at_existing_commit() {
    let sb = Sandbox::new();
    let a = sb.init_repo("a");
    sb.ok(&a, &["push", "-q", "origin", "main"]);
    let bundles_before = std::fs::read_dir(sb.path("remote/objects"))
        .unwrap()
        .count();

    sb.ok(&a, &["branch", "feat"]);
    sb.ok(&a, &["push", "-q", "origin", "feat"]);

    let bundles_after = std::fs::read_dir(sb.path("remote/objects"))
        .unwrap()
        .count();
    assert_eq!(bundles_after, bundles_before, "no empty bundle uploaded");
    let b = sb.clone("b");
    assert_eq!(sb.rev(&b, "origin/feat"), sb.rev(&a, "main"));
}

#[test]
fn push_lightweight_tag_at_existing_commit() {
    let sb = Sandbox::new();
    let a = sb.init_repo("a");
    sb.ok(&a, &["push", "-q", "origin", "main"]);

    sb.ok(&a, &["tag", "v1"]);
    sb.ok(&a, &["push", "-q", "origin", "v1"]);

    let b = sb.clone("b");
    assert_eq!(sb.rev(&b, "v1"), sb.rev(&a, "v1"));
}

#[test]
fn push_annotated_tag_at_existing_commit_uploads_tag_object() {
    let sb = Sandbox::new();
    let a = sb.init_repo("a");
    sb.ok(&a, &["push", "-q", "origin", "main"]);
    let bundles_before = std::fs::read_dir(sb.path("remote/objects"))
        .unwrap()
        .count();

    sb.ok(&a, &["tag", "-a", "v1", "-m", "v1"]);
    sb.ok(&a, &["push", "-q", "origin", "v1"]);

    let bundles_after = std::fs::read_dir(sb.path("remote/objects"))
        .unwrap()
        .count();
    assert_eq!(
        bundles_after,
        bundles_before + 1,
        "tag object must be uploaded"
    );
}
