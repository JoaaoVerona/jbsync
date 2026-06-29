//! The user-facing JSON config model. Field names mirror `idesync-jetbrains.schema.json`.
//! The same types are used to read configs (apply/check) and to write them (create).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Config {
	#[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
	pub schema: Option<String>,
	/// Which IDEs to apply to.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub targets: Vec<Target>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub editor: Option<EditorCfg>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub terminal: Option<TerminalCfg>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub console: Option<ConsoleCfg>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub ui: Option<UiCfg>,
	#[serde(default, rename = "editorBehavior", skip_serializing_if = "Option::is_none")]
	pub editor_behavior: Option<EditorBehaviorCfg>,
	#[serde(default, rename = "colorScheme", skip_serializing_if = "Option::is_none")]
	pub color_scheme: Option<SchemeRef>,
	#[serde(default, rename = "codeStyle", skip_serializing_if = "Option::is_none")]
	pub code_style: Option<SchemeRef>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub plugins: Option<PluginsCfg>,
	#[serde(default, rename = "vmOptions", skip_serializing_if = "Option::is_none")]
	pub vm_options: Option<VmOptionsCfg>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub keymap: Option<KeymapCfg>,
	/// Flat IDE settings keyed by registry name (see `settings.rs`).
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub settings: BTreeMap<String, Value>,
	/// Config-relative paths (files or dirs) copied verbatim into each IDE —
	/// for self-contained settings files (menus, templates, inspections, …).
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub files: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Target {
	/// JetBrains config-dir prefix, e.g. "IntelliJIdea", "WebStorm", "RustRover", "AndroidStudio".
	pub product: String,
	/// e.g. "2026.1". If omitted, the highest installed version is used.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub version: Option<String>,
	/// Per-target plugin overrides, merged (unioned) with the top-level `plugins`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub plugins: Option<PluginsCfg>,
	/// IDE-specific verbatim-copied files (e.g. window layouts), applied to THIS
	/// IDE only. Each entry is the IDE-relative destination path; the source lives
	/// at `targets/<product>/<path>` next to the config. Unlike the top-level
	/// `files`, these are not shared across IDEs.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub files: Vec<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct EditorCfg {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub font: Option<FontCfg>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct TerminalCfg {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub font: Option<FontCfg>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct ConsoleCfg {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub font: Option<FontCfg>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct FontCfg {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub family: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub size: Option<f32>,
	#[serde(default, rename = "lineSpacing", skip_serializing_if = "Option::is_none")]
	pub line_spacing: Option<f32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub ligatures: Option<bool>,
	#[serde(default, rename = "regularWeight", skip_serializing_if = "Option::is_none")]
	pub regular_weight: Option<String>,
	#[serde(default, rename = "boldWeight", skip_serializing_if = "Option::is_none")]
	pub bold_weight: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct UiCfg {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub font: Option<FontCfg>,
	/// IDE theme id (LAF), e.g. "ExperimentalDark", "ExperimentalLight", "Darcula".
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub theme: Option<String>,
	#[serde(default, rename = "compactTreeIndents", skip_serializing_if = "Option::is_none")]
	pub compact_tree_indents: Option<bool>,
	#[serde(
		default,
		rename = "mergeMainMenuIntoToolbar",
		skip_serializing_if = "Option::is_none"
	)]
	pub merge_main_menu_into_toolbar: Option<bool>,
	#[serde(default, rename = "contrastScrollbars", skip_serializing_if = "Option::is_none")]
	pub contrast_scrollbars: Option<bool>,
	#[serde(default, rename = "experimentalUi", skip_serializing_if = "Option::is_none")]
	pub experimental_ui: Option<bool>,
	/// Arbitrary registry keys -> string value (escape hatch for power users).
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub registry: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct EditorBehaviorCfg {
	#[serde(default, rename = "softWrap", skip_serializing_if = "Option::is_none")]
	pub soft_wrap: Option<bool>,
	#[serde(default, rename = "showBreadcrumbs", skip_serializing_if = "Option::is_none")]
	pub show_breadcrumbs: Option<bool>,
	#[serde(default, rename = "showStickyLines", skip_serializing_if = "Option::is_none")]
	pub show_sticky_lines: Option<bool>,
	#[serde(default, rename = "ensureNewlineAtEof", skip_serializing_if = "Option::is_none")]
	pub ensure_newline_at_eof: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub emmet: Option<bool>,
	#[serde(default, rename = "postfixTemplates", skip_serializing_if = "Option::is_none")]
	pub postfix_templates: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SchemeRef {
	/// The scheme name as it appears inside the IDE (must match the name inside the file).
	pub name: String,
	/// Path to the `.icls` / code-style `.xml`, relative to the config file.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub file: Option<String>,
	/// Whether to select this scheme as active.
	#[serde(default = "default_true", skip_serializing_if = "is_true")]
	pub activate: bool,
}
fn default_true() -> bool {
	true
}
fn is_true(b: &bool) -> bool {
	*b
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct PluginsCfg {
	/// Marketplace plugin IDs to ensure installed (via the IDE's installPlugins CLI).
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub install: Vec<String>,
	/// Optional custom plugin repository URLs to install from.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub repositories: Vec<String>,
	/// Plugin IDs to disable. Merged (union) with whatever the IDE already disabled.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub disabled: Vec<String>,
}

impl PluginsCfg {
	/// Union the global plugins block with a per-target override (each list
	/// deduplicated + sorted). `None` if neither contributes anything.
	pub fn effective(global: Option<&PluginsCfg>, over: Option<&PluginsCfg>) -> Option<PluginsCfg> {
		let mut install = BTreeSet::new();
		let mut repositories = BTreeSet::new();
		let mut disabled = BTreeSet::new();
		for p in [global, over].into_iter().flatten() {
			install.extend(p.install.iter().cloned());
			repositories.extend(p.repositories.iter().cloned());
			disabled.extend(p.disabled.iter().cloned());
		}
		if install.is_empty() && repositories.is_empty() && disabled.is_empty() {
			return None;
		}
		Some(PluginsCfg {
			install: install.into_iter().collect(),
			repositories: repositories.into_iter().collect(),
			disabled: disabled.into_iter().collect(),
		})
	}
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct VmOptionsCfg {
	#[serde(default, rename = "heapSizeMb", skip_serializing_if = "Option::is_none")]
	pub heap_size_mb: Option<u32>,
	/// Extra raw JVM lines to ensure present, e.g. "-XX:+UseZGC".
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub extra: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct KeymapCfg {
	/// Base name; per-OS variants become "<name> (Linux)", "<name> (macOS)", etc.
	pub name: String,
	#[serde(default = "default_parent", skip_serializing_if = "is_default_parent")]
	pub parent: String,
	/// action id -> keystroke(s). Keystroke syntax: "mod+1", "shift+ctrl+w",
	/// "alt enter". `mod` resolves to Ctrl (Linux/Windows) or Cmd (macOS).
	/// A comma separates a two-stroke chord: "ctrl k, ctrl s".
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub bindings: BTreeMap<String, Binding>,
}
fn default_parent() -> String {
	"$default".to_string()
}
fn is_default_parent(p: &str) -> bool {
	p == "$default"
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Binding {
	One(String),
	Many(Vec<String>),
}

impl Binding {
	pub fn keystrokes(&self) -> Vec<String> {
		match self {
			Binding::One(s) => vec![s.clone()],
			Binding::Many(v) => v.clone(),
		}
	}
}

impl Config {
	pub fn load(path: &Path) -> Result<Config> {
		let text = std::fs::read_to_string(path).with_context(|| format!("reading config {}", path.display()))?;
		let cfg: Config = serde_json::from_str(&text).with_context(|| format!("parsing config {}", path.display()))?;
		Ok(cfg)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn plugins(install: &[&str], disabled: &[&str]) -> PluginsCfg {
		PluginsCfg {
			install: install.iter().map(|s| s.to_string()).collect(),
			repositories: vec![],
			disabled: disabled.iter().map(|s| s.to_string()).collect(),
		}
	}

	#[test]
	fn effective_unions_global_and_target() {
		let global = plugins(&[], &["common.a"]);
		let target = plugins(&["x.install"], &["target.b"]);
		let eff = PluginsCfg::effective(Some(&global), Some(&target)).unwrap();
		assert_eq!(eff.disabled, vec!["common.a", "target.b"]); // sorted union
		assert_eq!(eff.install, vec!["x.install"]);
	}

	#[test]
	fn effective_passes_through_one_side_and_dedups() {
		// only target
		let t = plugins(&[], &["b", "a", "b"]);
		let eff = PluginsCfg::effective(None, Some(&t)).unwrap();
		assert_eq!(eff.disabled, vec!["a", "b"]);
		// nothing at all
		assert!(PluginsCfg::effective(None, None).is_none());
	}

	#[test]
	fn target_plugins_round_trip_through_json() {
		let json = r#"{ "targets": [ { "product": "WebStorm", "plugins": { "install": ["x"] } } ] }"#;
		let cfg: Config = serde_json::from_str(json).unwrap();
		let t = &cfg.targets[0];
		assert_eq!(t.plugins.as_ref().unwrap().install, vec!["x"]);
		// re-serialize keeps it
		let out = serde_json::to_string(&cfg).unwrap();
		assert!(out.contains(r#""plugins":{"install":["x"]}"#));
	}

	#[test]
	fn target_files_round_trip_through_json() {
		let json = r#"{ "targets": [ { "product": "RustRover", "files": ["options/window.layouts.xml"] } ] }"#;
		let cfg: Config = serde_json::from_str(json).unwrap();
		assert_eq!(cfg.targets[0].files, vec!["options/window.layouts.xml"]);
		let out = serde_json::to_string(&cfg).unwrap();
		assert!(out.contains(r#""files":["options/window.layouts.xml"]"#));
		// empty per-target files are omitted from output
		let bare = r#"{ "targets": [ { "product": "RustRover" } ] }"#;
		let cfg2: Config = serde_json::from_str(bare).unwrap();
		assert!(cfg2.targets[0].files.is_empty());
		assert!(!serde_json::to_string(&cfg2).unwrap().contains("files"));
	}
}
