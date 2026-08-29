//! Workflow companion for `git-remote-opendal`.
//!
//! This binary deliberately owns setup and diagnostics only.  Git invokes
//! `git-remote-opendal` for the remote-helper protocol and all storage I/O.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitCode};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;

const MIN_GIT: (u32, u32) = (2, 20);

const AFTER_HELP: &str = "\
Examples:
  git opendal setup --backend fs --path /tmp/git-remotes/myrepo --push
  git opendal setup --backend s3 --bucket my-bucket --path repos/myrepo --push
  git opendal doctor --backend s3
  git opendal schema
  git opendal --json doctor --backend s3

The helper is still used by normal Git commands: git fetch, git pull, and git push.";

/// Set up and inspect OpenDAL-backed Git remotes.
///
/// Installed beside git-remote-opendal, this is also available as
/// `git opendal <command>`. It validates prerequisites, builds opendal://
/// URLs, registers remotes, and helps with the first push.
#[derive(Parser)]
#[command(name = "git-opendal", version, after_help = AFTER_HELP)]
struct Cli {
    /// Emit machine-readable JSON output for scripting and AI agents
    #[arg(global = true, long)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check Git, the remote helper, and optional backend credentials
    Doctor {
        /// Check environment variables for this backend
        #[arg(long)]
        backend: Option<Backend>,
        /// Infer the backend from this configured remote
        #[arg(long, conflicts_with = "backend")]
        remote: Option<String>,
    },
    /// Print an opendal:// URL for scripting or manual Git setup
    Url {
        #[command(flatten)]
        target: Target,
    },
    /// Register an OpenDAL remote and optionally publish the current branch
    Setup {
        #[command(flatten)]
        target: Target,
        /// Remote name to create or update
        #[arg(long, default_value = "origin")]
        remote: String,
        /// Push the current branch and set its upstream after setup
        #[arg(long)]
        push: bool,
        /// Replace an existing remote that points somewhere else
        #[arg(long)]
        force: bool,
    },
    /// First push to an OpenDAL remote, setting the upstream branch
    Bootstrap {
        /// Configured remote to push to
        #[arg(default_value = "origin")]
        remote: String,
        /// Branch to push; defaults to the current branch
        branch: Option<String>,
    },
    /// Show an OpenDAL remote's configuration and local filesystem state
    Status {
        /// Configured remote to inspect
        #[arg(default_value = "origin")]
        remote: String,
        /// Query remote refs through the remote helper
        #[arg(long)]
        probe: bool,
    },
    /// Clone an OpenDAL remote after validating its URL and helper
    Clone {
        /// Full opendal:// URL
        url: String,
        /// Destination directory
        directory: Option<String>,
    },
    /// Show backend URL shapes and environment-variable guidance
    Config {
        #[arg(long)]
        backend: Option<Backend>,
    },
    /// Print machine-readable schema for all backends, URL formats, and parameters
    Schema,
}

#[derive(Args)]
struct Target {
    /// Storage backend
    #[arg(long)]
    backend: Backend,
    /// Repository root inside the backend (absolute path for fs)
    #[arg(long)]
    path: String,
    /// Bucket (s3, gcs) or container (azblob)
    #[arg(long)]
    bucket: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize)]
#[value(rename_all = "lower")]
#[serde(rename_all = "lowercase")]
enum Backend {
    Fs,
    S3,
    Gcs,
    Azblob,
    Gdrive,
}

impl Backend {
    const ALL: [Self; 5] = [Self::Fs, Self::S3, Self::Gcs, Self::Azblob, Self::Gdrive];

