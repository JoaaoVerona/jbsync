//! Copy self-contained config files/dirs verbatim from the config dir into the
//! IDE — for settings that live in whole managed files (menus/toolbars, live
//! templates, file templates, inspection profiles, Grazie, notifications,
//! parameter hints, file types, VCS/debugger/diff, advanced settings, …).
//!
//! Each `files` entry is a path relative to the config; a directory is copied
//! recursively. Files we option-patch elsewhere (editor.xml, ui.lnf.xml, …)
//! must NOT be listed here.

use super::{whole_file, Ctx};
use crate::config::Config;
use crate::plan::FileChange;
use anyhow::{Context, Result};
use std::path::Path;

pub fn copy(cfg: &Config, ctx: &Ctx) -> Result<Vec<FileChange>> {
	let mut out = vec![];
	for rel in &cfg.files {
		let src = ctx.config_dir.join(rel);
		if src.is_dir() {
			collect_dir(&src, rel, ctx, &mut out)?;
		} else if src.is_file() {
			if let Some(fc) = file_change(&src, rel, ctx) {
				out.push(fc);
			}
		} else {
			// Missing source is a config error worth surfacing.
			return Err(anyhow::anyhow!("files entry not found: {}", src.display()));
		}
	}
	Ok(out)
}

fn collect_dir(dir: &Path, rel: &str, ctx: &Ctx, out: &mut Vec<FileChange>) -> Result<()> {
	for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
		let entry = entry?;
		let path = entry.path();
		let child_rel = format!("{rel}/{}", entry.file_name().to_string_lossy());
		if path.is_dir() {
			collect_dir(&path, &child_rel, ctx, out)?;
		} else if let Some(fc) = file_change(&path, &child_rel, ctx) {
			out.push(fc);
		}
	}
	Ok(())
}

fn file_change(src: &Path, rel: &str, ctx: &Ctx) -> Option<FileChange> {
	// These config files are text; skip anything non-UTF-8 rather than corrupt it.
	let content = std::fs::read_to_string(src).ok()?;
	Some(whole_file(&ctx.ide_dir, rel, content))
}
