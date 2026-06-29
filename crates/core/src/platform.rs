//! Operating-system abstraction. The "target OS" is what we generate config
//! *for* — usually the host, but keymaps can be generated for every OS so they
//! can be committed and applied on other machines.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Os {
	Linux,
	Macos,
	Windows,
}

impl Os {
	pub fn host() -> Os {
		if cfg!(target_os = "macos") {
			Os::Macos
		} else if cfg!(target_os = "windows") {
			Os::Windows
		} else {
			Os::Linux
		}
	}

	/// Label used inside JetBrains keymap names, e.g. "Verona (Linux)".
	pub fn label(self) -> &'static str {
		match self {
			Os::Linux => "Linux",
			Os::Macos => "macOS",
			Os::Windows => "Windows",
		}
	}

	/// JetBrains' per-OS roamable settings subdir under `options/`
	/// (e.g. the active keymap lives in `options/<subdir>/keymap.xml`).
	pub fn settings_subdir(self) -> &'static str {
		match self {
			Os::Linux => "linux",
			Os::Macos => "mac",
			Os::Windows => "windows",
		}
	}

	pub fn parse(s: &str) -> Option<Os> {
		match s.to_ascii_lowercase().as_str() {
			"linux" => Some(Os::Linux),
			"mac" | "macos" | "osx" | "darwin" => Some(Os::Macos),
			"win" | "windows" => Some(Os::Windows),
			_ => None,
		}
	}

	pub const ALL: [Os; 3] = [Os::Linux, Os::Macos, Os::Windows];

	/// The platform-native primary modifier (the `mod` token) in JetBrains
	/// keystroke syntax: Cmd on macOS, Ctrl elsewhere.
	pub fn primary_modifier(self) -> &'static str {
		match self {
			Os::Macos => "meta",
			_ => "ctrl",
		}
	}
}

impl fmt::Display for Os {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.label())
	}
}
