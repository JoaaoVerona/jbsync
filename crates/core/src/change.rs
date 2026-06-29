//! A `FileChange` is the unit of work shared by every editor plugin: "this file
//! should have this content". Plugins are pure (existing content -> desired
//! content); the [runner](crate::runner) decides whether to write, diff, or just
//! report drift.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct FileChange {
	pub path: PathBuf,
	/// Display path relative to the editor's config dir.
	pub rel: String,
	/// Existing content, or None if the file does not exist yet.
	pub old: Option<String>,
	pub new: String,
}

impl FileChange {
	/// Construct a change that sets `path` to `new`, reading the current content
	/// from disk as the `old` baseline (None if absent).
	pub fn new(path: PathBuf, rel: impl Into<String>, new: String) -> Self {
		let old = std::fs::read_to_string(&path).ok();
		FileChange {
			path,
			rel: rel.into(),
			old,
			new,
		}
	}

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
