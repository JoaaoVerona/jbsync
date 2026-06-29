//! Generate a JetBrains keymap per target OS from one canonical binding set,
//! and select it as active.
//!
//! Keystroke syntax in the config: modifiers + key joined by `+` or spaces.
//!   - `mod`   -> platform primary modifier (Ctrl on Linux/Windows, Cmd on macOS)
//!   - `ctrl`  -> literal Control on every OS (your muscle-memory shortcuts)
//!   - `meta`/`cmd`/`win`, `alt`/`option`, `shift` -> literal
//!   - a comma separates a two-stroke chord: "ctrl k, ctrl s"
//!   - a `buttonN` token makes it a mouse shortcut: "ctrl+button1",
//!     "button1+doubleClick" -> `<mouse-shortcut>`
//!
//! The active-keymap pointer is written to the per-OS settings subdir
//! (`options/<linux|mac|windows>/keymap.xml`), not `options/keymap.xml`.

use super::{whole_file, Ctx, PatchSet};
use crate::config::{Config, KeymapCfg};
use crate::xmlpatch::ensure;
use anyhow::Result;
use idesync_core::FileChange;
use idesync_core::Os;

pub fn keymap(cfg: &Config, ctx: &Ctx, ps: &mut PatchSet) -> Result<Vec<FileChange>> {
	let Some(km) = cfg.keymap.as_ref() else {
		return Ok(vec![]);
	};
	let os = ctx.target_os;
	let name = keymap_name(&km.name, os);
	let rel = format!("keymaps/{}", keymap_filename(&name));
	let xml = generate(km, os);

	let out = vec![whole_file(&ctx.ide_dir, &rel, xml)];
	// The active-keymap pointer lives in the per-OS settings subdir.
	let active_rel = format!("options/{}/keymap.xml", os.settings_subdir());
	ps.patch(&active_rel, |x| {
		ensure(x, "KeymapManager", "active_keymap", None, &[("name", &name)])
	})?;
	Ok(out)
}

pub fn keymap_name(base: &str, os: Os) -> String {
	format!("{base} ({})", os.label())
}

/// Replicate JetBrains' keymap filename sanitisation: non `[A-Za-z0-9 _.-]`
/// becomes `_`. So "Verona (Linux)" -> "Verona _Linux_.xml".
pub fn keymap_filename(name: &str) -> String {
	let mut out = String::with_capacity(name.len() + 4);
	for c in name.chars() {
		if c.is_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.') {
			out.push(c);
		} else {
			out.push('_');
		}
	}
	out.push_str(".xml");
	out
}

pub fn generate(km: &KeymapCfg, os: Os) -> String {
	let name = keymap_name(&km.name, os);
	let mut s = format!(
		"<keymap version=\"1\" name=\"{}\" parent=\"{}\">\n",
		xml_escape(&name),
		xml_escape(&km.parent)
	);
	for (action, binding) in &km.bindings {
		let strokes: Vec<Stroke> = binding
			.keystrokes()
			.iter()
			.filter_map(|raw| resolve_shortcut(raw, os))
			.collect();
		if strokes.is_empty() {
			// An action with no shortcut explicitly removes the inherited binding.
			s.push_str(&format!("  <action id=\"{}\" />\n", xml_escape(action)));
			continue;
		}
		s.push_str(&format!("  <action id=\"{}\">\n", xml_escape(action)));
		for stroke in &strokes {
			match stroke {
				Stroke::Keyboard {
					first,
					second: Some(sec),
				} => s.push_str(&format!(
					"    <keyboard-shortcut first-keystroke=\"{first}\" second-keystroke=\"{sec}\" />\n"
				)),
				Stroke::Keyboard { first, second: None } => {
					s.push_str(&format!("    <keyboard-shortcut first-keystroke=\"{first}\" />\n"))
				}
				Stroke::Mouse(ks) => s.push_str(&format!("    <mouse-shortcut keystroke=\"{ks}\" />\n")),
			}
		}
		s.push_str("  </action>\n");
	}
	s.push_str("</keymap>");
	s
}

