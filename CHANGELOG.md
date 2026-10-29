# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.5](https://github.com/good-jinu/git-opendal/compare/v0.4.4...v0.4.5) - 2026-10-29

### Added

- add GHCR multi-arch container image release and documentation ([#21](https://github.com/good-jinu/git-opendal/pull/21))

## [0.4.4](https://github.com/good-jinu/git-opendal/compare/v0.4.3...v0.4.4) - 2026-10-29

### Other

- accept S3 session tokens via OPENDAL_S3_SESSION_TOKEN ([#19](https://github.com/good-jinu/git-opendal/pull/19))

## [0.4.3](https://github.com/good-jinu/git-opendal/compare/v0.4.2...v0.4.3) - 2026-10-27

### Other

- import into private ref namespace and skip empty push bundles ([#17](https://github.com/good-jinu/git-opendal/pull/17))

## [0.4.2](https://github.com/good-jinu/git-opendal/compare/v0.4.1...v0.4.2) - 2026-09-12

### Added

- support Backblaze B2 backend ([#16](https://github.com/good-jinu/git-opendal/pull/16))

### Fixed

- strip leading '+' from refspec and add force flag to Command::Push ([#13](https://github.com/good-jinu/git-opendal/pull/13))

### Other

- support webdav ([#11](https://github.com/good-jinu/git-opendal/pull/11))
- add unit tests for S3 operator builder ([#15](https://github.com/good-jinu/git-opendal/pull/15))
- add release-plz workflow and configuration