    fn as_str(self) -> &'static str {
        match self {
            Self::Fs => "fs",
            Self::S3 => "s3",
            Self::Gcs => "gcs",
            Self::Azblob => "azblob",
            Self::Gdrive => "gdrive",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "fs" => Ok(Self::Fs),
            "s3" => Ok(Self::S3),
            "gcs" => Ok(Self::Gcs),
            "azblob" => Ok(Self::Azblob),
            "gdrive" => Ok(Self::Gdrive),
            _ => bail!("unsupported backend '{value}'; use fs, s3, gcs, azblob, or gdrive"),
        }
    }

    fn bucket_label(self) -> Option<&'static str> {
        match self {
            Self::S3 | Self::Gcs => Some("bucket"),
            Self::Azblob => Some("container"),
            Self::Fs | Self::Gdrive => None,
        }
    }

    fn credential_vars(self) -> &'static [&'static str] {
        match self {
            Self::Fs => &[],
            Self::S3 => &["OPENDAL_S3_ACCESS_KEY_ID", "OPENDAL_S3_SECRET_ACCESS_KEY"],
            Self::Gcs => &["OPENDAL_GCS_CREDENTIAL_PATH", "OPENDAL_GCS_CREDENTIAL"],
            Self::Azblob => &["OPENDAL_AZBLOB_ACCOUNT_NAME", "OPENDAL_AZBLOB_ACCOUNT_KEY"],
            Self::Gdrive => &[
                "OPENDAL_GDRIVE_CLIENT_ID",
                "OPENDAL_GDRIVE_CLIENT_SECRET",
                "OPENDAL_GDRIVE_REFRESH_TOKEN",
                "OPENDAL_GDRIVE_ACCESS_TOKEN",
            ],
        }
    }

    fn credential_hint(self) -> Option<&'static str> {
        match self {
            Self::Fs => None,
            Self::S3 => Some(
                "set OPENDAL_S3_ACCESS_KEY_ID, OPENDAL_S3_SECRET_ACCESS_KEY, and usually OPENDAL_S3_REGION",
            ),
            Self::Gcs => Some("set OPENDAL_GCS_CREDENTIAL_PATH (or OPENDAL_GCS_CREDENTIAL)"),
            Self::Azblob => Some("set OPENDAL_AZBLOB_ACCOUNT_NAME and OPENDAL_AZBLOB_ACCOUNT_KEY"),
            Self::Gdrive => {
                Some("set Google Drive OAuth variables, such as OPENDAL_GDRIVE_ACCESS_TOKEN")
            }
        }
    }

    fn credentials_present(self) -> bool {
        let vars = self.credential_vars();
        vars.is_empty() || vars.iter().any(|name| env::var_os(name).is_some())
    }

    fn schema(self) -> BackendSchema {
        let url_format = match self.bucket_label() {
            Some(label) => format!("opendal://{}/<{label}>/<path>", self.as_str()),
            None => format!("opendal://{}/<path>", self.as_str()),
        };
        BackendSchema {
            backend: self.as_str().to_string(),
            url_format,
            bucket_label: self.bucket_label().map(|s| s.to_string()),
            credential_variables: self
                .credential_vars()
                .iter()
                .map(|s| s.to_string())
                .collect(),
            credential_hint: self.credential_hint().map(|s| s.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct RemoteSpec {
    backend: Backend,
    bucket: Option<String>,
    path: String,
}

impl RemoteSpec {
    fn new(backend: Backend, bucket: Option<String>, path: &str) -> Result<Self> {
        let path = path.trim().trim_matches('/').to_string();
        if path.is_empty() {
            bail!("--path must not be empty")
        }
        let bucket = bucket
            .map(|value| value.trim().trim_matches('/').to_string())
            .filter(|value| !value.is_empty());

        match (backend.bucket_label(), bucket.is_some()) {
            (Some(label), false) => {
                bail!("backend '{}' needs --bucket <{label}>", backend.as_str())
            }
            (None, true) => bail!("--bucket is not used with backend '{}'", backend.as_str()),
            _ => Ok(Self {
                backend,
                bucket,
                path,
            }),
        }
    }

    fn from_target(target: &Target) -> Result<Self> {
        Self::new(target.backend, target.bucket.clone(), &target.path)
    }

    fn parse_url(url: &str) -> Result<Self> {
        let original = url.trim();
        let rest = original.strip_prefix("opendal://").ok_or_else(|| {
            anyhow::anyhow!(
                "'{original}' is not an opendal URL; expected opendal://<backend>/<path>"
            )
        })?;
        let (scheme, remainder) = rest
            .split_once('/')
            .ok_or_else(|| anyhow::anyhow!("'{original}' is missing a path after the backend"))?;
        let backend = Backend::parse(scheme)?;
        if backend.bucket_label().is_some() {
            let (bucket, path) = remainder.split_once('/').ok_or_else(|| {
                anyhow::anyhow!(
                    "'{original}' is missing a repository path after its bucket or container"
                )
            })?;
            Self::new(backend, Some(bucket.to_owned()), path)
        } else {
            Self::new(backend, None, remainder)
        }
    }

    fn url(&self) -> String {
        match &self.bucket {
            Some(bucket) => format!("opendal://{}/{bucket}/{}", self.backend.as_str(), self.path),
            None => format!("opendal://{}/{}", self.backend.as_str(), self.path),
        }
    }

    fn local_fs_path(&self) -> Option<PathBuf> {
        (self.backend == Backend::Fs).then(|| PathBuf::from("/").join(&self.path))
    }
}

// ─── JSON Data Transfer Objects ──────────────────────────────────────────────

#[derive(Serialize)]
struct ErrorOutput {
    success: bool,
    error: String,
}

#[derive(Serialize)]
struct DoctorOutput {
    status: &'static str,
    git: GitCheck,
    helper: HelperCheck,
    backend: Option<String>,
    credentials: Option<CredentialsCheck>,
}

#[derive(Serialize)]
struct GitCheck {
    ok: bool,
    version: Option<String>,
    error: Option<String>,
}

#[derive(Serialize)]
struct HelperCheck {
    ok: bool,
    found: bool,
}

#[derive(Serialize)]
struct CredentialsCheck {
    ok: bool,
    variables: Vec<EnvVarStatus>,
    hint: Option<String>,
}

#[derive(Serialize)]
struct EnvVarStatus {
    name: String,
    set: bool,
}

#[derive(Serialize)]
struct StatusOutput {
    remote: String,
    url: String,
    backend: String,
    bucket: Option<String>,
    path: String,
    storage: Option<StorageStatus>,
    refs: Option<Vec<RefEntry>>,
}

#[derive(Serialize)]
struct StorageStatus {
    state: String,
    bundles: usize,
}

#[derive(Serialize)]
struct RefEntry {
    sha: String,
    name: String,
}

#[derive(Serialize)]
struct SetupOutput {
    success: bool,
    remote: String,
    url: String,
    backend: String,
    bucket: Option<String>,
    path: String,
    pushed: bool,
    branch: Option<String>,
}

#[derive(Serialize)]
struct BootstrapOutput {
    success: bool,
    remote: String,
    branch: String,
    url: String,
}

#[derive(Serialize)]
struct UrlOutput {
    url: String,
    backend: String,
    bucket: Option<String>,
    path: String,
}

#[derive(Serialize)]
struct BackendSchema {
    backend: String,
    url_format: String,
    bucket_label: Option<String>,
    credential_variables: Vec<String>,
    credential_hint: Option<String>,
}

#[derive(Serialize)]
struct SchemaOutput {
    version: &'static str,
    supported_backends: Vec<&'static str>,
    backends: Vec<BackendSchema>,
}

// ─── Entry Point ─────────────────────────────────────────────────────────────

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if json {
                let err_obj = ErrorOutput {
                    success: false,
                    error: format!("{error:#}"),
                };
                if let Ok(json_str) = serde_json::to_string_pretty(&err_obj) {
                    eprintln!("{json_str}");
                } else {
                    eprintln!("{{\"success\":false,\"error\":\"{error:#}\"}}");
                }
            } else {
                eprintln!("error: {error:#}");
            }
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let json = cli.json;
    match cli.command {
        Command::Doctor { backend, remote } => {
            let backend = match (backend, remote) {
                (Some(backend), _) => Some(backend),
                (None, Some(remote)) => Some(RemoteSpec::parse_url(&remote_url(&remote)?)?.backend),
                (None, None) => None,
            };
            doctor(backend, json)
        }
        Command::Url { target } => {
            let spec = RemoteSpec::from_target(&target)?;
            if json {
                let out = UrlOutput {
                    url: spec.url(),
                    backend: spec.backend.as_str().to_string(),
                    bucket: spec.bucket.clone(),
                    path: spec.path.clone(),
                };
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                println!("{}", spec.url());
            }
            Ok(())
        }
        Command::Setup {
            target,
            remote,
            push,
            force,
        } => setup(
            RemoteSpec::from_target(&target)?,
            &remote,
            push,
            force,
            json,
        ),
        Command::Bootstrap { remote, branch } => {
            bootstrap(&remote, branch.as_deref(), json)?;
            Ok(())
        }
        Command::Status { remote, probe } => status(&remote, probe, json),
        Command::Clone { url, directory } => clone(&url, directory.as_deref(), json),
        Command::Config { backend } => config(backend, json),
        Command::Schema => schema(json),
    }
}

fn schema(json: bool) -> Result<()> {
    let schema_data = SchemaOutput {
        version: env!("CARGO_PKG_VERSION"),
        supported_backends: Backend::ALL.iter().map(|b| b.as_str()).collect(),
        backends: Backend::ALL.iter().map(|b| b.schema()).collect(),
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&schema_data)?);
    } else {
        println!("git-opendal v{}", schema_data.version);
        println!(
            "Supported backends: {}",
            schema_data.supported_backends.join(", ")
        );
        println!();
        for b in &schema_data.backends {
            println!("{}: {}", b.backend, b.url_format);
            if let Some(ref hint) = b.credential_hint {
                println!("  {hint}");
            }
        }
    }
    Ok(())
}

