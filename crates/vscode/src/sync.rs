//! VSCode-family support — the second editor family alongside JetBrains.
//!
//! Where JetBrains config is surgically-patched XML keyed by `<Product><Version>`
//! dirs, VSCode editors all share one layout: a single per-install user dir
//! (`<config>/<AppDir>/User/`) holding `settings.json` + `keybindings.json`
//! (JSONC), with extensions installed via a CLI. idesync treats this as
//! pass-through: `settings.json` keys are merged surgically (other keys/comments
//! preserved), `keybindings.json` is owned wholesale (like a generated keymap),
//! and extensions are ensure-installed via `<cli> --install-extension`.
//!
//! `IDESYNC_VSC_CONFIG_HOME` overrides the base dir that holds the per-editor
//! `<AppDir>` folders; `IDESYNC_VSC_HOME` overrides the home dir used to find
//! per-editor `extensions/` (both for tests / non-standard installs).

use crate::config::VsCodeCfg;
use crate::jsonc;
use anyhow::{Context, Result};
use idesync_core::FileChange;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Ensure-install VSCode extensions via the editor CLI (`code
/// --install-extension <id>`). The VSCode-family counterpart to a JetBrains
/// plugin install: only the IDs not already present are listed.
#[derive(Debug, Clone)]
pub struct ExtensionInstall {
	/// The editor's display name (e.g. "Code", "VSCodium").
	pub editor: String,
	/// The CLI command name (e.g. "code"), for the not-found error message.
	pub cli_name: String,
	/// Resolved CLI path on PATH, or None if it could not be found.
	pub cli: Option<PathBuf>,
	pub ids: Vec<String>,
}

impl ExtensionInstall {
	/// Arguments after the CLI binary: `--install-extension <id>` per id.
	pub fn args(&self) -> Vec<String> {
		let mut a = Vec::with_capacity(self.ids.len() * 2);
		for id in &self.ids {
			a.push("--install-extension".to_string());
			a.push(id.clone());
		}
		a
	}

	pub fn command_display(&self) -> String {
		let cli = self
			.cli
			.as_ref()
			.map(|p| p.display().to_string())
			.unwrap_or_else(|| self.cli_name.clone());
		format!("{cli} {}", self.args().join(" "))
	}
}

/// A VSCode-family editor: where its config lives and how to drive its CLI.
pub struct Family {
	/// Display name, also the value used in `vscode.targets` and `idesync list`.
	pub key: &'static str,
	/// Config sub-dir under the OS config base (`<base>/<app_dir>/User`).
	pub app_dir: &'static str,
	/// CLI command (looked up on PATH) for `--install-extension`.
	pub cli: &'static str,
	/// Home-relative extensions dir, used to detect already-installed extensions.
	pub ext_dir: &'static str,
}

/// Editors that reuse the VSCode config layout. Order is the discovery order.
pub const FAMILIES: &[Family] = &[
	Family {
		key: "Code",
		app_dir: "Code",
		cli: "code",
		ext_dir: ".vscode/extensions",
	},
	Family {
		key: "Code - Insiders",
		app_dir: "Code - Insiders",
		cli: "code-insiders",
		ext_dir: ".vscode-insiders/extensions",
	},
	Family {
		key: "VSCodium",
		app_dir: "VSCodium",
		cli: "codium",
		ext_dir: ".vscode-oss/extensions",
	},
	Family {
		key: "Cursor",
		app_dir: "Cursor",
		cli: "cursor",
		ext_dir: ".cursor/extensions",
	},
	Family {
		key: "Windsurf",
		app_dir: "Windsurf",
		cli: "windsurf",
		ext_dir: ".windsurf/extensions",
	},
];

/// True if `name` is a known VSCode-family editor (so `--product` routes here).
pub fn is_family(name: &str) -> bool {
	FAMILIES.iter().any(|f| f.key == name)
}

/// The known editor keys (for help/error messages).
pub fn family_keys() -> Vec<&'static str> {
	FAMILIES.iter().map(|f| f.key).collect()
}

pub fn family(name: &str) -> Option<&'static Family> {
	FAMILIES.iter().find(|f| f.key == name)
}

/// The base dir holding the per-editor `<AppDir>` folders (overridable).
fn config_base() -> Result<PathBuf> {
	if let Ok(over) = std::env::var("IDESYNC_VSC_CONFIG_HOME") {
		return Ok(PathBuf::from(over));
	}
	dirs::config_dir().context("cannot determine OS config dir")
}