/// A resolved shortcut: a keyboard combo (optionally a two-stroke chord) or a
/// mouse gesture.
enum Stroke {
	Keyboard { first: String, second: Option<String> },
	Mouse(String),
}

fn is_button(tok: &str) -> bool {
	matches!(tok.to_ascii_lowercase().as_str(), "button1" | "button2" | "button3")
}

/// Resolve one shortcut spec. A `buttonN` token makes it a mouse shortcut; a
/// comma separates the two strokes of a keyboard chord ("ctrl+k, ctrl+s").
fn resolve_shortcut(spec: &str, os: Os) -> Option<Stroke> {
	if spec.split([',', '+', ' ']).any(is_button) {
		let ks = resolve_mouse(spec, os);
		return (!ks.is_empty()).then_some(Stroke::Mouse(ks));
	}
	let mut parts = spec.splitn(2, ',');
	let first = resolve_keystroke(parts.next().unwrap_or(spec), os);
	if first.is_empty() {
		return None;
	}
	let second = parts.next().map(|s| resolve_keystroke(s, os)).filter(|s| !s.is_empty());
	Some(Stroke::Keyboard { first, second })
}

/// Build a JetBrains mouse keystroke, e.g. "control button1" or
/// "button1 doubleClick". Mouse modifiers use AWT spelling ("control").
fn resolve_mouse(spec: &str, os: Os) -> String {
	let (mut shift, mut control, mut meta, mut alt, mut dbl) = (false, false, false, false, false);
	let mut button: Option<&str> = None;
	for tok in spec.split(['+', ' ', ',']).filter(|t| !t.is_empty()) {
		match tok.to_ascii_lowercase().as_str() {
			"mod" => match os.primary_modifier() {
				"meta" => meta = true,
				_ => control = true,
			},
			"shift" => shift = true,
			"ctrl" | "control" => control = true,
			"meta" | "cmd" | "command" | "win" | "super" => meta = true,
			"alt" | "option" | "opt" => alt = true,
			"button1" => button = Some("button1"),
			"button2" => button = Some("button2"),
			"button3" => button = Some("button3"),
			"doubleclick" | "double_click" | "double" => dbl = true,
			_ => {}
		}
	}
	let mut parts: Vec<&str> = Vec::new();
	if shift {
		parts.push("shift");
	}
	if control {
		parts.push("control");
	}
	if meta {
		parts.push("meta");
	}
	if alt {
		parts.push("alt");
	}
	if let Some(b) = button {
		parts.push(b);
	}
	if dbl {
		parts.push("doubleClick");
	}
	parts.join(" ")
}

fn resolve_keystroke(spec: &str, os: Os) -> String {
	let mut shift = false;
	let mut ctrl = false;
	let mut meta = false;
	let mut alt = false;
	let mut key: Option<String> = None;

	for tok in spec.split(['+', ' ']).filter(|t| !t.is_empty()) {
		match tok.to_ascii_lowercase().as_str() {
			"mod" => match os.primary_modifier() {
				"meta" => meta = true,
				_ => ctrl = true,
			},
			"ctrl" | "control" => ctrl = true,
			"meta" | "cmd" | "command" | "win" | "super" => meta = true,
			"alt" | "option" | "opt" => alt = true,
			"shift" => shift = true,
			// Preserve the key's original case: JetBrains/AWT named keys are
			// upper-case ("MINUS", "F2", "ENTER") and lower-casing them yields an
			// invalid keystroke. Single letters work either way.
			_ => key = Some(tok.to_string()),
		}
	}

	// JetBrains/AWT modifier order: shift, ctrl, meta, alt, then key.
	let mut parts: Vec<&str> = Vec::new();
	if shift {
		parts.push("shift");
	}
	if ctrl {
		parts.push("ctrl");
	}
	if meta {
		parts.push("meta");
	}
	if alt {
		parts.push("alt");
	}
	let key = key.unwrap_or_default();
	if !key.is_empty() {
		parts.push(&key);
	}
	parts.join(" ")
}

