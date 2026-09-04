//! Credential resolution for git-remote-opendal.
//!
//! Parameter resolution order (highest priority first):
//! 1. Environment variables (`OPENDAL_<SCHEME>_<KEY>`) — already in `cfg.params`
//! 2. Local `.git/config` (`opendal.<scheme>.<key>`)
//! 3. Interactive prompt via `/dev/tty`  →  saves result back to `.git/config`
//!
//! The `.git/config` section format is:
//! ```ini
//! [opendal "s3"]
//!     bucket = my-bucket
//!     secret-access-key = wJalrXUtn...
//! ```
//!
//! Sensitive values (secret keys, tokens) are saved with a warning message.
//! In non-TTY environments (CI), prompting fails with a clear error pointing to env vars.

use crate::config::RemoteConfig;
use anyhow::{Context, Result, bail};
use tracing::debug;

// ─── ParamSpec ────────────────────────────────────────────────────────────────

pub struct ParamSpec {
    /// Lowercase-hyphenated key matching `cfg.params` keys, e.g. `"access-key-id"`.
    pub key: &'static str,
    /// Human-readable label shown in the prompt.
    pub description: &'static str,
    /// If true, an empty/absent value is a fatal error.
    pub required: bool,
    /// If true, echo is masked during input and a warning is printed before saving.
    pub sensitive: bool,
    /// If false, the param is silently skipped when absent (backend uses default auth).
    pub prompt: bool,
    /// Optional input validator. Returns an error message string if the value is invalid;
    /// the prompt is then shown again. `None` means no validation.
    pub validator: Option<fn(&str) -> Option<&'static str>>,
}

fn validate_url(v: &str) -> Option<&'static str> {
    if v.starts_with("http://") || v.starts_with("https://") {
        None
    } else {
        Some("Must start with http:// or https://")
    }
}

// ─── Per-scheme specs ─────────────────────────────────────────────────────────

pub fn specs_for_scheme(scheme: &str) -> &'static [ParamSpec] {
    match scheme {
        "s3" => S3_SPECS,
        "gcs" => GCS_SPECS,
        "azblob" => AZBLOB_SPECS,
        "gdrive" => GDRIVE_SPECS,
        "webdav" => WEBDAV_SPECS,
        _ => &[],
    }
}

static S3_SPECS: &[ParamSpec] = &[
    ParamSpec {
        key: "bucket",
        description: "S3 bucket name",
        required: true,
        sensitive: false,
        prompt: true,
        validator: None,
    },
    ParamSpec {
        key: "region",
        description: "AWS region (e.g. us-east-1)",
        required: false,
        sensitive: false,
        prompt: true,
        validator: None,
    },
    ParamSpec {
        key: "endpoint",
        description: "Custom endpoint URL (blank to skip)",
        required: false,
        sensitive: false,
        prompt: true,
        validator: Some(validate_url),
    },
    ParamSpec {
        key: "access-key-id",
        description: "AWS access key ID (blank for IAM auth)",
        required: false,
        sensitive: false,
        prompt: true,
        validator: None,
    },
    ParamSpec {
        key: "secret-access-key",
        description: "AWS secret access key",
        required: false,
        sensitive: true,
        prompt: true,
        validator: None,
    },
];

static GCS_SPECS: &[ParamSpec] = &[
    ParamSpec {
        key: "bucket",
        description: "GCS bucket name",
        required: true,
        sensitive: false,
        prompt: true,
        validator: None,
    },
    ParamSpec {
        key: "credential-path",
        description: "Service account JSON path (blank for ADC)",
        required: false,
        sensitive: false,
        prompt: true,
        validator: None,
    },
    // credential (raw JSON) is intentionally not prompted — use credential-path instead
    ParamSpec {
        key: "credential",
        description: "Raw GCS credential JSON",
        required: false,
        sensitive: true,
        prompt: false,
        validator: None,
    },
    ParamSpec {
        key: "endpoint",
        description: "Custom GCS endpoint URL (blank to skip)",
        required: false,
        sensitive: false,
        prompt: true,
        validator: Some(validate_url),
    },
];

