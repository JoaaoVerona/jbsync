//! Install + activate color schemes and code styles.
//!
//! The big `.icls` / code-style `.xml` files are treated as opaque managed
//! artifacts (copied verbatim); only activation is patched into the small
//! selector files.

use super::{whole_file, Ctx, PatchSet};
use crate::config::Config;
use crate::plan::FileChange;
use crate::xmlpatch::{ensure, ensure_option};
use anyhow::{Context, Result};
use std::path::Path;

pub fn color_scheme(cfg: &Config, ctx: &Ctx, ps: &mut PatchSet) -> Result<Vec<FileChange>> {
	let Some(s) = cfg.color_scheme.as_ref() else {
		return Ok(vec![]);
	};
	let mut out = vec![];
	if let Some(file) = &s.file {
		out.push(install(ctx, "colors", file)?);
	}
	if s.activate {
		ps.patch("options/colors.scheme.xml", |xml| {
			ensure(
				xml,
				"EditorColorsManagerImpl",
				"global_color_scheme",
				None,
				&[("name", &s.name)],
			)
		})?;
	}
	Ok(out)
}

pub fn code_style(cfg: &Config, ctx: &Ctx, ps: &mut PatchSet) -> Result<Vec<FileChange>> {
	let Some(s) = cfg.code_style.as_ref() else {
		return Ok(vec![]);
	};
	let mut out = vec![];
	if let Some(file) = &s.file {
		out.push(install(ctx, "codestyles", file)?);
	}
	if s.activate {
		ps.patch("options/code.style.schemes.xml", |xml| {
			ensure_option(xml, "CodeStyleSchemeSettings", "CURRENT_SCHEME_NAME", &s.name)
		})?;
	}
	Ok(out)
}

/// Copy a referenced scheme file (relative to the config) into `<ide>/<subdir>/`.
fn install(ctx: &Ctx, subdir: &str, file: &str) -> Result<FileChange> {
	let src = ctx.config_dir.join(file);
	let content = std::fs::read_to_string(&src).with_context(|| format!("reading scheme file {}", src.display()))?;
	let basename = Path::new(file)
		.file_name()
		.map(|n| n.to_string_lossy().into_owned())
		.unwrap_or_else(|| "scheme".to_string());
	let rel = format!("{subdir}/{basename}");
	Ok(whole_file(&ctx.ide_dir, &rel, content))
}