/// The home dir used to resolve per-editor `extensions/` (overridable).
fn home_base() -> Option<PathBuf> {
	std::env::var_os("IDESYNC_VSC_HOME")
		.map(PathBuf::from)
		.or_else(dirs::home_dir)
}

/// The `User` config dir for an editor (where settings/keybindings live).
pub fn user_dir(fam: &Family) -> Result<PathBuf> {
	Ok(config_base()?.join(fam.app_dir).join("User"))
}

/// True if this editor appears installed (its `<AppDir>` config folder exists).
pub fn is_installed(fam: &Family) -> bool {
	config_base().map(|b| b.join(fam.app_dir).is_dir()).unwrap_or(false)
}

/// Every discovered VSCode-family editor (its config folder is present).
pub fn discover() -> Vec<&'static Family> {
	FAMILIES.iter().filter(|f| is_installed(f)).collect()
}

/// The editors to act on for one run.
///
/// - `Some(product)` that names a family → just that editor.
/// - else with `cfg.targets` set → those editors (intersected with `targets_only`
///   semantics handled by the caller); otherwise every discovered editor, unioned
///   with any explicitly-configured `targets`.
pub fn resolve_editors(vc: &VsCodeCfg, product: Option<&str>, targets_only: bool) -> Vec<&'static Family> {
	if let Some(p) = product {
		return family(p).into_iter().collect();
	}
	if vc.targets.is_empty() {
		return discover();
	}
	let configured: Vec<&'static Family> = vc.targets.iter().filter_map(|t| family(t)).collect();
	if targets_only {
		return configured;
	}
	// Union configured (kept even if not currently installed) with discovered.
	let mut out = configured;
	for f in discover() {
		if !out.iter().any(|c| c.key == f.key) {
			out.push(f);
		}
	}
	out
}

/// The declarative + imperative work for one VSCode editor.
pub struct VsPlan {
	pub files: Vec<FileChange>,
	pub installs: Vec<ExtensionInstall>,
}

impl VsPlan {
	pub fn is_empty(&self) -> bool {
		self.files.is_empty() && self.installs.is_empty()
	}

	pub fn change_count(&self) -> usize {
		self.files.len() + self.installs.len()
	}
}

/// Compute changes for one editor: settings.json merge, keybindings.json
/// (owned), and any missing-extension installs.
pub fn build_plan(vc: &VsCodeCfg, fam: &Family) -> Result<VsPlan> {
	let dir = user_dir(fam)?;
	let mut files = Vec::new();

	if !vc.settings.is_empty() {
		let path = dir.join("settings.json");
		let old = std::fs::read_to_string(&path).ok();
		let new = jsonc::merge_settings(old.as_deref().unwrap_or(""), &vc.settings)
			.with_context(|| format!("patching {}", path.display()))?;
		files.push(file_change(path, "settings.json", old, new));
	}

	if let Some(bindings) = &vc.keybindings {
		let path = dir.join("keybindings.json");
		let old = std::fs::read_to_string(&path).ok();
		// Expand the `mod` token (Ctrl on Linux/Windows, Cmd on macOS) before owning the file.
		let expanded = crate::keymap::expand(bindings);
		let new = render_keybindings(&expanded)?;
		files.push(file_change(path, "keybindings.json", old, new));
	}

	let installs = vc
		.extensions
		.as_ref()
		.map(|ext| plan_extensions(fam, &ext.install))
		.transpose()?
		.flatten()
		.into_iter()
		.collect();

	Ok(VsPlan {
		files: files.into_iter().filter(FileChange::is_change).collect(),
		installs,
	})
}

fn file_change(path: PathBuf, rel: &str, old: Option<String>, new: String) -> FileChange {
	FileChange {
		path,
		rel: rel.to_string(),
		old,
		new,
	}
}

/// idesync owns `keybindings.json`, so render the whole array deterministically
/// with a managed-by header (4-space indent, trailing newline).
fn render_keybindings(bindings: &[Value]) -> Result<String> {
	let body = serde_json::to_string_pretty(bindings)?;
	Ok(format!(
		"// Managed by idesync — edit your idesync config, not this file.\n{body}\n"
	))
}

