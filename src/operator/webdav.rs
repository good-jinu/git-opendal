//! WebDAV backend builder for opendal operator.
//!
//! This file contains the `build_webdav` function which constructs an
//! `opendal::Operator` configured for WebDAV-compatible servers (Nextcloud,
//! ownCloud, nginx `ngx_http_dav_module`, etc.).
//!
//! Required parameter: `endpoint`, supplied via `OPENDAL_WEBDAV_ENDPOINT`.
//! Optional params: `username`, `password` (HTTP basic auth). The repository
//! root comes from the URL path (e.g.
//! `opendal://webdav/remote.php/dav/files/user/repos/myrepo`).
//!
//! # Configuration
//!
//! Configuration is provided via environment variables:
//!
//! ```bash
//! export OPENDAL_WEBDAV_ENDPOINT=https://cloud.example.com
//! export OPENDAL_WEBDAV_USERNAME=my-user
//! export OPENDAL_WEBDAV_PASSWORD=my-app-password
//! ```

use crate::config::RemoteConfig;
use anyhow::Result;
use opendal::Operator;

/// Build a WebDAV `Operator` from the `RemoteConfig`.
///
/// Examples of config keys used:
/// - `endpoint` (required)
/// - `username`
/// - `password`
pub fn build_webdav(cfg: &RemoteConfig) -> Result<Operator> {
    use anyhow::anyhow;
    use opendal::services::Webdav;
    use tracing::debug;

    let endpoint = cfg.params.get("endpoint").ok_or_else(|| {
        anyhow!(
            "WebDAV requires an endpoint, e.g. OPENDAL_WEBDAV_ENDPOINT=https://cloud.example.com\n\
             The repository path comes from the URL: opendal://webdav/remote.php/dav/files/<user>/repos/myrepo"
        )
    })?;

    debug!("building WebDAV operator for endpoint={}", endpoint);

    let mut b = Webdav::default();
    b = b.endpoint(endpoint);
    b = b.root(&cfg.root);

    if let Some(v) = cfg.params.get("username") {
        b = b.username(v);
    }
    if let Some(v) = cfg.params.get("password") {
        b = b.password(v);
    }

    Ok(opendal::Operator::new(b)?.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_build_webdav_missing_endpoint() {
        let cfg = RemoteConfig {
            scheme: "webdav".to_string(),
            root: "/remote.php/dav/files/user/repo".to_string(),
            params: HashMap::new(),
        };

        let res = build_webdav(&cfg);
        assert!(res.is_err());
        let err_msg = res.unwrap_err().to_string();
        assert!(err_msg.contains("WebDAV requires an endpoint"));
    }

    #[test]
    fn test_build_webdav_minimal() {
        let mut params = HashMap::new();
        params.insert(
            "endpoint".to_string(),
            "https://cloud.example.com".to_string(),
        );

        let cfg = RemoteConfig {
            scheme: "webdav".to_string(),
            root: "/remote.php/dav/files/user/repo".to_string(),
            params,
        };

        let op = build_webdav(&cfg);
        assert!(op.is_ok());
    }

    #[test]
    fn test_build_webdav_with_all_params() {
        let mut params = HashMap::new();
        params.insert(
            "endpoint".to_string(),
            "https://cloud.example.com".to_string(),
        );
        params.insert("username".to_string(), "testuser".to_string());
        params.insert("password".to_string(), "testpass".to_string());

        let cfg = RemoteConfig {
            scheme: "webdav".to_string(),
            root: "/remote.php/dav/files/user/repo".to_string(),
            params,
        };

        let op = build_webdav(&cfg);
        assert!(op.is_ok());
    }
}
