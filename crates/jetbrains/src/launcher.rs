//! Locate the IDE launcher used for `installPlugins`.
//!
//! Search order: `IDESYNC_JB_LAUNCHER` override -> PATH -> Toolbox scripts dir
//! (`<data>/JetBrains/Toolbox/scripts/<name>`). On a Toolbox install the script
//! is named after the product (`idea`, `webstorm`, `rustrover`, `studio`, ...).

use idesync_core::Os;
use std::path::{Path, PathBuf};

/// The launcher base name for a product on a given OS.
pub fn script_name(product: &str, os: Os) -> String {
	let base = match product {
		"IntelliJIdea" => "idea",
		"WebStorm" => "webstorm",
		"RustRover" => "rustrover",
		"PyCharm" => "pycharm",
		"CLion" => "clion",
		"GoLand" => "goland",
		"PhpStorm" => "phpstorm",
		"Rider" => "rider",
		"DataGrip" => "datagrip",
		"RubyMine" => "rubymine",
		"AndroidStudio" => "studio",
		other => return lower_for_os(&other.to_ascii_lowercase(), os),
	};
	lower_for_os(base, os)
}

fn lower_for_os(base: &str, os: Os) -> String {
	match os {
		Os::Windows => format!("{base}64.exe"),
		_ => base.to_string(),
	}
}

/// Resolve a launcher path, reading the environment for overrides.
pub fn find_launcher(product: &str, os: Os) -> Option<PathBuf> {
	let script = script_name(product, os);
	let over = std::env::var_os("IDESYNC_JB_LAUNCHER").map(PathBuf::from);
	let path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
		.map(|p| std::env::split_paths(&p).collect())
		.unwrap_or_default();
	let toolbox_scripts = crate::discovery::jetbrains_data_base()
		.ok()
		.map(|b| b.join("Toolbox").join("scripts"));
	find_launcher_in(&script, over.as_deref(), &path_dirs, toolbox_scripts.as_deref())
}

/// Pure resolution (no env access) so it can be unit-tested deterministically.
pub fn find_launcher_in(
	script: &str,
	override_path: Option<&Path>,
	path_dirs: &[PathBuf],
	toolbox_scripts: Option<&Path>,
) -> Option<PathBuf> {
	if let Some(o) = override_path {
		if o.is_file() {
			return Some(o.to_path_buf());
		}
	}
	for dir in path_dirs {
		let cand = dir.join(script);
		if cand.is_file() {
			return Some(cand);
		}
	}
	if let Some(scripts) = toolbox_scripts {
		let cand = scripts.join(script);
		if cand.is_file() {
			return Some(cand);
		}
	}
	None
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn script_names_per_product_and_os() {
		assert_eq!(script_name("IntelliJIdea", Os::Linux), "idea");
		assert_eq!(script_name("WebStorm", Os::Macos), "webstorm");
		assert_eq!(script_name("IntelliJIdea", Os::Windows), "idea64.exe");
		assert_eq!(script_name("RustRover", Os::Linux), "rustrover");
	}

	#[test]
	fn finds_in_toolbox_scripts_dir() {
		let tmp = tempfile::tempdir().unwrap();
		let scripts = tmp.path().join("Toolbox/scripts");
		std::fs::create_dir_all(&scripts).unwrap();
		let launcher = scripts.join("idea");
		std::fs::write(&launcher, "#!/bin/sh\n").unwrap();

		let found = find_launcher_in("idea", None, &[], Some(&scripts));
		assert_eq!(found.as_deref(), Some(launcher.as_path()));
	}

	#[test]
	fn override_wins_and_missing_returns_none() {
		let tmp = tempfile::tempdir().unwrap();
		let over = tmp.path().join("my-idea");
		std::fs::write(&over, "x").unwrap();
		assert_eq!(
			find_launcher_in("idea", Some(&over), &[], None).as_deref(),
			Some(over.as_path())
		);
		assert!(find_launcher_in("idea", None, &[], None).is_none());
	}
}
