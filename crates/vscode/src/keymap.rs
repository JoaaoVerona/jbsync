//! The `mod` token for VSCode keybindings — the VSCode analog of the JetBrains
//! `mod` modifier (Ctrl on Linux/Windows, Cmd on macOS).
//!
//! VSCode `keybindings.json` is a single cross-platform file: an entry's `key`
//! applies everywhere, and an optional `mac`/`linux`/`win` field overrides it on
//! that platform. So idesync doesn't generate per-OS files like it does for
//! JetBrains — instead it lets you write `"key": "mod+d"` once and **expands** it
//! on apply into `"key": "ctrl+d"` + `"mac": "cmd+d"`, which VSCode reads as
//! Ctrl on Linux/Windows and Cmd on macOS.
//!
//! [`expand`] runs on every apply (`mod` is just config syntax idesync resolves).
//! [`collapse`] is the reverse, used by `create --portable-keymap` to fold a
//! captured `ctrl` key + matching `cmd` mac override back into `mod`.

use serde_json::Value;

/// Expand every `mod` token in a keybindings array (apply direction).
pub fn expand(bindings: &[Value]) -> Vec<Value> {
	bindings.iter().map(expand_one).collect()
}

/// Collapse `ctrl`-key + `cmd`-mac pairs back into `mod` (capture direction).
pub fn collapse(bindings: &[Value]) -> Vec<Value> {
	bindings.iter().map(collapse_one).collect()
}

fn expand_one(entry: &Value) -> Value {
	let Value::Object(map) = entry else {
		return entry.clone();
	};
	let mut out = map.clone();
	let orig_key = map.get("key").and_then(Value::as_str).map(str::to_string);
	let key_has_mod = orig_key.as_deref().map(has_mod).unwrap_or(false);
	let had_mac = map.contains_key("mac");

	// Resolve `mod` in each platform field: the mac override → cmd, the rest → ctrl.
	for field in ["key", "linux", "win"] {
		if let Some(s) = out.get(field).and_then(Value::as_str) {
			if has_mod(s) {
				out.insert(field.to_string(), Value::String(replace_mod(s, "ctrl")));
			}
		}
	}
	if let Some(s) = out.get("mac").and_then(Value::as_str) {
		if has_mod(s) {
			out.insert("mac".to_string(), Value::String(replace_mod(s, "cmd")));
		}
	}

	// When `key` used `mod` and there's no explicit mac override, add the cmd
	// variant so macOS gets Cmd while Linux/Windows fall through to the ctrl `key`.
	if key_has_mod && !had_mac {
		if let Some(k) = orig_key {
			out.insert("mac".to_string(), Value::String(replace_mod(&k, "cmd")));
		}
	}
	Value::Object(out)
}

fn collapse_one(entry: &Value) -> Value {
	let Value::Object(map) = entry else {
		return entry.clone();
	};
	let (Some(key), Some(mac)) = (
		map.get("key").and_then(Value::as_str),
		map.get("mac").and_then(Value::as_str),
	) else {
		return entry.clone();
	};
	// Only collapse when the mac override is *exactly* the cmd-variant of a
	// ctrl-bearing key — otherwise the mac binding is a genuine, different override.
	if has_ctrl(key) && mac == replace_ctrl(key, "cmd") {
		let mut out = map.clone();
		out.insert("key".to_string(), Value::String(replace_ctrl(key, "mod")));
		out.remove("mac");
		Value::Object(out)
	} else {
		entry.clone()
	}
}

/// Replace whole `mod` modifier tokens with `primary` (e.g. "ctrl"/"cmd").
fn replace_mod(key: &str, primary: &str) -> String {
	map_tokens(key, |seg| seg.eq_ignore_ascii_case("mod").then(|| primary.to_string()))
}

/// Replace whole `ctrl`/`control` modifier tokens with `repl`.
fn replace_ctrl(key: &str, repl: &str) -> String {
	map_tokens(key, |seg| {
		(seg.eq_ignore_ascii_case("ctrl") || seg.eq_ignore_ascii_case("control")).then(|| repl.to_string())
	})
}