static AZBLOB_SPECS: &[ParamSpec] = &[
    ParamSpec {
        key: "container",
        description: "Azure Blob container name",
        required: true,
        sensitive: false,
        prompt: true,
        validator: None,
    },
    ParamSpec {
        key: "account-name",
        description: "Azure storage account name",
        required: false,
        sensitive: false,
        prompt: true,
        validator: None,
    },
    ParamSpec {
        key: "account-key",
        description: "Azure storage account key",
        required: false,
        sensitive: true,
        prompt: true,
        validator: None,
    },
    ParamSpec {
        key: "endpoint",
        description: "Custom Azure endpoint URL (blank to skip)",
        required: false,
        sensitive: false,
        prompt: true,
        validator: Some(validate_url),
    },
];

static GDRIVE_SPECS: &[ParamSpec] = &[
    ParamSpec {
        key: "client-id",
        description: "OAuth2 client ID",
        required: false,
        sensitive: false,
        prompt: true,
        validator: None,
    },
    ParamSpec {
        key: "client-secret",
        description: "OAuth2 client secret",
        required: false,
        sensitive: true,
        prompt: true,
        validator: None,
    },
    ParamSpec {
        key: "refresh-token",
        description: "OAuth2 refresh token",
        required: false,
        sensitive: true,
        prompt: true,
        validator: None,
    },
    // access-token expires quickly; not worth prompting — use refresh-token instead
    ParamSpec {
        key: "access-token",
        description: "OAuth2 access token (temporary)",
        required: false,
        sensitive: true,
        prompt: false,
        validator: None,
    },
];

static WEBDAV_SPECS: &[ParamSpec] = &[
    ParamSpec {
        key: "endpoint",
        description: "WebDAV server URL (e.g. https://cloud.example.com)",
        required: true,
        sensitive: false,
        prompt: true,
        validator: Some(validate_url),
    },
    ParamSpec {
        key: "username",
        description: "WebDAV username (blank for anonymous)",
        required: false,
        sensitive: false,
        prompt: true,
        validator: None,
    },
    ParamSpec {
        key: "password",
        description: "WebDAV password or app token",
        required: false,
        sensitive: true,
        prompt: true,
        validator: None,
    },
];

// ─── Public entry point ───────────────────────────────────────────────────────

/// Fill any missing params in `cfg` by reading `.git/config` then prompting the user.
///
/// Call this before `build_operator()` and before `helper.run()` locks stdin.
pub fn resolve(cfg: &mut RemoteConfig) -> Result<()> {
    let specs = specs_for_scheme(&cfg.scheme);
    if specs.is_empty() {
        return Ok(());
    }

    let git_dir = std::env::var("GIT_DIR").ok().filter(|s| !s.is_empty());
    debug!(
        "credentials::resolve scheme={} git_dir={:?}",
        cfg.scheme, git_dir
    );

    for spec in specs {
        // 1. Env var already populated this key — highest priority, skip.
        if cfg.params.contains_key(spec.key) {
            debug!("param '{}' already set via env var", spec.key);
            continue;
        }

        // 2. Try .git/config.
        //    An empty string means "user explicitly skipped" — don't prompt again.
        if let Some(value) = git_config_get(&git_dir, &cfg.scheme, spec.key) {
            if !value.is_empty() {
                debug!("param '{}' loaded from .git/config", spec.key);
                cfg.params.insert(spec.key.to_string(), value);
            } else {
                debug!(
                    "param '{}' was previously skipped (empty in .git/config)",
                    spec.key
                );
            }
            continue;
        }

        // 3. Not prompted — leave absent so the backend uses its default auth.
        if !spec.prompt {
            continue;
        }

        // 4. Interactive prompt via /dev/tty.
        let value = prompt_value(spec)?;

        match value {
            None if spec.required => bail!(
                "Required parameter '{}' was not provided.\n\
                 Set OPENDAL_{}_{} or re-run and enter a value.",
                spec.key,
                cfg.scheme.to_uppercase(),
                spec.key.to_uppercase().replace('-', "_")
            ),
            None => {
                debug!("optional param '{}' skipped by user", spec.key);
                // Save empty string so we don't ask again on the next run.
                let _ = git_config_set(&git_dir, &cfg.scheme, spec.key, "");
                continue;
            }
            Some(v) => {
                if spec.sensitive {
                    eprintln!(
                        "Warning: '{}' will be saved as plain text in .git/config. \
                         Use the OPENDAL_{}_{} env var to avoid this.",
                        spec.key,
                        cfg.scheme.to_uppercase(),
                        spec.key.to_uppercase().replace('-', "_")
                    );
                }
                cfg.params.insert(spec.key.to_string(), v.clone());
                if let Err(e) = git_config_set(&git_dir, &cfg.scheme, spec.key, &v) {
                    eprintln!(
                        "Warning: could not save '{}' to .git/config: {}",
                        spec.key, e
                    );
                }
            }
        }
    }

    Ok(())
}

