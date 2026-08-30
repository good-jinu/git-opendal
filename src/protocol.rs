//! Git remote helper line protocol types and I/O helpers.
//!
//! Git communicates with the helper over stdin/stdout using a simple
//! newline-terminated text protocol.  This module provides:
//!
//! - [`Command`]: the parsed command from git → helper direction.
//! - [`read_command`]: read one command from stdin.
//! - [`write_line`]: write a line to stdout and flush.

use anyhow::Result;
use std::io::{self, BufRead, Write};
use tracing::trace;

/// Commands that git sends to a remote helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `capabilities` — git asks what the helper supports.
    Capabilities,

    /// `list` — git wants the set of remote refs (for fetch/clone).
    List,

    /// `list for-push` — git wants refs before a push.
    ListForPush,

    /// `import <refspec>` — git wants us to produce a fast-import stream.
    Import(Vec<String>),

    /// `push <src>:<dst>` — git wants to push a local ref to a remote ref.
    Push {
        src: String,
        dst: String,
        force: bool,
    },

    /// `option <key> <value>` — git sends options (verbosity, etc.).
    Option(String, String),

    /// Empty line — batch terminator.
    Blank,

    /// Unknown command — for graceful handling.
    Unknown(String),
}

/// Read one command line from stdin and parse it.
///
/// Returns `None` on EOF (git closed the pipe → helper should exit).
pub fn read_command(stdin: &mut impl BufRead) -> Result<Option<Command>> {
    let mut line = String::new();
    let n = stdin.read_line(&mut line)?;
    if n == 0 {
        return Ok(None); // EOF
    }

    let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
    trace!("← {}", trimmed);

    let cmd = parse_command(trimmed)?;
    Ok(Some(cmd))
}

fn parse_command(line: &str) -> Result<Command> {
    if line.is_empty() {
        return Ok(Command::Blank);
    }

    let mut parts = line.splitn(2, ' ');
    let verb = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();

    match verb {
        "capabilities" => Ok(Command::Capabilities),
        "list" if rest == "for-push" => Ok(Command::ListForPush),
        "list" => Ok(Command::List),
        "import" => Ok(Command::Import(vec![rest.to_string()])),
        "push" => {
            let (force, refspec) = if let Some(stripped) = rest.strip_prefix('+') {
                (true, stripped)
            } else {
                (false, rest)
            };
            let mut parts = refspec.splitn(2, ':');
            let src = parts.next().unwrap_or("").to_string();
            let dst = parts.next().unwrap_or("").to_string();
            Ok(Command::Push { src, dst, force })
        }
        "option" => {
            let mut opt_parts = rest.splitn(2, ' ');
            let key = opt_parts.next().unwrap_or("").to_string();
            let val = opt_parts.next().unwrap_or("").to_string();
            Ok(Command::Option(key, val))
        }
        other => Ok(Command::Unknown(other.to_string())),
    }
}

/// Write a line to stdout and flush.
pub fn write_line(line: &str) -> Result<()> {
    trace!("→ {}", line);
    let stdout = io::stdout();
    let mut out = stdout.lock();
    writeln!(out, "{}", line)?;
    out.flush()?;
    Ok(())
}

/// Write a blank line (batch/section terminator) to stdout.
pub fn write_blank() -> Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    writeln!(out)?;
    out.flush()?;
    trace!("→ <blank>");
    Ok(())
}

/// Write raw bytes to stdout (used when streaming fast-import data).
pub fn write_raw(data: &[u8]) -> Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    out.write_all(data)?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_capabilities() {
        assert_eq!(
            parse_command("capabilities").unwrap(),
            Command::Capabilities
        );
    }

    #[test]
    fn parse_list() {
        assert_eq!(parse_command("list").unwrap(), Command::List);
    }

    #[test]
    fn parse_list_for_push() {
        assert_eq!(
            parse_command("list for-push").unwrap(),
            Command::ListForPush
        );
    }

    #[test]
    fn parse_import() {
        assert_eq!(
            parse_command("import refs/heads/main").unwrap(),
            Command::Import(vec!["refs/heads/main".to_string()])
        );
    }

    #[test]
    fn parse_option() {
        assert_eq!(
            parse_command("option verbosity 1").unwrap(),
            Command::Option("verbosity".to_string(), "1".to_string())
        );
    }

    #[test]
    fn parse_blank() {
        assert_eq!(parse_command("").unwrap(), Command::Blank);
    }

    #[test]
    fn parse_push_standard() {
        assert_eq!(
            parse_command("push refs/heads/main:refs/heads/main").unwrap(),
            Command::Push {
                src: "refs/heads/main".to_string(),
                dst: "refs/heads/main".to_string(),
                force: false,
            }
        );
    }

    #[test]
    fn parse_push_force() {
        assert_eq!(
            parse_command("push +refs/heads/main:refs/heads/main").unwrap(),
            Command::Push {
                src: "refs/heads/main".to_string(),
                dst: "refs/heads/main".to_string(),
                force: true,
            }
        );
    }

    #[test]
    fn parse_push_delete() {
        assert_eq!(
            parse_command("push :refs/heads/main").unwrap(),
            Command::Push {
                src: "".to_string(),
                dst: "refs/heads/main".to_string(),
                force: false,
            }
        );
    }

    #[test]
    fn parse_push_force_delete() {
        assert_eq!(
            parse_command("push +:refs/heads/main").unwrap(),
            Command::Push {
                src: "".to_string(),
                dst: "refs/heads/main".to_string(),
                force: true,
            }
        );
    }
}