/// An [`ExtensionInstall`] for the IDs not already present, or None if all are
/// installed (or none requested).
fn plan_extensions(fam: &Family, want: &[String]) -> Result<Option<ExtensionInstall>> {
	if want.is_empty() {
		return Ok(None);
	}
	let installed = installed_extensions(fam);
	let missing: Vec<String> = want
		.iter()
		.filter(|id| !installed.contains(&id.to_ascii_lowercase()))
		.cloned()
		.collect();
	if missing.is_empty() {
		return Ok(None);
	}
	Ok(Some(ExtensionInstall {
		editor: fam.key.to_string(),
		cli_name: fam.cli.to_string(),
		cli: find_on_path(fam.cli),
		ids: missing,
	}))
}

/// Extension IDs (`publisher.name`, original casing, sorted/deduped) installed
/// for this editor, read from the `extensions/extensions.json` manifest VSCode
/// maintains. Used by `create` to capture the installed set.
pub fn installed_extension_ids(fam: &Family) -> Vec<String> {
	let Some(home) = home_base() else {
		return vec![];
	};
	let manifest = home.join(fam.ext_dir).join("extensions.json");
	let Ok(text) = std::fs::read_to_string(&manifest) else {
		return vec![];
	};
	let mut out = BTreeSet::new();
	if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&text) {
		for it in items {
			if let Some(id) = it.get("identifier").and_then(|i| i.get("id")).and_then(Value::as_str) {
				out.insert(id.to_string());
			}
		}
	}
	out.into_iter().collect()
}

/// Lower-cased installed-extension IDs, for case-insensitive "already present?"
/// checks during apply (Marketplace IDs are case-insensitive).
pub fn installed_extensions(fam: &Family) -> BTreeSet<String> {
	installed_extension_ids(fam)
		.iter()
		.map(|id| id.to_ascii_lowercase())
		.collect()
}

/// Find an executable named `cmd` on PATH (trying `.cmd`/`.exe` on Windows).
fn find_on_path(cmd: &str) -> Option<PathBuf> {
	let dirs: Vec<PathBuf> = std::env::var_os("PATH")
		.map(|p| std::env::split_paths(&p).collect())
		.unwrap_or_default();
	let candidates: &[String] = &if cfg!(windows) {
		vec![format!("{cmd}.cmd"), format!("{cmd}.exe"), cmd.to_string()]
	} else {
		vec![cmd.to_string()]
	};
	for dir in &dirs {
		for name in candidates {
			let cand = dir.join(name);
			if cand.is_file() {
				return Some(cand);
			}
		}
	}
	None
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn resolve_prefers_explicit_product() {
		let vc = VsCodeCfg::default();
		let got = resolve_editors(&vc, Some("VSCodium"), false);
		assert_eq!(got.iter().map(|f| f.key).collect::<Vec<_>>(), vec!["VSCodium"]);
		// an unknown / JetBrains product resolves to no VSCode editors
		assert!(resolve_editors(&vc, Some("RustRover"), false).is_empty());
	}

	#[test]
	fn targets_only_restricts_to_configured() {
		let vc = VsCodeCfg {
			targets: vec!["Code".into(), "Cursor".into(), "Bogus".into()],
			..Default::default()
		};
		let got = resolve_editors(&vc, None, true);
		// "Bogus" is not a known family and is dropped; the rest are kept.
		assert_eq!(got.iter().map(|f| f.key).collect::<Vec<_>>(), vec!["Code", "Cursor"]);
	}

	#[test]
	fn render_keybindings_is_deterministic_with_header() {
		let out = render_keybindings(&[json!({"key": "ctrl+s", "command": "save"})]).unwrap();
		assert!(out.starts_with("// Managed by idesync"));
		assert!(out.contains("\"key\": \"ctrl+s\""));
		assert!(out.ends_with("\n"));
	}

	#[test]
	fn installed_extensions_reads_manifest() {
		let tmp = tempfile::tempdir().unwrap();
		let ext = tmp.path().join(".vscode/extensions");
		std::fs::create_dir_all(&ext).unwrap();
		std::fs::write(
			ext.join("extensions.json"),
			r#"[{"identifier":{"id":"Rust-Lang.Rust-Analyzer"}},{"identifier":{"id":"esbenp.prettier-vscode"}}]"#,
		)
		.unwrap();
		std::env::set_var("IDESYNC_VSC_HOME", tmp.path());
		let code = family("Code").unwrap();
		let installed = installed_extensions(code);
		std::env::remove_var("IDESYNC_VSC_HOME");
		assert!(installed.contains("rust-lang.rust-analyzer"));
		assert!(installed.contains("esbenp.prettier-vscode"));
	}
}