// ─── resolve_with_injector (test seam) ───────────────────────────────────────

/// Like `resolve`, but uses `injector` instead of opening `/dev/tty`.
///
/// The injector receives the `ParamSpec` and returns `Ok(Some(value))`, `Ok(None)`
/// (blank / skip), or `Err` (abort).  Used in unit tests to avoid a real terminal.
#[cfg(test)]
pub fn resolve_with_injector<F>(cfg: &mut RemoteConfig, mut injector: F) -> Result<()>
where
    F: FnMut(&ParamSpec) -> Result<Option<String>>,
{
    let specs = specs_for_scheme(&cfg.scheme);
    if specs.is_empty() {
        return Ok(());
    }

    let git_dir = std::env::var("GIT_DIR").ok().filter(|s| !s.is_empty());

    for spec in specs {
        if cfg.params.contains_key(spec.key) {
            continue;
        }
        if let Some(value) = git_config_get(&git_dir, &cfg.scheme, spec.key) {
            if !value.is_empty() {
                cfg.params.insert(spec.key.to_string(), value);
            }
            continue;
        }
        if !spec.prompt {
            continue;
        }

        let value = injector(spec)?;

        match value {
            None if spec.required => bail!("Required parameter '{}' was not provided.", spec.key),
            None => {
                let _ = git_config_set(&git_dir, &cfg.scheme, spec.key, "");
                continue;
            }
            Some(v) => {
                cfg.params.insert(spec.key.to_string(), v.clone());
                let _ = git_config_set(&git_dir, &cfg.scheme, spec.key, &v);
            }
        }
    }

    Ok(())
}

// ─── Git config helpers ───────────────────────────────────────────────────────

