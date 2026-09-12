//! Configuration parsing for git-remote-opendal.
//!
//! ## Parameter resolution
//!
//! **Environment variables** — `OPENDAL_<SCHEME>_<KEY>=<value>`, mirroring
//! the convention used by OpenDAL's own CLI (`oli`).  Useful in CI/CD or
//! when you don't want to touch any config file.
//!
//! Credentials (keys, passwords) belong in env vars or a git credential
//! helper — not in git config where they could be committed accidentally.
//!
//! ## Environment Variable Reference
//!
//! You can configure any backend using environment variables following the
//! `OPENDAL_<SCHEME>_<KEY>` pattern.
//!
//! ### General Examples
//!
//! ```bash
//! # S3 Example
//! export OPENDAL_S3_BUCKET=my-bucket
//! export OPENDAL_S3_REGION=us-east-1
//! export OPENDAL_S3_ACCESS_KEY_ID=AKIA...
//! export OPENDAL_S3_SECRET_ACCESS_KEY=...
//!
//! # GCS Example
//! export OPENDAL_GCS_BUCKET=my-bucket
//! export OPENDAL_GCS_CREDENTIAL_PATH=/path/to/key.json
//! ```
//!
//! | Backend | Variable | Description |
//! | :--- | :--- | :--- |
//! | **S3** | `OPENDAL_S3_BUCKET` | **Required.** The S3 bucket name. |
//! | | `OPENDAL_S3_REGION` | AWS region (e.g., `us-east-1`). |
//! | | `OPENDAL_S3_ENDPOINT` | Custom endpoint (e.g., MinIO, B2). |
//! | | `OPENDAL_S3_ACCESS_KEY_ID` | AWS Access Key. |
//! | | `OPENDAL_S3_SECRET_ACCESS_KEY`| AWS Secret Key. |
//! | **GCS** | `OPENDAL_GCS_BUCKET` | **Required.** The GCS bucket name. |
//! | | `OPENDAL_GCS_CREDENTIAL` | Raw JSON credential string. |
//! | | `OPENDAL_GCS_CREDENTIAL_PATH` | Path to service account JSON file. |
//! | | `OPENDAL_GCS_ENDPOINT` | Custom GCS-compatible endpoint. |
//! | **Azblob** | `OPENDAL_AZBLOB_CONTAINER` | **Required.** Azure container name. |
//! | | `OPENDAL_AZBLOB_ACCOUNT_NAME` | Storage account name. |
//! | | `OPENDAL_AZBLOB_ACCOUNT_KEY` | Storage account key. |
//! | | `OPENDAL_AZBLOB_ENDPOINT` | Custom Azure endpoint. |
//! | **Gdrive** | `OPENDAL_GDRIVE_CLIENT_ID` | OAuth2 Client ID. |
//! | | `OPENDAL_GDRIVE_CLIENT_SECRET`| OAuth2 Client Secret. |
//! | | `OPENDAL_GDRIVE_REFRESH_TOKEN`| OAuth2 Refresh Token. |
//! | | `OPENDAL_GDRIVE_ACCESS_TOKEN` | Temporary Access Token. |
//! | **WebDAV** | `OPENDAL_WEBDAV_ENDPOINT` | **Required.** Server URL (e.g., `https://cloud.example.com`). |
//! | | `OPENDAL_WEBDAV_USERNAME` | Basic-auth username. |
//! | | `OPENDAL_WEBDAV_PASSWORD` | Basic-auth password or app token. |
//! | **B2** | `OPENDAL_B2_BUCKET` | **Required.** Bucket name (or URL first segment). |
//! | | `OPENDAL_B2_BUCKET_ID` | **Required.** Backblaze Bucket ID. |
//! | | `OPENDAL_B2_APPLICATION_KEY_ID` | **Required.** Application Key ID. |
//! | | `OPENDAL_B2_APPLICATION_KEY` | **Required.** Application Key. |

use anyhow::{Result, bail};
use std::collections::HashMap;
use tracing::debug;

/// Parsed remote configuration.
#[derive(Debug, Clone)]
pub struct RemoteConfig {
    /// OpenDAL scheme string, e.g. "s3", "gcs", "azblob", "fs".
    pub scheme: String,

    /// Root path inside the storage backend for this repository.
    pub root: String,

    /// Backend parameters.
    ///
    /// Keys are lowercase-hyphenated (`bucket`, `region`, `access-key-id`).
    /// Values come from environment variables.
    pub params: HashMap<String, String>,
}

impl RemoteConfig {
    /// Parse configuration from the remote URL and env vars.
    pub fn from_url_and_env(url: &str) -> Result<Self> {
        let (scheme, root) = parse_url(url)?;

        // Environment variables OPENDAL_<SCHEME>_<KEY>
        let params = collect_env_params(&scheme);
        let (root, params) = inject_url_params(&scheme, &root, params)?;

        debug!(
            "config resolved: scheme={scheme}, root={root}, params={:?}",
            params.keys().collect::<Vec<_>>()
        );

        Ok(RemoteConfig {
            scheme,
            root,
            params,
        })
    }
}

fn inject_url_params(
    scheme: &str,
    root: &str,
    mut params: HashMap<String, String>,
) -> Result<(String, HashMap<String, String>)> {
    let required_key = match scheme {
        "s3" | "gcs" | "b2" => Some("bucket"),
        "azblob" => Some("container"),
        _ => None,
    };

    let Some(required_key) = required_key else {
        return Ok((root.to_string(), params));
    };

    let value = root.trim_start_matches('/').split('/').next().unwrap_or("");

    if value.is_empty() {
        bail!(
            "URL must include a {} for scheme '{}', e.g. opendal://{}/<{}>/path",
            required_key,
            scheme,
            scheme,
            required_key
        );
    }

    if !params.contains_key(required_key) {
        params.insert(required_key.to_string(), value.to_string());
    }

    let stripped_root = root
        .trim_start_matches('/')
        .split_once('/')
        .map(|(_, rest)| format!("/{}", rest))
        .unwrap_or_else(|| "/".to_string());

    Ok((stripped_root, params))
}