fn xml_escape(s: &str) -> String {
	s.replace('&', "&amp;")
		.replace('<', "&lt;")
		.replace('>', "&gt;")
		.replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::config::Binding;
	use std::collections::BTreeMap;

	fn km() -> KeymapCfg {
		let mut bindings = BTreeMap::new();
		bindings.insert("ReformatCode".to_string(), Binding::One("mod+1".to_string()));
		bindings.insert(
			"ActivateTerminalToolWindow".to_string(),
			Binding::One("ctrl+b".to_string()),
		);
		bindings.insert("CloseAllEditors".to_string(), Binding::One("shift+mod+w".to_string()));
		bindings.insert("CopyElement".to_string(), Binding::Many(vec![]));
		KeymapCfg {
			name: "Verona".to_string(),
			parent: "$default".to_string(),
			bindings,
		}
	}

	#[test]
	fn mod_resolves_to_ctrl_on_linux() {
		let xml = generate(&km(), Os::Linux);
		assert!(xml.contains("name=\"Verona (Linux)\""));
		assert!(xml.contains("<action id=\"ReformatCode\">\n    <keyboard-shortcut first-keystroke=\"ctrl 1\" />"));
		// literal ctrl stays ctrl on linux too
		assert!(xml.contains("first-keystroke=\"ctrl b\""));
		// shift+mod ordering
		assert!(xml.contains("first-keystroke=\"shift ctrl w\""));
		// empty binding removes shortcut
		assert!(xml.contains("<action id=\"CopyElement\" />"));
	}

	#[test]
	fn mod_resolves_to_cmd_on_mac_but_literal_ctrl_stays() {
		let xml = generate(&km(), Os::Macos);
		assert!(xml.contains("name=\"Verona (macOS)\""));
		// mod -> meta (Cmd)
		assert!(xml.contains("first-keystroke=\"meta 1\""));
		// literal ctrl is preserved as ctrl, NOT swapped to cmd
		assert!(xml.contains("first-keystroke=\"ctrl b\""));
		// shift+mod -> shift meta
		assert!(xml.contains("first-keystroke=\"shift meta w\""));
	}

	#[test]
	fn two_stroke_chord_emits_second_keystroke() {
		let mut bindings = BTreeMap::new();
		bindings.insert("ReformatCode".to_string(), Binding::One("ctrl+k, ctrl+s".to_string()));
		let cfg = KeymapCfg {
			name: "V".into(),
			parent: "$default".into(),
			bindings,
		};
		let xml = generate(&cfg, Os::Linux);
		assert!(xml.contains(r#"first-keystroke="ctrl k" second-keystroke="ctrl s""#));

		// `mod` resolves in BOTH strokes of the chord.
		let mut b2 = BTreeMap::new();
		b2.insert("X".to_string(), Binding::One("mod+k, mod+s".to_string()));
		let cfg2 = KeymapCfg {
			name: "V".into(),
			parent: "$default".into(),
			bindings: b2,
		};
		assert!(generate(&cfg2, Os::Macos).contains(r#"first-keystroke="meta k" second-keystroke="meta s""#));
	}

	#[test]
	fn named_keys_keep_their_case() {
		// Materialised default bindings carry AWT named keys ("MINUS", "F2"); they
		// must not be lower-cased into invalid keystrokes.
		let mut bindings = BTreeMap::new();
		bindings.insert("ZoomOut".to_string(), Binding::One("mod+MINUS".to_string()));
		bindings.insert("Rename".to_string(), Binding::One("shift+F6".to_string()));
		let cfg = KeymapCfg {
			name: "V".into(),
			parent: "$default".into(),
			bindings,
		};
		let xml = generate(&cfg, Os::Macos);
		assert!(xml.contains(r#"first-keystroke="meta MINUS""#), "{xml}");
		assert!(xml.contains(r#"first-keystroke="shift F6""#), "{xml}");
	}

	#[test]
	fn filename_matches_jetbrains_sanitisation() {
		assert_eq!(keymap_filename("Verona (Linux)"), "Verona _Linux_.xml");
		assert_eq!(keymap_filename("Verona (macOS)"), "Verona _macOS_.xml");
	}
}
