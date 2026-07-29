//! Operator builders for each supported OpenDAL service.
//!
//! This module used to be a single-file implementation. It is now split into
//! per-service files to make adding new backends easier and to keep each
//! builder small and well-documented. Each backend file exposes a
//! `build_<scheme>(cfg: &RemoteConfig) -> Result<Operator>` function.

use crate::config::RemoteConfig;
use anyhow::{Result, bail};
use opendal::{Operator, layers::LoggingLayer};
use tracing::debug;

mod azblob;
mod fs;
mod gcs;
mod gdrive;
mod memory;
mod s3;
mod webdav;

const USER_SUPPORTED_SCHEMES: &str = "s3, gcs, azblob, gdrive, webdav, fs";

/// Build a configured [`Operator`] for the remote.
pub async fn build_operator(cfg: &RemoteConfig) -> Result<Operator> {
    debug!("scheme={} root={}", cfg.scheme, cfg.root);

    let op = match cfg.scheme.as_str() {
        "s3" => s3::build_s3(cfg)?,
        "gcs" => gcs::build_gcs(cfg)?,
        "azblob" => azblob::build_azblob(cfg)?,
        "gdrive" => gdrive::build_gdrive(cfg)?,
        "webdav" => webdav::build_webdav(cfg)?,
        "fs" => fs::build_fs(cfg)?,
        "memory" => memory::build_memory()?,
        other => bail!(
            "Unsupported scheme '{other}'. Supported: {USER_SUPPORTED_SCHEMES}.\n\
             Enable the matching 'services-<name>' feature and add a branch in operator.rs."
        ),
    };

    Ok(op.layer(LoggingLayer::default()))
}

// No helper traits required — per-backend builders return `Result<Operator>` directly.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_supported_schemes_do_not_advertise_memory() {
        assert_eq!(
            USER_SUPPORTED_SCHEMES,
            "s3, gcs, azblob, gdrive, webdav, fs"
        );
        assert!(!USER_SUPPORTED_SCHEMES.contains("memory"));
    }
}
