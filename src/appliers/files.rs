//! Copy self-contained config files/dirs verbatim from the config dir into the
//! IDE — for settings that live in whole managed files (menus/toolbars, live
//! templates, file templates, inspection profiles, Grazie, notifications,
//! parameter hints, file types, VCS/debugger/diff, advanced settings, …).
//!
//! Top-level `files` entries are paths relative to the config, shared across all
//! IDEs; a directory is copied recursively. Per-target `Target.files` are
//! IDE-specific (e.g. window layouts), sourced from `targets/<product>/<path>`
//! and copied into that IDE only. Files we option-patch elsewhere (editor.xml,
//! ui.lnf.xml, …) must NOT be listed here.

use super::{whole_file, Ctx};
use crate::config::Config;
use crate::plan::FileChange;
use anyhow::{Context, Result};
use std::path::Path;

pub fn copy(cfg: &Config, ctx: &Ctx) -> Result<Vec<FileChange>> {
	let mut out = vec![];
	// Shared files: sourced from the config dir, applied to every IDE.
	for rel in &cfg.files {
		let src = ctx.config_dir.join(rel);
		collect_one(&src, rel, ctx, &mut out)?;
	}
	// IDE-specific files: sourced from `targets/<product>/<rel>`, applied here only.
	let target_root = ctx.config_dir.join("targets").join(&ctx.product);
	for rel in &ctx.files {
		let src = target_root.join(rel);
		collect_one(&src, rel, ctx, &mut out)?;
	}
	Ok(out)
}

/// Resolve one `files` entry: a directory is copied recursively, a file
/// verbatim. The IDE-relative destination is `rel` regardless of source root.
fn collect_one(src: &Path, rel: &str, ctx: &Ctx, out: &mut Vec<FileChange>) -> Result<()> {
	if src.is_dir() {
		collect_dir(src, rel, ctx, out)?;
	} else if src.is_file() {
		if let Some(fc) = file_change(src, rel, ctx) {
			out.push(fc);
		}
	} else {
		// Missing source is a config error worth surfacing.
		return Err(anyhow::anyhow!("files entry not found: {}", src.display()));
	}
	Ok(())
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