fn git_config_get(git_dir: &Option<String>, scheme: &str, key: &str) -> Option<String> {
    let config_key = format!("opendal.{}.{}", scheme, key);
    let mut cmd = std::process::Command::new("git");
    cmd.args(["config", "--local", "--get", &config_key]);
    if let Some(dir) = git_dir {
        cmd.env("GIT_DIR", dir);
    }
    let output = cmd.output().ok()?;
    if output.status.success() {
        // Empty string = user explicitly skipped this param. Distinguish from "key not found".
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn git_config_set(git_dir: &Option<String>, scheme: &str, key: &str, value: &str) -> Result<()> {
    let config_key = format!("opendal.{}.{}", scheme, key);
    let mut cmd = std::process::Command::new("git");
    cmd.args(["config", "--local", &config_key, value]);
    if let Some(dir) = git_dir {
        cmd.env("GIT_DIR", dir);
    }
    let status = cmd.status().context("failed to run `git config`")?;
    if !status.success() {
        bail!("`git config --local {}` failed", config_key);
    }
    Ok(())
}

// ─── Terminal prompting ───────────────────────────────────────────────────────

fn prompt_value(spec: &ParamSpec) -> Result<Option<String>> {
    loop {
        let raw = if spec.sensitive {
            prompt_secret(spec)?
        } else {
            prompt_plain(spec)?
        };

        match raw {
            None => return Ok(None), // blank = skip, no validation needed
            Some(ref v) => {
                if let Some(validator) = spec.validator {
                    if let Some(err_msg) = validator(v) {
                        if let Ok(mut tty) = open_tty_write(spec.key) {
                            use std::io::Write;
                            let _ = writeln!(tty, "  Error: {err_msg}");
                        }
                        continue; // re-prompt
                    }
                }
                return Ok(raw);
            }
        }
    }
}

fn prompt_plain(spec: &ParamSpec) -> Result<Option<String>> {
    use std::io::{BufRead, Write};

    let mut tty_out = open_tty_write(spec.key)?;
    if spec.required {
        write!(tty_out, "{}: ", spec.description)?;
    } else {
        write!(tty_out, "{} (blank to skip): ", spec.description)?;
    }
    tty_out.flush()?;

    let tty_in = open_tty_read(spec.key)?;
    let mut reader = std::io::BufReader::new(tty_in);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let v = line.trim().to_string();
    Ok(if v.is_empty() { None } else { Some(v) })
}

fn prompt_secret(spec: &ParamSpec) -> Result<Option<String>> {
    let label = if spec.required {
        format!("{}: ", spec.description)
    } else {
        format!("{} (blank to skip): ", spec.description)
    };
    let v = rpassword::prompt_password(&label).with_context(|| tty_unavailable_hint(spec.key))?;
    let v = v.trim().to_string();
    Ok(if v.is_empty() { None } else { Some(v) })
}

fn tty_unavailable_hint(key: &str) -> String {
    format!(
        "Cannot open terminal for '{}' prompt. \
         Set the corresponding OPENDAL_*_{} environment variable to configure without a TTY.",
        key,
        key.to_uppercase().replace('-', "_")
    )
}

#[cfg(unix)]
fn open_tty_read(_key: &str) -> Result<std::fs::File> {
    std::fs::File::open("/dev/tty").context("Cannot open /dev/tty for input")
}

#[cfg(unix)]
fn open_tty_write(key: &str) -> Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/tty")
        .with_context(|| tty_unavailable_hint(key))
}

#[cfg(windows)]
fn open_tty_read(_key: &str) -> Result<std::fs::File> {
    std::fs::File::open("CONIN$").context("Cannot open CONIN$ for input")
}

#[cfg(windows)]
fn open_tty_write(key: &str) -> Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .open("CONOUT$")
        .with_context(|| tty_unavailable_hint(key))
}

#[cfg(not(any(unix, windows)))]
fn open_tty_read(key: &str) -> Result<std::fs::File> {
    bail!("{}", tty_unavailable_hint(key))
}

