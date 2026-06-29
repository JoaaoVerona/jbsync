//! Locate per-product IDE directories across operating systems and vendors.
//!
//! Most IDEs are JetBrains products and live under a `JetBrains` vendor dir, but
//! Android Studio is a Google product and lives under `Google` instead:
//!
//!   config:  Linux ~/.config/<Vendor>, macOS ~/Library/Application Support/<Vendor>,
//!            Windows %APPDATA%\<Vendor>
//!   data:    Linux ~/.local/share/<Vendor>, macOS ~/Library/Application Support/<Vendor>,
//!            Windows %APPDATA%\<Vendor>   (data = installed plugins)
//!
//! `IDESYNC_JB_CONFIG_HOME` / `IDESYNC_JB_DATA_HOME` override the JetBrains vendor dir;
//! the Google vendor dir is resolved as its sibling.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

/// Products we recognise when scanning.
pub const KNOWN_PRODUCTS: &[&str] = &[
	"IntelliJIdea",
	"WebStorm",
	"RustRover",
	"PyCharm",
	"CLion",
	"GoLand",
	"PhpStorm",
	"Rider",
	"DataGrip",
	"RubyMine",
	"AndroidStudio",
];

/// The vendor a product ships under.
pub fn vendor(product: &str) -> &'static str {
	match product {
		"AndroidStudio" => "Google",
		_ => "JetBrains",
	}
}

/// The JetBrains config vendor dir (overridable via `IDESYNC_JB_CONFIG_HOME`).
pub fn jetbrains_base() -> Result<PathBuf> {
	if let Ok(over) = std::env::var("IDESYNC_JB_CONFIG_HOME") {
		return Ok(PathBuf::from(over));
	}
	let cfg = dirs::config_dir().ok_or_else(|| anyhow!("cannot determine OS config dir"))?;
	Ok(cfg.join("JetBrains"))
}

/// The JetBrains data (installed-plugins) vendor dir (overridable via `IDESYNC_JB_DATA_HOME`).
pub fn jetbrains_data_base() -> Result<PathBuf> {
	if let Ok(over) = std::env::var("IDESYNC_JB_DATA_HOME") {
		return Ok(PathBuf::from(over));
	}
	let data = dirs::data_dir().ok_or_else(|| anyhow!("cannot determine OS data dir"))?;
	Ok(data.join("JetBrains"))
}

/// The config vendor dir for a product (JetBrains base, or its `Google` sibling).
pub fn config_base(product: &str) -> Result<PathBuf> {
	Ok(sibling_for_vendor(&jetbrains_base()?, vendor(product)))
}

/// The data (installed-plugins) vendor dir for a product.
pub fn data_base(product: &str) -> Result<PathBuf> {
	Ok(sibling_for_vendor(&jetbrains_data_base()?, vendor(product)))
}

fn sibling_for_vendor(jetbrains_base: &Path, vendor: &str) -> PathBuf {
	if vendor == "JetBrains" {
		return jetbrains_base.to_path_buf();
	}
	match jetbrains_base.parent() {
		Some(parent) => parent.join(vendor),
		None => PathBuf::from(vendor),
	}
}

/// Resolve the config dir for one product (+ optional pinned version).
pub fn resolve_ide_dir(product: &str, version: Option<&str>) -> Result<PathBuf> {
	let base = config_base(product)?;
	if let Some(v) = version {
		return Ok(base.join(format!("{product}{v}")));
	}
	let mut candidates = list_product_versions(&base, product)?;
	// Sort by parsed version so 2026.1 > 2025.3 > 2025.10 correctly.
	candidates.sort_by(|a, b| version_key(&a.0).cmp(&version_key(&b.0)));
	candidates
		.pop()
		.map(|(_, p)| p)
		.ok_or_else(|| anyhow!("no installed config dir for '{product}' under {}", base.display()))
}

/// (version-string, path) pairs for a single product under `base`.
pub fn list_product_versions(base: &Path, product: &str) -> Result<Vec<(String, PathBuf)>> {
	let mut out = vec![];
	if !base.exists() {
		return Ok(out);
	}
	for entry in std::fs::read_dir(base).with_context(|| format!("reading {}", base.display()))? {
		let entry = entry?;
		if !entry.file_type()?.is_dir() {
			continue;
		}
		let name = entry.file_name().to_string_lossy().into_owned();
		if let Some(ver) = name.strip_prefix(product) {
			// The remainder must be a pure version string ("2026.1", "2026.1.1").
			// This rejects a different product whose name extends this one
			// ("IntelliJIdeaEdu") and — crucially — user-made backup copies like
			// "IntelliJIdea2026.1-backup", which must not be detected as an IDE.
			if is_version_string(ver) {
				out.push((ver.to_string(), entry.path()));
			}
		}
	}
	Ok(out)
}

/// True for a JetBrains config-dir version suffix: starts with a digit and is
/// only digits and dots (e.g. "2026.1", "2026.1.1"). Rejects "Edu", EAP, and
/// backup-copy suffixes like "2026.1-backup".
fn is_version_string(s: &str) -> bool {
	s.chars().next().is_some_and(|c| c.is_ascii_digit()) && s.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// (product, version, path) for every recognised IDE, across vendors.
pub fn discover_all() -> Result<Vec<(String, String, PathBuf)>> {
	let mut out = vec![];
	for product in KNOWN_PRODUCTS {
		let base = config_base(product)?;
		for (ver, path) in list_product_versions(&base, product)? {
			out.push((product.to_string(), ver, path));
		}
	}
	out.sort();
	Ok(out)
}

/// Turn "2026.1.1" into a comparable tuple of integers.
fn version_key(v: &str) -> Vec<u32> {
	v.split(|c: char| !c.is_ascii_digit())
		.filter(|s| !s.is_empty())
		.map(|s| s.parse().unwrap_or(0))
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn version_string_accepts_real_versions_only() {
		assert!(is_version_string("2026.1"));
		assert!(is_version_string("2026.1.1"));
		assert!(is_version_string("2024.3"));
		// a user-made backup copy of a config dir is NOT an IDE
		assert!(!is_version_string("2026.1-backup"));
		assert!(!is_version_string("2026.1-copy"));
		// a longer product name ("IntelliJIdeaEdu") leaves a non-version remainder
		assert!(!is_version_string("Edu"));
		assert!(!is_version_string(""));
	}

	#[test]
	fn list_product_versions_ignores_backup_dirs() {
		let tmp = std::env::temp_dir().join(format!("idesync-disco-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&tmp);
		std::fs::create_dir_all(tmp.join("IntelliJIdea2026.1")).unwrap();
		std::fs::create_dir_all(tmp.join("IntelliJIdea2026.1-backup")).unwrap();
		std::fs::create_dir_all(tmp.join("IntelliJIdeaEdu2026.1")).unwrap();
		let mut found = list_product_versions(&tmp, "IntelliJIdea").unwrap();
		found.sort();
		let versions: Vec<_> = found.iter().map(|(v, _)| v.as_str()).collect();
		assert_eq!(
			versions,
			vec!["2026.1"],
			"only the real IDE dir, not the -backup/Edu ones"
		);
		let _ = std::fs::remove_dir_all(&tmp);
	}
}
