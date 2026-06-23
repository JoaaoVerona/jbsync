//! Patch the per-user `<bin>.vmoptions` file.
//!
//! We only touch lines we own: `-Xmx` for the heap, plus any `extra` lines we
//! ensure are present. Every other line (Toolbox-managed tokens, JIT flags) is
//! preserved in place.

use super::{whole_file, Ctx};
use crate::config::Config;
use crate::plan::FileChange;
use anyhow::Result;

/// JetBrains names the user vmoptions file after the launcher binary.
fn vmoptions_basename(product: &str) -> String {
	let bin = match product {
		"IntelliJIdea" => "idea64",
		"WebStorm" => "webstorm64",
		"RustRover" => "rustrover64",
		"PyCharm" => "pycharm64",
		"CLion" => "clion64",
		"GoLand" => "goland64",
		"PhpStorm" => "phpstorm64",
		"Rider" => "rider64",
		"DataGrip" => "datagrip64",
		"RubyMine" => "rubymine64",
		"AndroidStudio" => "studio64",
		other => return format!("{}64.vmoptions", other.to_ascii_lowercase()),
	};
	format!("{bin}.vmoptions")
}

pub fn vmoptions(cfg: &Config, ctx: &Ctx) -> Result<Vec<FileChange>> {
	let Some(vm) = cfg.vm_options.as_ref() else {
		return Ok(vec![]);
	};
	if vm.heap_size_mb.is_none() && vm.extra.is_empty() {
		return Ok(vec![]);
	}

	let rel = vmoptions_basename(&ctx.product);
	let path = ctx.ide_dir.join(&rel);
	let existing = std::fs::read_to_string(&path).unwrap_or_default();
	// Preserve the file's existing trailing-newline convention so an unchanged
	// file stays byte-identical (JetBrains writes these without a final newline).
	let trailing_newline = if existing.is_empty() {
		true
	} else {
		existing.ends_with('\n')
	};
	let mut lines: Vec<String> = existing.lines().map(str::to_string).collect();

	if let Some(mb) = vm.heap_size_mb {
		let want = format!("-Xmx{mb}m");
		match lines.iter_mut().find(|l| l.trim_start().starts_with("-Xmx")) {
			Some(l) => *l = want,
			None => lines.insert(0, want),
		}
	}
	for extra in &vm.extra {
		if !lines.iter().any(|l| l.trim() == extra.trim()) {
			lines.push(extra.clone());
		}
	}

	let mut content = lines.join("\n");
	if trailing_newline {
		content.push('\n');
	}
	Ok(vec![whole_file(&ctx.ide_dir, &rel, content)])
}
