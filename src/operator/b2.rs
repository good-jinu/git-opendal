//! Backblaze B2 backend builder for opendal operator.
//!
//! This file contains the `build_b2` function which constructs an
//! `opendal::Operator` configured for Backblaze B2.
//!
//! Unlike S3, B2 does not require a region — the region is determined
//! at bucket creation time and Backblaze resolves the correct endpoint
//! automatically via `bucket_id`.
//!
//! Required parameters:
//! - `bucket`    — bucket name, from the first URL path segment
//!                 (e.g. `opendal://b2/my-bucket/repos/myrepo`) or
//!                 `OPENDAL_B2_BUCKET`.
//! - `bucket-id` — the numeric/alphanumeric bucket ID shown in the
//!                 Backblaze console, or `OPENDAL_B2_BUCKET_ID`.
//!
//! Optional parameters:
//! - `application-key-id` — B2 Application Key ID (`OPENDAL_B2_APPLICATION_KEY_ID`).
//! - `application-key`    — B2 Application Key   (`OPENDAL_B2_APPLICATION_KEY`).
//!
//! # Configuration
//!
//! ```bash
//! export OPENDAL_B2_BUCKET=my-git-bucket
//! export OPENDAL_B2_BUCKET_ID=e73ede9969c64867a77587cb
//! export OPENDAL_B2_APPLICATION_KEY_ID=000abc123def456...
//! export OPENDAL_B2_APPLICATION_KEY=K0007xyz...
//! ```

use crate::config::RemoteConfig;
use anyhow::Result;
use opendal::Operator;

/// Build a Backblaze B2 `Operator` from the `RemoteConfig`.
///
/// Required config keys:
/// - `bucket`            (required)
/// - `bucket-id`         (required)
/// - `application-key-id` (required — B2 always requires auth)
/// - `application-key`    (required — B2 always requires auth)
pub fn build_b2(cfg: &RemoteConfig) -> Result<Operator> {
    use anyhow::anyhow;
    use opendal::services::B2;
    use tracing::debug;

    let bucket = cfg.params.get("bucket").ok_or_else(|| {
        anyhow!(
            "B2 requires a bucket name, e.g. opendal://b2/my-bucket/path or OPENDAL_B2_BUCKET.\n\
             Example: export OPENDAL_B2_BUCKET=my-git-bucket"
        )
    })?;

    let bucket_id = cfg.params.get("bucket-id").ok_or_else(|| {
        anyhow!(
            "B2 requires a bucket ID (OPENDAL_B2_BUCKET_ID).\n\
             Find it in the Backblaze console under Buckets → your bucket → Bucket ID.\n\
             Example: export OPENDAL_B2_BUCKET_ID=e73ede9969c64867a77587cb"
        )
    })?;

    let application_key_id = cfg.params.get("application-key-id").ok_or_else(|| {
        anyhow!(
            "B2 requires an Application Key ID (OPENDAL_B2_APPLICATION_KEY_ID).\n\
             Generate one in the Backblaze console under App Keys.\n\
             Example: export OPENDAL_B2_APPLICATION_KEY_ID=000abc123..."
        )
    })?;

    let application_key = cfg.params.get("application-key").ok_or_else(|| {
        anyhow!(
            "B2 requires an Application Key (OPENDAL_B2_APPLICATION_KEY).\n\
             Generate one in the Backblaze console under App Keys.\n\
             Example: export OPENDAL_B2_APPLICATION_KEY=K0007xyz..."
        )
    })?;

    debug!(
        "building B2 operator for bucket={} bucket_id={}",
        bucket, bucket_id
    );

    let mut b = B2::default();
    b = b.bucket(bucket);
    b = b.bucket_id(bucket_id);
    b = b.root(&cfg.root);
    b = b.application_key_id(application_key_id);
    b = b.application_key(application_key);

    Ok(opendal::Operator::new(b)?.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn base_params() -> HashMap<String, String> {
        let mut p = HashMap::new();
        p.insert("bucket".to_string(), "my-bucket".to_string());
        p.insert("bucket-id".to_string(), "e73ede9969c64867a77587cb".to_string());
        p.insert("application-key-id".to_string(), "000abc".to_string());
        p.insert("application-key".to_string(), "K0007xyz".to_string());
        p
    }

    #[test]
    fn test_build_b2_missing_bucket() {
        let cfg = RemoteConfig {
            scheme: "b2".to_string(),
            root: "/".to_string(),
            params: HashMap::new(),
        };
        let res = build_b2(&cfg);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("bucket name"));
    }

    #[test]
    fn test_build_b2_missing_bucket_id() {
        let mut params = HashMap::new();
        params.insert("bucket".to_string(), "my-bucket".to_string());
        let cfg = RemoteConfig {
            scheme: "b2".to_string(),
            root: "/".to_string(),
            params,
        };
        let res = build_b2(&cfg);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("bucket ID"));
    }

    #[test]
    fn test_build_b2_missing_application_key_id() {
        let mut params = HashMap::new();
        params.insert("bucket".to_string(), "my-bucket".to_string());
        params.insert("bucket-id".to_string(), "e73ede9969c64867a77587cb".to_string());
        let cfg = RemoteConfig {
            scheme: "b2".to_string(),
            root: "/".to_string(),
            params,
        };
        let res = build_b2(&cfg);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("Application Key ID"));
    }

    #[test]
    fn test_build_b2_missing_application_key() {
        let mut params = HashMap::new();
        params.insert("bucket".to_string(), "my-bucket".to_string());
        params.insert("bucket-id".to_string(), "e73ede9969c64867a77587cb".to_string());
        params.insert("application-key-id".to_string(), "000abc".to_string());
        let cfg = RemoteConfig {
            scheme: "b2".to_string(),
            root: "/".to_string(),
            params,
        };
        let res = build_b2(&cfg);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("Application Key"));
    }

    #[test]
    fn test_build_b2_all_params() {
        let cfg = RemoteConfig {
            scheme: "b2".to_string(),
            root: "/repos".to_string(),
            params: base_params(),
        };
        assert!(build_b2(&cfg).is_ok());
    }
}