fn doctor(backend: Option<Backend>, json: bool) -> Result<()> {
    let mut failed = false;

    let git_check = match git_capture(&["--version"]) {
        Ok(version) if git_version_at_least(&version, MIN_GIT) => GitCheck {
            ok: true,
            version: Some(version),
            error: None,
        },
        Ok(version) => {
            failed = true;
            GitCheck {
                ok: false,
                version: Some(version),
                error: Some(format!("git >= {}.{} is required", MIN_GIT.0, MIN_GIT.1)),
            }
        }
        Err(error) => {
            failed = true;
            GitCheck {
                ok: false,
                version: None,
                error: Some(error.to_string()),
            }
        }
    };

    let helper_ok = helper_installed();
    if !helper_ok {
        failed = true;
    }
    let helper_check = HelperCheck {
        ok: helper_ok,
        found: helper_ok,
    };

    let creds_check = backend.map(|b| {
        let vars: Vec<EnvVarStatus> = b
            .credential_vars()
            .iter()
            .map(|&name| EnvVarStatus {
                name: name.to_string(),
                set: env::var_os(name).is_some(),
            })
            .collect();
        let present = b.credentials_present();
        CredentialsCheck {
            ok: present,
            variables: vars,
            hint: if !present {
                b.credential_hint().map(|s| s.to_string())
            } else {
                None
            },
        }
    });

    if json {
        let out = DoctorOutput {
            status: if failed { "fail" } else { "ok" },
            git: git_check,
            helper: helper_check,
            backend: backend.map(|b| b.as_str().to_string()),
            credentials: creds_check,
        };
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        if git_check.ok {
            println!("ok   git: {}", git_check.version.as_deref().unwrap_or(""));
        } else if let Some(ref err) = git_check.error {
            println!("fail git: {err}");
        }

        if helper_check.ok {
            println!("ok   git-remote-opendal: found on PATH");
        } else {
            println!("fail git-remote-opendal: not found on PATH");
            println!("     install this package with: cargo install --locked git-opendal");
        }

        if let Some(ref creds) = creds_check {
            for v in &creds.variables {
                let state = if v.set { "set" } else { "not set" };
                println!("info {}: {state}", v.name);
            }
            if !creds.ok
                && let Some(ref hint) = creds.hint
            {
                println!(
                    "warn {} credentials: no credential variables detected",
                    backend.map(|b| b.as_str()).unwrap_or("")
                );
                println!("     {hint}");
            }
        } else {
            println!("info add --backend <backend> to check credential variables");
        }
    }

    if failed {
        bail!("environment checks failed")
    } else {
        Ok(())
    }
}

