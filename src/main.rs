//! git-remote-opendal
//!
//! A Git remote helper that stores repository data on any storage backend
//! supported by Apache OpenDAL (S3, GCS, Azure Blob, local fs, and many more).
//!
//! Git invokes this binary as:
//!   `git-remote-opendal <remote-name> <url>`
//!
//! The URL format is:
//!   `opendal://<scheme>/<root-path>`
//!
//! For backends that require a bucket/container (`s3`, `gcs`, `azblob`), the
//! first path segment supplies that name and the remaining path becomes the
//! repository root, e.g. `opendal://s3/my-bucket/repos/myrepo`.
//!
//! Additional backend configuration is read from environment variables using
//! the `OPENDAL_<SCHEME>_<KEY>` pattern.

mod config;
mod credentials;
mod helper;
mod operator;
mod protocol;
mod storage;

use anyhow::{Context, Result, bail};
use tracing::debug;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing to stderr — git remote helpers must NEVER write
    // protocol data to stderr, only stdout.  All our diagnostics go to stderr.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_env("GIT_REMOTE_OPENDAL_LOG")
                .add_directive(tracing::Level::WARN.into()),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    debug!("invoked with args: {:?}", args);

    // Git remote helpers receive: <program> <remote-name> <url>
    // The remote-name may be absent in some invocations (e.g. direct test).
    let url = match args.len() {
        3 => args[2].clone(),
        2 => args[1].clone(),
        _ => bail!(
            "Usage: git-remote-opendal <remote-name> <url>\n\
             Got args: {:?}",
            args
        ),
    };

    let mut cfg = config::RemoteConfig::from_url_and_env(&url)
        .context("Failed to parse remote configuration")?;

    credentials::resolve(&mut cfg).context("Failed to resolve backend credentials")?;

    debug!("remote config: {:?}", cfg);

    let op = operator::build_operator(&cfg)
        .await
        .context("Failed to initialize OpenDAL operator")?;

    let mut h = helper::RemoteHelper::new(op, cfg);
    h.run().await.context("Remote helper error")?;

    Ok(())
}
