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
//! `JBSYNC_CONFIG_HOME` / `JBSYNC_DATA_HOME` override the JetBrains vendor dir;
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

/// The JetBrains config vendor dir (overridable via `JBSYNC_CONFIG_HOME`).
pub fn jetbrains_base() -> Result<PathBuf> {
	if let Ok(over) = std::env::var("JBSYNC_CONFIG_HOME") {
		return Ok(PathBuf::from(over));
	}
	let cfg = dirs::config_dir().ok_or_else(|| anyhow!("cannot determine OS config dir"))?;
	Ok(cfg.join("JetBrains"))
}

/// The JetBrains data (installed-plugins) vendor dir (overridable via `JBSYNC_DATA_HOME`).
pub fn jetbrains_data_base() -> Result<PathBuf> {
	if let Ok(over) = std::env::var("JBSYNC_DATA_HOME") {
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
			// The remainder must start with a digit, else "IntelliJIdea" would
			// also swallow a hypothetical "IntelliJIdeaEdu".
			if ver.chars().next().is_some_and(|c| c.is_ascii_digit()) {
				out.push((ver.to_string(), entry.path()));
			}
		}
	}
	Ok(out)
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
