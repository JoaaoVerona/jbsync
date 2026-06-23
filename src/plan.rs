//! A `FileChange` is the unit of work: "this file should have this content".
//! Appliers are pure (existing content -> desired content); the CLI decides
//! whether to write, diff, or just report drift.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct FileChange {
	pub path: PathBuf,
	/// Display path relative to the IDE config dir.
	pub rel: String,
	/// Existing content, or None if the file does not exist yet.
	pub old: Option<String>,
	pub new: String,
}

impl FileChange {
	pub fn is_change(&self) -> bool {
		match &self.old {
			Some(o) => o != &self.new,
			None => true,
		}
	}

	pub fn is_new(&self) -> bool {
		self.old.is_none()
	}
}

/// An imperative action: install the given (currently-missing) plugin IDs via
/// the IDE's `installPlugins` CLI. Unlike `FileChange` this is networked and has
/// side effects, so it is surfaced separately.
#[derive(Debug, Clone)]
pub struct PluginInstall {
	pub product: String,
	pub launcher: Option<PathBuf>,
	pub ids: Vec<String>,
	pub repositories: Vec<String>,
	pub install_dir: PathBuf,
}

impl PluginInstall {
	/// Arguments after the launcher binary: `installPlugins <ids...> [repos...]`.
	pub fn args(&self) -> Vec<String> {
		let mut a = vec!["installPlugins".to_string()];
		a.extend(self.ids.iter().cloned());
		a.extend(self.repositories.iter().cloned());
		a
	}

	pub fn command_display(&self) -> String {
		let launcher = self
			.launcher
			.as_ref()
			.map(|p| p.display().to_string())
			.unwrap_or_else(|| "<launcher not found>".to_string());
		format!("{launcher} {}", self.args().join(" "))
	}
}
