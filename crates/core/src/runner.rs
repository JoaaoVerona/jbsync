//! Editor-agnostic execution of a [`FileChange`]: print a diff, or write it
//! atomically with an optional timestamped backup. Shared by every plugin so
//! `apply`/`check`/`--dry-run` behave identically across editors.

use crate::change::FileChange;
use anyhow::{Context, Result};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Print a unified-ish diff of one change to stdout (used by `--dry-run`).
pub fn print_diff(ch: &FileChange) {
	use similar::{ChangeTag, TextDiff};
	let old = ch.old.as_deref().unwrap_or("");
	println!("  ── {} {}", ch.rel, if ch.is_new() { "(new)" } else { "" });
	let diff = TextDiff::from_lines(old, &ch.new);
	for change in diff.iter_all_changes() {
		let sign = match change.tag() {
			ChangeTag::Delete => "-",
			ChangeTag::Insert => "+",
			ChangeTag::Equal => continue,
		};
		print!("    {sign} {}", change.value());
		if !change.value().ends_with('\n') {
			println!();
		}
	}
}

/// Write a change to disk, creating parents and (optionally) backing up the
/// overwritten content first. Writes are atomic (temp file + rename).
pub fn write_change(ch: &FileChange, backup: bool) -> Result<()> {
	if let Some(parent) = ch.path.parent() {
		std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
	}
	if backup {
		if let Some(old) = &ch.old {
			backup_file(&ch.path, old, &ch.rel)?;
		}
	}
	atomic_write(&ch.path, &ch.new)
}

/// Back up `content` to `<config-dir>/.idesync-backups/<unix-secs>/<rel>`.
fn backup_file(path: &Path, content: &str, rel: &str) -> Result<()> {
	let config_dir = backup_root(path, rel);
	let ts = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map(|d| d.as_secs())
		.unwrap_or(0);
	let dest = config_dir.join(".idesync-backups").join(ts.to_string()).join(rel);
	if let Some(parent) = dest.parent() {
		std::fs::create_dir_all(parent)?;
	}
	std::fs::write(&dest, content).with_context(|| format!("backing up to {}", dest.display()))?;
	Ok(())
}

/// Recover the config dir by stripping the relative path from the full path.
fn backup_root(path: &Path, rel: &str) -> std::path::PathBuf {
	let depth = Path::new(rel).components().count();
	let mut p = path.to_path_buf();
	for _ in 0..depth {
		p.pop();
	}
	p
}

fn atomic_write(path: &Path, content: &str) -> Result<()> {
	let tmp = path.with_extension("idesync-tmp");
	std::fs::write(&tmp, content).with_context(|| format!("writing {}", tmp.display()))?;
	std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
	Ok(())
}
