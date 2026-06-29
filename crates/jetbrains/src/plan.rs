//! JetBrains-specific imperative actions. The declarative file-change model lives
//! in `idesync_core` ([`FileChange`](idesync_core::FileChange)); this module adds
//! the one networked side effect JetBrains needs: installing plugins.

use std::path::PathBuf;

/// An imperative action: install the given (currently-missing) plugin IDs via
/// the IDE's `installPlugins` CLI. Unlike a `FileChange` this is networked and
/// has side effects, so it is surfaced separately.
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