fn setup(spec: RemoteSpec, remote: &str, push: bool, force: bool, json: bool) -> Result<()> {
    require_work_tree()?;
    // For non-json, run doctor checks visually. For json, verify silently.
    if !json {
        doctor(Some(spec.backend), false)?;
    } else {
        if !helper_installed() {
            bail!("git-remote-opendal is not found on PATH; install this package first");
        }
    }

    if let Some(path) = spec.local_fs_path() {
        fs::create_dir_all(&path)
            .with_context(|| format!("create local storage directory {}", path.display()))?;
    }
    register_remote(remote, &spec.url(), force, json)?;

    let mut pushed_branch = None;
    if push {
        let branch = bootstrap(remote, None, json)?;
        pushed_branch = Some(branch);
    }

    if json {
        let out = SetupOutput {
            success: true,
            remote: remote.to_string(),
            url: spec.url(),
            backend: spec.backend.as_str().to_string(),
            bucket: spec.bucket.clone(),
            path: spec.path.clone(),
            pushed: push,
            branch: pushed_branch,
        };
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("configured '{remote}' -> {}", spec.url());
    }
    Ok(())
}

fn register_remote(name: &str, url: &str, force: bool, json: bool) -> Result<()> {
    if let Ok(existing) = git_capture(&["remote", "get-url", name]) {
        if existing == url {
            if !json {
                println!("remote '{name}' already points at {url}");
            }
            return Ok(());
        }
        if !force {
            bail!("remote '{name}' already points at {existing}; pass --force to replace it")
        }
        git_capture(&["remote", "set-url", name, url])?;
    } else {
        git_capture(&["remote", "add", name, url])?;
    }
    Ok(())
}

