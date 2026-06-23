//! Each applier maps a slice of the config to one or more `FileChange`s.
//! Appliers are pure with respect to the filesystem snapshot they read.

mod files;
pub mod keymap;
mod named_settings;
mod options;
pub mod plugins;
mod scheme;
mod vmoptions;

use crate::config::{Config, PluginsCfg};
use crate::plan::{FileChange, PluginInstall};
use crate::platform::Os;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Everything an applier needs to compute changes for one IDE target.
pub struct Ctx {
	pub ide_dir: PathBuf,
	/// Where installed plugins live (data dir), used for install detection.
	pub install_dir: PathBuf,
	pub product: String,
	pub target_os: Os,
	/// Directory of the config file, for resolving referenced scheme files.
	pub config_dir: PathBuf,
	/// Effective plugins for this target (global ∪ per-target override).
	pub plugins: Option<PluginsCfg>,
}

/// The computed work for one target: declarative file changes plus any
/// imperative plugin installs.
pub struct Plan {
	pub files: Vec<FileChange>,
	pub installs: Vec<PluginInstall>,
}

impl Plan {
	pub fn is_empty(&self) -> bool {
		self.files.is_empty() && self.installs.is_empty()
	}

	pub fn change_count(&self) -> usize {
		self.files.len() + self.installs.len()
	}
}

/// Build the full, deduplicated plan of real changes for one target.
pub fn build_plan(cfg: &Config, ctx: &Ctx) -> Result<Plan> {
	let mut files: Vec<FileChange> = Vec::new();
	// Option-patched files compose through one accumulator: several appliers can
	// touch the same file (e.g. editor.xml ← editor_behavior + named_settings)
	// without clobbering each other (each reads the running content, not disk).
	let mut ps = PatchSet::new(&ctx.ide_dir);
	options::editor_font(cfg, &mut ps)?;
	options::terminal_font(cfg, &mut ps)?;
	options::console_font(cfg, &mut ps)?;
	options::ui(cfg, &mut ps)?;
	options::editor_behavior(cfg, &mut ps)?;
	named_settings::settings(cfg, &mut ps)?;
	files.extend(scheme::color_scheme(cfg, ctx, &mut ps)?);
	files.extend(scheme::code_style(cfg, ctx, &mut ps)?);
	files.extend(keymap::keymap(cfg, ctx, &mut ps)?);

	// Whole-file owners (each owns its file exclusively).
	files.extend(self::files::copy(cfg, ctx)?);
	files.extend(plugins::disabled(ctx)?);
	files.extend(vmoptions::vmoptions(cfg, ctx)?);

	files.extend(ps.into_changes());
	Ok(Plan {
		files: files.into_iter().filter(FileChange::is_change).collect(),
		installs: plugins::installs(ctx),
	})
}

// --- shared helpers ---------------------------------------------------------

/// Accumulates surgical option-patches per file so multiple appliers compose
/// instead of each reading the original from disk and the last write winning.
pub(crate) struct PatchSet<'a> {
	ide_dir: &'a Path,
	/// rel path -> (original on disk, running content)
	entries: std::collections::BTreeMap<String, (Option<String>, String)>,
}

impl<'a> PatchSet<'a> {
	fn new(ide_dir: &'a Path) -> Self {
		PatchSet {
			ide_dir,
			entries: std::collections::BTreeMap::new(),
		}
	}

	/// Apply `f` to the file's current content (seeded from disk on first touch).
	pub(crate) fn patch(&mut self, rel: &str, f: impl FnOnce(&str) -> Result<String>) -> Result<()> {
		if !self.entries.contains_key(rel) {
			let old = std::fs::read_to_string(self.ide_dir.join(rel)).ok();
			let cur = old.clone().unwrap_or_default();
			self.entries.insert(rel.to_string(), (old, cur));
		}
		let (_, cur) = self.entries.get_mut(rel).unwrap();
		let updated = f(cur)?;
		*cur = updated;
		Ok(())
	}

	fn into_changes(self) -> Vec<FileChange> {
		let ide_dir = self.ide_dir;
		self.entries
			.into_iter()
			.map(|(rel, (old, new))| FileChange {
				path: ide_dir.join(&rel),
				rel,
				old,
				new,
			})
			.collect()
	}
}

/// A change that sets a file's full content (used for generated/copied files).
pub(crate) fn whole_file(ide_dir: &Path, rel: &str, new: String) -> FileChange {
	let path = ide_dir.join(rel);
	let old = std::fs::read_to_string(&path).ok();
	FileChange {
		path,
		rel: rel.to_string(),
		old,
		new,
	}
}

pub(crate) fn bool_str(b: bool) -> &'static str {
	if b {
		"true"
	} else {
		"false"
	}
}

/// Format a float the way JetBrains does: "15.0", "1.25".
pub(crate) fn fmt_f(x: f32) -> String {
	let v = x as f64;
	if v.fract() == 0.0 {
		format!("{v:.1}")
	} else {
		// up to 4 decimals, trailing zeros trimmed
		let s = format!("{v:.4}");
		let s = s.trim_end_matches('0');
		s.trim_end_matches('.').to_string()
	}
}