fn has_mod(key: &str) -> bool {
	has_token(key, &["mod"])
}

fn has_ctrl(key: &str) -> bool {
	has_token(key, &["ctrl", "control"])
}

fn has_token(key: &str, names: &[&str]) -> bool {
	key.split([' ', '+'])
		.any(|seg| names.iter().any(|n| seg.eq_ignore_ascii_case(n)))
}

/// Apply `f` to each `+`-joined modifier/key token, across space-separated chord
/// parts, preserving the original separators. `f` returns `Some(replacement)` to
/// rewrite a token or `None` to keep it (so key names are never touched).
fn map_tokens(key: &str, f: impl Fn(&str) -> Option<String>) -> String {
	key.split(' ')
		.map(|chord| {
			chord
				.split('+')
				.map(|seg| f(seg).unwrap_or_else(|| seg.to_string()))
				.collect::<Vec<_>>()
				.join("+")
		})
		.collect::<Vec<_>>()
		.join(" ")
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn expands_mod_key_into_ctrl_plus_cmd_mac() {
		let out = expand(&[json!({ "key": "mod+d", "command": "x" })]);
		assert_eq!(out[0], json!({ "key": "ctrl+d", "mac": "cmd+d", "command": "x" }));
	}

	#[test]
	fn expands_chords_and_multiple_mods() {
		let out = expand(&[json!({ "key": "mod+k mod+s", "command": "x" })]);
		assert_eq!(
			out[0],
			json!({ "key": "ctrl+k ctrl+s", "mac": "cmd+k cmd+s", "command": "x" })
		);
	}

	#[test]
	fn literal_ctrl_is_left_alone() {
		let entry = json!({ "key": "ctrl+d", "command": "x" });
		assert_eq!(
			expand(std::slice::from_ref(&entry))[0],
			entry,
			"no mod token → untouched"
		);
	}

	#[test]
	fn explicit_mac_override_is_respected_not_clobbered() {
		let out = expand(&[json!({ "key": "mod+d", "mac": "cmd+shift+d", "command": "x" })]);
		assert_eq!(
			out[0],
			json!({ "key": "ctrl+d", "mac": "cmd+shift+d", "command": "x" }),
			"key resolves to ctrl; user's mac override kept"
		);
	}

	#[test]
	fn mod_only_in_mac_resolves_to_cmd() {
		let out = expand(&[json!({ "key": "alt+d", "mac": "mod+d", "command": "x" })]);
		assert_eq!(out[0], json!({ "key": "alt+d", "mac": "cmd+d", "command": "x" }));
	}

	#[test]
	fn mod_is_a_whole_token_not_a_substring() {
		// "model" must not be mangled; only the standalone `mod` modifier expands.
		let out = expand(&[json!({ "key": "mod+m", "command": "openModel" })]);
		assert_eq!(
			out[0],
			json!({ "key": "ctrl+m", "mac": "cmd+m", "command": "openModel" })
		);
	}

	#[test]
	fn collapse_folds_ctrl_and_matching_cmd_into_mod() {
		let out = collapse(&[json!({ "key": "ctrl+d", "mac": "cmd+d", "command": "x" })]);
		assert_eq!(out[0], json!({ "key": "mod+d", "command": "x" }));
	}

	#[test]
	fn collapse_leaves_genuine_different_mac_override() {
		let entry = json!({ "key": "ctrl+d", "mac": "cmd+shift+d", "command": "x" });
		assert_eq!(
			collapse(std::slice::from_ref(&entry))[0],
			entry,
			"mac isn't the cmd-variant → keep"
		);
	}

	#[test]
	fn expand_then_collapse_round_trips() {
		let original = json!({ "key": "mod+k mod+s", "command": "x" });
		let expanded = expand(std::slice::from_ref(&original));
		let collapsed = collapse(&expanded);
		assert_eq!(collapsed[0], original);
	}

	#[test]
	fn expand_is_idempotent_on_already_expanded() {
		// A second apply (config still says mod) must produce the same file bytes.
		let once = expand(&[json!({ "key": "mod+d", "command": "x" })]);
		let twice = expand(&once);
		assert_eq!(once, twice);
	}
}