#[cfg(not(any(unix, windows)))]
fn open_tty_write(key: &str) -> Result<std::fs::File> {
    bail!("{}", tty_unavailable_hint(key))
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RemoteConfig;
    use serial_test::serial;

    fn make_cfg(scheme: &str) -> RemoteConfig {
        RemoteConfig {
            scheme: scheme.to_string(),
            root: "/".to_string(),
            params: Default::default(),
        }
    }

    #[test]
    fn specs_s3_bucket_is_required() {
        let specs = specs_for_scheme("s3");
        let bucket = specs
            .iter()
            .find(|s| s.key == "bucket")
            .expect("bucket spec");
        assert!(bucket.required);
        assert!(!bucket.sensitive);
        assert!(bucket.prompt);
    }

    #[test]
    fn specs_s3_secret_is_sensitive() {
        let specs = specs_for_scheme("s3");
        let secret = specs
            .iter()
            .find(|s| s.key == "secret-access-key")
            .expect("secret spec");
        assert!(!secret.required);
        assert!(secret.sensitive);
        assert!(secret.prompt);
    }

    #[test]
    fn specs_webdav_endpoint_is_required() {
        let specs = specs_for_scheme("webdav");
        let endpoint = specs
            .iter()
            .find(|s| s.key == "endpoint")
            .expect("endpoint spec");
        assert!(endpoint.required);
        assert!(!endpoint.sensitive);
        assert!(endpoint.prompt);
    }

    #[test]
    fn specs_fs_is_empty() {
        assert!(specs_for_scheme("fs").is_empty());
    }

    #[test]
    fn specs_memory_is_empty() {
        assert!(specs_for_scheme("memory").is_empty());
    }

    #[test]
    fn specs_unknown_is_empty() {
        assert!(specs_for_scheme("nonexistent").is_empty());
    }

    #[test]
    fn gcs_credential_has_no_prompt() {
        let specs = specs_for_scheme("gcs");
        let cred = specs
            .iter()
            .find(|s| s.key == "credential")
            .expect("credential spec");
        assert!(!cred.prompt);
    }

    #[test]
    fn gdrive_access_token_has_no_prompt() {
        let specs = specs_for_scheme("gdrive");
        let tok = specs
            .iter()
            .find(|s| s.key == "access-token")
            .expect("access-token spec");
        assert!(!tok.prompt);
    }

    /// Create a temp git repo, set GIT_DIR, run `f`, then restore GIT_DIR.
    /// Returns the TempDir so it stays alive for the duration of the call.
    fn with_temp_git_repo<F: FnOnce()>(f: F) -> tempfile::TempDir {
        let tmp = tempfile::TempDir::new().expect("TempDir");
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(tmp.path())
            .status()
            .expect("git init");
        let git_dir = tmp.path().join(".git");
        // SAFETY: single-threaded via #[serial]
        unsafe { std::env::set_var("GIT_DIR", &git_dir) };
        f();
        unsafe { std::env::remove_var("GIT_DIR") };
        tmp
    }

    #[test]
    #[serial]
    fn resolve_skips_already_set_params() {
        let _tmp = with_temp_git_repo(|| {
            let mut cfg = make_cfg("s3");
            cfg.params
                .insert("bucket".to_string(), "pre-set".to_string());
            let call_count = std::cell::Cell::new(0u32);
            resolve_with_injector(&mut cfg, |spec| {
                if spec.key == "bucket" {
                    call_count.set(call_count.get() + 1);
                }
                Ok(None)
            })
            .unwrap();
            assert_eq!(
                call_count.get(),
                0,
                "injector should not be called for pre-set params"
            );
            assert_eq!(cfg.params["bucket"], "pre-set");
        });
    }

    #[test]
    #[serial]
    fn resolve_prompts_for_missing_required() {
        let _tmp = with_temp_git_repo(|| {
            let mut cfg = make_cfg("s3");
            resolve_with_injector(&mut cfg, |spec| {
                if spec.key == "bucket" {
                    Ok(Some("injected-bucket".to_string()))
                } else {
                    Ok(None)
                }
            })
            .unwrap();
            assert_eq!(cfg.params["bucket"], "injected-bucket");
        });
    }

    #[test]
    #[serial]
    fn resolve_errors_on_blank_required() {
        let _tmp = with_temp_git_repo(|| {
            let mut cfg = make_cfg("s3");
            let result = resolve_with_injector(&mut cfg, |_| Ok(None));
            assert!(result.is_err(), "should fail when required param is blank");
        });
    }

    #[test]
    #[serial]
    fn resolve_skips_no_prompt_specs() {
        let _tmp = with_temp_git_repo(|| {
            let mut cfg = make_cfg("gcs");
            let mut prompted_keys: Vec<&str> = Vec::new();
            resolve_with_injector(&mut cfg, |spec| {
                prompted_keys.push(spec.key);
                if spec.key == "bucket" {
                    Ok(Some("test-bucket".to_string()))
                } else {
                    Ok(None)
                }
            })
            .unwrap();
            assert!(
                !prompted_keys.contains(&"credential"),
                "credential (prompt:false) should not trigger injector"
            );
        });
    }

    #[test]
    fn git_config_key_format() {
        assert_eq!(
            format!("opendal.{}.{}", "s3", "bucket"),
            "opendal.s3.bucket"
        );
        assert_eq!(
            format!("opendal.{}.{}", "azblob", "account-name"),
            "opendal.azblob.account-name"
        );
    }
}