fn bootstrap(remote: &str, branch: Option<&str>, json: bool) -> Result<String> {
    require_work_tree()?;
    let url = remote_url(remote)?;
    let spec = RemoteSpec::parse_url(&url).context("bootstrap requires an opendal:// remote")?;
    if !helper_installed() {
        bail!("git-remote-opendal is not found on PATH; install this package first")
    }
    if git_capture(&["rev-parse", "--verify", "-q", "HEAD"]).is_err() {
        bail!("this repository has no commits yet; make an initial commit first")
    }
    let branch = match branch {
        Some(branch) => branch.to_owned(),
        None => git_capture(&["branch", "--show-current"])
            .context("could not detect current branch; pass it explicitly")?,
    };
    if branch.is_empty() {
        bail!("current branch is empty; pass a branch explicitly")
    }
    if !json {
        warn_missing_credentials(spec.backend);
    }
    git_passthrough(&["push", "-u", remote, &branch])?;

    if json {
        let out = BootstrapOutput {
            success: true,
            remote: remote.to_string(),
            branch: branch.clone(),
            url,
        };
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("published '{branch}' to '{remote}'");
    }
    Ok(branch)
}

fn status(remote: &str, probe: bool, json: bool) -> Result<()> {
    require_work_tree()?;
    let url = remote_url(remote)?;
    let spec = RemoteSpec::parse_url(&url).context("configured remote is not an opendal:// URL")?;

    let mut storage_status = None;
    if let Some(root) = spec.local_fs_path() {
        let refs = root.join("info/refs.json");
        let bundles = fs::read_dir(root.join("objects"))
            .map(|entries| entries.count())
            .unwrap_or(0);
        let state = if !root.is_dir() {
            "directory missing"
        } else if !refs.is_file() {
            "empty (nothing pushed yet)"
        } else {
            "initialized"
        };
        storage_status = Some(StorageStatus {
            state: state.to_string(),
            bundles,
        });
    }

    let mut probed_refs = None;
    if probe {
        let refs_raw = git_capture(&["ls-remote", remote])?;
        let entries: Vec<RefEntry> = refs_raw
            .lines()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let sha = parts.next()?.to_string();
                let name = parts.next()?.to_string();
                Some(RefEntry { sha, name })
            })
            .collect();
        probed_refs = Some(entries);
    }

    if json {
        let out = StatusOutput {
            remote: remote.to_string(),
            url,
            backend: spec.backend.as_str().to_string(),
            bucket: spec.bucket.clone(),
            path: spec.path.clone(),
            storage: storage_status,
            refs: probed_refs,
        };
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("remote:  {remote}\nurl:     {url}");
        println!("backend: {}", spec.backend.as_str());
        if let Some(bucket) = &spec.bucket {
            println!("{}:  {bucket}", spec.backend.bucket_label().unwrap());
        }
        println!("path:    {}", spec.path);
        if let Some(ref storage) = storage_status {
            println!("storage: {} ({} bundle(s))", storage.state, storage.bundles);
        }
        if probe {
            println!("refs:");
            if let Some(ref refs) = probed_refs {
                if refs.is_empty() {
                    println!("  remote is empty");
                } else {
                    for r in refs {
                        println!("  {} {}", r.sha, r.name);
                    }
                }
            }
        }
    }
    Ok(())
}

