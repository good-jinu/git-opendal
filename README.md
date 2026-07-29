# git-opendal

Use any cloud storage — **S3, GCS, Azure Blob, local filesystem, and [50+ more](https://opendal.apache.org)** — as a plain Git remote.

Backed by [Apache OpenDAL™](https://opendal.apache.org), one unified API for all storage.

One installation provides two Git extensions:

- `git-remote-opendal` is the transport Git starts automatically for `opendal://` remotes.
- `git-opendal` is available as `git opendal ...` for setup, diagnostics, and first-push workflows.

---

## How it works

`git-remote-opendal` is a [Git remote helper](https://git-scm.com/docs/gitremote-helpers).
Git invokes it automatically when it sees a remote URL starting with `opendal://`.

The helper implements the **`import`/`export`** capability pair:

- **Push** (`git push`): git streams a `git fast-export` bundle → helper converts
  it to a `.bundle` file → uploads to cloud storage → updates `info/refs.json`.
- **Fetch/Clone** (`git fetch`, `git clone`): helper downloads all stored bundles
  in order → pipes them through `git bundle unbundle` → feeds the resulting
  fast-import stream back to git.

The companion command does not replace normal Git commands or reimplement
transport. It only makes setup and operations easier:

```sh
git opendal setup --backend fs --path /tmp/my-git-remotes/myrepo --push
git opendal doctor --backend fs
git opendal status --probe
```

After setup, use ordinary `git clone`, `git fetch`, `git pull`, and `git push`.

### Storage layout (inside the backend root)

```
<root>/
  info/
    refs.json        ← ref name → SHA-1 map + ordered bundle list
  objects/
    <timestamp>.bundle  ← one bundle per push
```

---

## Requirements

- Git ≥ 2.20

---

## Installation

### Option 1 — Install script (recommended)

The quickest way to get started on **macOS** and **Linux**:

```sh
curl -fsSL https://raw.githubusercontent.com/good-jinu/git-opendal/main/install.sh | sh
```

This detects your platform, downloads the release archive containing both Git
extensions, and optionally adds them to your `PATH`.

**Install a specific version:**

```sh
curl -fsSL https://raw.githubusercontent.com/good-jinu/git-opendal/main/install.sh | sh -s -- v0.4.0
```

**Non-interactive (CI / scripts):**

```sh
curl -fsSL https://raw.githubusercontent.com/good-jinu/git-opendal/main/install.sh | sh -s -- --yes
```

> The script supports `--no-modify-path` if you prefer to manage `PATH` yourself.

---

### Option 2 — Download a pre-built binary

Download the archive for your platform from the
[**Releases page**](https://github.com/good-jinu/git-opendal/releases), then extract
and place the binary somewhere on your `PATH`.

| Platform | Archive |
|----------|---------|
| Linux x86\_64 | `git-opendal-{version}-x86_64-unknown-linux-musl.tar.gz` |
| Linux aarch64 | `git-opendal-{version}-aarch64-unknown-linux-musl.tar.gz` |
| macOS x86\_64 | `git-opendal-{version}-x86_64-apple-darwin.tar.gz` |
| macOS Apple Silicon | `git-opendal-{version}-aarch64-apple-darwin.tar.gz` |
| Windows x86\_64 | `git-opendal-{version}-x86_64-pc-windows-msvc.zip` |

Linux binaries are statically linked (musl) — no glibc dependency.

```sh
# Example for Linux x86_64
VERSION=v0.1.0
curl -fsSL "https://github.com/good-jinu/git-opendal/releases/download/$VERSION/git-opendal-$VERSION-x86_64-unknown-linux-musl.tar.gz" \
  | tar -xz --strip-components=1 -C ~/.local/bin
```

This extracts both `git-remote-opendal` and `git-opendal`.

Checksums are published alongside each release as `SHA256SUMS.txt`.

---

### Option 3 — Install via Cargo (crates.io)

If you already have Rust and Cargo installed, one package installs both
executables directly from [crates.io](https://crates.io):

```sh
cargo install --locked git-opendal
```

> **Note:** Requires Rust ≥ 1.85 (due to OpenDAL 0.56 MSRV). Make sure the Cargo binary directory (usually `~/.cargo/bin`) is in your `PATH`.

---

### Option 4 — Build from source

To build and install the binary directly from the source repository:

```sh
git clone https://github.com/good-jinu/git-opendal
cd git-opendal
cargo build --release
# Copy both binaries to a directory on your PATH, for example:
cp target/release/git-remote-opendal target/release/git-opendal ~/.local/bin/
```

---

## Usage

### Guided setup with `git opendal`

The workflow command validates Git and the helper, builds the remote URL,
registers it, and can bootstrap the first push. It is installed as both
`git-opendal` and the Git subcommand `git opendal`.

```bash
# Local filesystem
git opendal setup --backend fs --path /tmp/my-bare-repos/myrepo --push

# S3, GCS, and Azure Blob require --bucket (the Azure value is a container)
git opendal setup --backend s3 --bucket my-git-bucket --path repos/myrepo --push

# Validate prerequisites and credentials without changing a repository
git opendal doctor --backend s3

# Programmatic discovery & AI agent schema
git opendal schema

# Output all diagnostics and operations as machine-readable JSON
git opendal --json doctor --backend s3
```

Use `git opendal --help` for `bootstrap`, `status`, `clone`, `url`, `schema`, and
backend configuration guidance. The global `--json` flag formats output (and errors) as
structured JSON for automation and AI agents.

### URL format

```
opendal://<scheme>/<root-path>
```

| Part | Description |
|------|-------------|
| `scheme` | OpenDAL backend: `s3`, `gcs`, `azblob`, `gdrive`, `webdav`, `fs` |
| `root-path` | Path inside the backend that acts as the repository root |

For bucketed/container backends (`s3`, `gcs`, `azblob`), the first path
segment is used as the bucket/container name and the rest is the repository
root. Example: `opendal://s3/my-bucket/repos/myrepo`.

The `memory` backend exists only for unit tests and single-process debugging.
It is not suitable for normal Git operations because each helper invocation gets
an isolated in-memory store.

### Backend configuration

Backend parameters come from environment variables using the pattern:

```
OPENDAL_<SCHEME>_<PARAM>=<value>
```

This keeps credentials out of git-config and out of repository history.

---

## Examples

### Amazon S3 / S3-compatible (MinIO, Cloudflare R2, …)

```bash
# Set credentials
export OPENDAL_S3_BUCKET=my-git-bucket
export OPENDAL_S3_REGION=us-east-1
export OPENDAL_S3_ACCESS_KEY_ID=AKIA...
export OPENDAL_S3_SECRET_ACCESS_KEY=...
# Optional for S3-compatible endpoints (MinIO, R2, etc.):
export OPENDAL_S3_ENDPOINT=https://my-minio.example.com

# Configure the remote and publish the current branch
git opendal setup --backend s3 --bucket my-git-bucket --path myrepo --push

# Use normal Git commands afterward
git clone opendal://s3/my-git-bucket/myrepo
```

---

### Google Cloud Storage

```bash
git remote add origin opendal://gcs/my-gcs-bucket/myrepo

export OPENDAL_GCS_CREDENTIAL_PATH=/path/to/service-account.json
# or: export OPENDAL_GCS_CREDENTIAL=raw_json_string
# Optional:
export OPENDAL_GCS_ENDPOINT=https://storage.googleapis.com

git push origin main
```

---

### Azure Blob Storage

```bash
git remote add origin opendal://azblob/my-git-container/myrepo

export OPENDAL_AZBLOB_ACCOUNT_NAME=myaccount
export OPENDAL_AZBLOB_ACCOUNT_KEY=...
# Optional:
export OPENDAL_AZBLOB_ENDPOINT=https://myaccount.blob.core.windows.net

git push origin main
```

---

### Local filesystem (testing)

```bash
git remote add origin opendal://fs/tmp/my-bare-repos/myrepo

git push origin main
git clone opendal://fs/tmp/my-bare-repos/myrepo local-clone
```

---

### Google Drive

```bash
git remote add origin opendal://gdrive/my-git-repos/myrepo

export OPENDAL_GDRIVE_CLIENT_ID=my-client-id
export OPENDAL_GDRIVE_CLIENT_SECRET=my-client-secret
export OPENDAL_GDRIVE_REFRESH_TOKEN=my-refresh-token
export OPENDAL_GDRIVE_ACCESS_TOKEN=my-access-token

git push origin main
```

---

### WebDAV (Nextcloud, ownCloud, …)

```bash
git remote add origin opendal://webdav/remote.php/dav/files/my-user/myrepo

export OPENDAL_WEBDAV_ENDPOINT=https://cloud.example.com
export OPENDAL_WEBDAV_USERNAME=my-user
export OPENDAL_WEBDAV_PASSWORD=my-app-password

git push origin main
```

The URL path is the repository root on the server; the endpoint (scheme +
host, and any base path your server needs) is configured via
`OPENDAL_WEBDAV_ENDPOINT`.

---

## Contributing

Contributions are welcome. Please open an issue or pull request with a clear
description and include tests when practical.

---

## Limitations

- **Concurrent pushes**: last-writer-wins for `refs.json`.  Use a separate
  coordination mechanism (e.g. object locking, DynamoDB) for teams.
- **Large histories**: all bundles are downloaded on fetch.  Incremental
  fetch (only new bundles since last fetch) is on the roadmap.
- **Shallow clones** (`--depth`): not supported.

---

## License

MIT