// ─── URL parsing ─────────────────────────────────────────────────────────────

fn parse_url(url: &str) -> Result<(String, String)> {
    let rest = url
        .strip_prefix("opendal://")
        .ok_or_else(|| anyhow::anyhow!("URL must start with 'opendal://', got: {}", url))?;

    let (scheme, root_suffix) = match rest.find('/') {
        Some(pos) => (&rest[..pos], &rest[pos..]),
        None => (rest, "/"),
    };

    if scheme.is_empty() {
        bail!("Scheme must not be empty in URL: {}", url);
    }

    let root = if root_suffix.is_empty() {
        "/".to_string()
    } else {
        root_suffix.to_string()
    };

    Ok((scheme.to_string(), root))
}

// ─── Environment variable collection ─────────────────────────────────────────

/// Collect `OPENDAL_<SCHEME>_<KEY>` vars and return them as lowercase-hyphenated keys.
fn collect_env_params(scheme: &str) -> HashMap<String, String> {
    let prefix = format!("OPENDAL_{}_", scheme.to_uppercase());
    std::env::vars()
        .filter_map(|(k, v)| {
            k.strip_prefix(&prefix)
                .map(|suffix| (suffix.to_lowercase().replace('_', "-"), v))
        })
        .collect()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn parse_s3_url() {
        let (scheme, root) = parse_url("opendal://s3/repos/myrepo.git").unwrap();
        assert_eq!(scheme, "s3");
        assert_eq!(root, "/repos/myrepo.git");
    }

    #[test]
    #[serial]
    fn from_url_and_env_extracts_s3_bucket_from_path() {
        let cfg =
            RemoteConfig::from_url_and_env("opendal://s3/my-bucket/repos/myrepo.git").unwrap();
        assert_eq!(cfg.scheme, "s3");
        assert_eq!(cfg.root, "/repos/myrepo.git");
        assert_eq!(
            cfg.params.get("bucket").map(String::as_str),
            Some("my-bucket")
        );
    }

    #[test]
    fn from_url_and_env_extracts_gcs_bucket_from_path() {
        let cfg =
            RemoteConfig::from_url_and_env("opendal://gcs/my-bucket/repos/myrepo.git").unwrap();
        assert_eq!(cfg.scheme, "gcs");
        assert_eq!(cfg.root, "/repos/myrepo.git");
        assert_eq!(
            cfg.params.get("bucket").map(String::as_str),
            Some("my-bucket")
        );
    }

    #[test]
    fn from_url_and_env_extracts_azblob_container_from_path() {
        let cfg = RemoteConfig::from_url_and_env("opendal://azblob/my-container/repos/myrepo.git")
            .unwrap();
        assert_eq!(cfg.scheme, "azblob");
        assert_eq!(cfg.root, "/repos/myrepo.git");
        assert_eq!(
            cfg.params.get("container").map(String::as_str),
            Some("my-container")
        );
    }

    #[test]
    fn from_url_and_env_extracts_b2_bucket_from_path() {
        let cfg =
            RemoteConfig::from_url_and_env("opendal://b2/my-bucket/repos/myrepo.git").unwrap();
        assert_eq!(cfg.scheme, "b2");
        assert_eq!(cfg.root, "/repos/myrepo.git");
        assert_eq!(
            cfg.params.get("bucket").map(String::as_str),
            Some("my-bucket")
        );
    }

    #[test]
    fn parse_fs_url() {
        let (scheme, root) = parse_url("opendal://fs/tmp/repos/myrepo.git").unwrap();
        assert_eq!(scheme, "fs");
        assert_eq!(root, "/tmp/repos/myrepo.git");
    }

    #[test]
    fn parse_scheme_only() {
        let (scheme, root) = parse_url("opendal://memory").unwrap();
        assert_eq!(scheme, "memory");
        assert_eq!(root, "/");
    }

    #[test]
    fn from_url_and_env_rejects_missing_required_path_segment() {
        let err = RemoteConfig::from_url_and_env("opendal://s3/").unwrap_err();
        assert!(err.to_string().contains("bucket"));
    }

    #[test]
    #[serial]
    fn from_url_and_env_keeps_env_bucket_over_url_bucket() {
        unsafe {
            std::env::set_var("OPENDAL_S3_BUCKET", "env-bucket");
        }
        let cfg =
            RemoteConfig::from_url_and_env("opendal://s3/url-bucket/repos/myrepo.git").unwrap();
        assert_eq!(
            cfg.params.get("bucket").map(String::as_str),
            Some("env-bucket")
        );
        assert_eq!(cfg.root, "/repos/myrepo.git");
        unsafe {
            std::env::remove_var("OPENDAL_S3_BUCKET");
        }
    }

    #[test]
    fn reject_non_opendal_url() {
        assert!(parse_url("s3://bucket/path").is_err());
    }

    #[test]
    #[serial]
    fn env_params_collected() {
        // Temporarily set a fake env var.
        unsafe {
            std::env::set_var("OPENDAL_S3_BUCKET", "test-bucket");
        }
        let params = collect_env_params("s3");
        assert_eq!(
            params.get("bucket").map(String::as_str),
            Some("test-bucket")
        );
        unsafe {
            std::env::remove_var("OPENDAL_S3_BUCKET");
        }
    }
}
