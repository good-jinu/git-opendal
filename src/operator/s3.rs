//! S3 backend builder for opendal operator.
//!
//! This file contains the `build_s3` function which constructs an
//! `opendal::Operator` configured for S3-compatible services (AWS S3,
//! MinIO, Cloudflare R2, etc.).
//!
//! Required parameter: `bucket`, supplied either as the first path segment of
//! the remote URL (e.g. `opendal://s3/my-bucket/repos/myrepo`) or via
//! `OPENDAL_S3_BUCKET`. Optional params: `region`, `endpoint`,
//! `access-key-id`, `secret-access-key`.
//!
//! # Configuration
//!
//! Configuration is provided via environment variables:
//!
//! ```bash
//! export OPENDAL_S3_BUCKET=my-git-bucket
//! export OPENDAL_S3_REGION=us-east-1
//! export OPENDAL_S3_ENDPOINT=https://s3.amazonaws.com
//! export OPENDAL_S3_ACCESS_KEY_ID=access_key
//! export OPENDAL_S3_SECRET_ACCESS_KEY=secret_key
//! ```

use crate::config::RemoteConfig;
use anyhow::Result;
use opendal::Operator;

/// Build an S3 `Operator` from the `RemoteConfig`.
///
/// Examples of config keys used:
/// - `bucket` (required)
/// - `region`
/// - `endpoint`
/// - `access-key-id`
/// - `secret-access-key`
pub fn build_s3(cfg: &RemoteConfig) -> Result<Operator> {
    use anyhow::anyhow;
    use opendal::services::S3;
    use tracing::debug;

    let bucket = cfg.params.get("bucket").ok_or_else(|| {
        anyhow!(
            "S3 requires a bucket, e.g. opendal://s3/my-bucket/path or OPENDAL_S3_BUCKET.\n\
             Example: export OPENDAL_S3_BUCKET=my-git-bucket"
        )
    })?;

    debug!("building S3 operator for bucket={}", bucket);

    let mut b = S3::default();
    b = b.bucket(bucket);
    b = b.root(&cfg.root);

    if let Some(v) = cfg.params.get("region") {
        b = b.region(v);
    }
    if let Some(v) = cfg.params.get("endpoint") {
        b = b.endpoint(v);
    }
    if let Some(v) = cfg.params.get("access-key-id") {
        b = b.access_key_id(v);
    }
    if let Some(v) = cfg.params.get("secret-access-key") {
        b = b.secret_access_key(v);
    }

    Ok(opendal::Operator::new(b)?.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_build_s3_missing_bucket() {
        let cfg = RemoteConfig {
            scheme: "s3".to_string(),
            root: "/path".to_string(),
            params: HashMap::new(),
        };

        let res = build_s3(&cfg);
        assert!(res.is_err());
        let err_msg = res.unwrap_err().to_string();
        assert!(
            err_msg.contains("S3 requires a bucket"),
            "Expected error message to contain 'S3 requires a bucket', got: {}",
            err_msg
        );
    }

    #[test]
    fn test_build_s3_minimal() {
        let mut params = HashMap::new();
        params.insert("bucket".to_string(), "my-bucket".to_string());
        params.insert("region".to_string(), "us-east-1".to_string());

        let cfg = RemoteConfig {
            scheme: "s3".to_string(),
            root: "/path".to_string(),
            params,
        };

        let op = build_s3(&cfg);
        assert!(
            op.is_ok(),
            "Expected build_s3 to succeed with bucket and region provided"
        );
    }

    #[test]
    fn test_build_s3_with_all_params() {
        let mut params = HashMap::new();
        params.insert("bucket".to_string(), "my-bucket".to_string());
        params.insert("region".to_string(), "us-west-2".to_string());
        params.insert(
            "endpoint".to_string(),
            "https://s3.us-west-2.amazonaws.com".to_string(),
        );
        params.insert("access-key-id".to_string(), "test-key-id".to_string());
        params.insert(
            "secret-access-key".to_string(),
            "test-secret-key".to_string(),
        );

        let cfg = RemoteConfig {
            scheme: "s3".to_string(),
            root: "/path".to_string(),
            params,
        };

        let op = build_s3(&cfg);
        assert!(
            op.is_ok(),
            "Expected build_s3 to succeed with all parameters provided"
        );
    }
}