fn clone(url: &str, directory: Option<&str>, json: bool) -> Result<()> {
    let spec = RemoteSpec::parse_url(url)?;
    if !helper_installed() {
        bail!("git-remote-opendal is not found on PATH; install this package first")
    }
    if !json {
        warn_missing_credentials(spec.backend);
    }
    match directory {
        Some(directory) => git_passthrough(&["clone", &spec.url(), directory]),
        None => git_passthrough(&["clone", &spec.url()]),
    }
}

fn config(backend: Option<Backend>, json: bool) -> Result<()> {
    let backends = backend.map_or_else(|| Backend::ALL.to_vec(), |backend| vec![backend]);
    let schemas: Vec<BackendSchema> = backends.iter().map(|b| b.schema()).collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&schemas)?);
    } else {
        for s in &schemas {
            println!("{}: {}", s.backend, s.url_format);
            if let Some(ref hint) = s.credential_hint {
                println!("  {hint}");
            }
        }
    }
    Ok(())
}

fn remote_url(remote: &str) -> Result<String> {
    git_capture(&["remote", "get-url", remote])
        .with_context(|| format!("remote '{remote}' is not configured"))
}

fn require_work_tree() -> Result<()> {
    if matches!(
        git_capture(&["rev-parse", "--is-inside-work-tree"]),
        Ok(value) if value == "true"
    ) {
        Ok(())
    } else {
        bail!("not inside a Git work tree; run this from the repository you want to publish")
    }
}

fn helper_installed() -> bool {
    env::var_os("PATH").is_some_and(|paths| {
        env::split_paths(&paths).any(|dir| executable_file(&dir.join("git-remote-opendal")))
    })
}

fn executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn git_capture(args: &[&str]) -> Result<String> {
    let output = ProcessCommand::new("git")
        .args(args)
        .output()
        .context("run git")?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
}

fn git_passthrough(args: &[&str]) -> Result<()> {
    let status = ProcessCommand::new("git")
        .args(args)
        .status()
        .context("run git")?;
    if status.success() {
        Ok(())
    } else {
        bail!("git {} failed with {status}", args.join(" "))
    }
}

fn git_version_at_least(line: &str, minimum: (u32, u32)) -> bool {
    line.split_whitespace()
        .find_map(|word| {
            let mut parts = word.split('.');
            Some((
                parts.next()?.parse::<u32>().ok()?,
                parts.next()?.parse::<u32>().ok()?,
            ))
        })
        .is_some_and(|version| version >= minimum)
}

fn warn_missing_credentials(backend: Backend) {
    if !backend.credentials_present()
        && let Some(hint) = backend.credential_hint()
    {
        eprintln!(
            "warning: no {} credentials detected; {hint}",
            backend.as_str()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_and_parses_bucketed_urls() {
        let spec = RemoteSpec::new(Backend::S3, Some("bucket".into()), "repos/example").unwrap();
        assert_eq!(spec.url(), "opendal://s3/bucket/repos/example");
        assert_eq!(RemoteSpec::parse_url(&spec.url()).unwrap(), spec);
    }

    #[test]
    fn fs_urls_keep_absolute_paths() {
        let spec = RemoteSpec::new(Backend::Fs, None, "/tmp/repos/example").unwrap();
        assert_eq!(spec.url(), "opendal://fs/tmp/repos/example");
        assert_eq!(
            spec.local_fs_path(),
            Some(PathBuf::from("/tmp/repos/example"))
        );
    }

    #[test]
    fn bucketed_backends_require_a_bucket() {
        assert!(RemoteSpec::new(Backend::Gcs, None, "repos/example").is_err());
    }

    #[test]
    fn schema_output_contains_all_backends() {
        let schema_data = SchemaOutput {
            version: env!("CARGO_PKG_VERSION"),
            supported_backends: Backend::ALL.iter().map(|b| b.as_str()).collect(),
            backends: Backend::ALL.iter().map(|b| b.schema()).collect(),
        };
        assert_eq!(schema_data.supported_backends.len(), 5);
        assert!(schema_data.supported_backends.contains(&"s3"));
        assert!(schema_data.supported_backends.contains(&"fs"));
    }
}
