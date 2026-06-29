//! The standalone VSCode config model — a flat, pass-through file. Field names
//! mirror `schema/idesync-vscode.schema.json`. Used both to read configs
//! (apply/check) and to write them (create).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// A whole VSCode-family config (`idesync vsc apply <this>`). Nothing here is
/// translated: `settings`/`keybindings` are raw VSCode values applied verbatim,
/// and `extensions` are Marketplace IDs ensure-installed via the editor CLI.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct VsCodeCfg {
	#[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
	pub schema: Option<String>,
	/// Restrict to these editors by name (e.g. "Code", "VSCodium"). Empty =
	/// every discovered VSCode-family editor.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub targets: Vec<String>,
	/// Top-level `settings.json` keys to set. Every OTHER key, plus comments and
	/// formatting, is preserved — only these keys are (re)written.
	#[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
	pub settings: serde_json::Map<String, Value>,
	/// The `keybindings.json` array. When present, idesync OWNS the file (like a
	/// generated keymap — seed it with `create`); absent = left untouched.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub keybindings: Option<Vec<Value>>,
	/// Extensions to ensure installed via the editor CLI.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub extensions: Option<VsCodeExtensionsCfg>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct VsCodeExtensionsCfg {
	/// Marketplace extension IDs (`publisher.name`) to ensure installed. Only the
	/// ones not already present are installed, so `apply` stays idempotent.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub install: Vec<String>,
}

impl VsCodeCfg {
	pub fn load(path: &Path) -> Result<VsCodeCfg> {
		let text = std::fs::read_to_string(path).with_context(|| format!("reading config {}", path.display()))?;
		let cfg: VsCodeCfg =
			serde_json::from_str(&text).with_context(|| format!("parsing config {}", path.display()))?;
		Ok(cfg)
	}
}
