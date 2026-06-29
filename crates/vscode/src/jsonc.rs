//! Minimal JSONC (JSON-with-comments) support, the VSCode counterpart to
//! `xmlpatch.rs`: byte-minimal, comment-preserving edits to `settings.json`.
//!
//! VSCode `settings.json` is JSONC — it allows `//` / `/* */` comments and
//! trailing commas, and mixes user-managed keys with machine-local ones
//! (telemetry opt-outs, window state, absolute paths). So we never re-serialise
//! the whole file: [`merge_settings`] sets only the requested TOP-LEVEL keys and
//! leaves every other byte — comments, key order, spacing — exactly as it was.
//!
//! Settings keys are dotted strings (`"editor.fontSize"`) but those are *single*
//! top-level keys, not nested paths, so a top-level merge covers real-world use.
//! A key whose value is an object/array is replaced wholesale (the config owns
//! that value); idesync does not deep-merge into it.

use anyhow::{bail, Result};
use serde::Serialize;
use serde_json::{Map, Value};

/// Parse JSONC text into a [`Value`], tolerating comments and trailing commas.
/// An empty / whitespace-only document parses to an empty object.
pub fn parse(text: &str) -> Result<Value> {
	if text.trim().is_empty() {
		return Ok(Value::Object(Map::new()));
	}
	let stripped = strip_to_plain_json(text);
	Ok(serde_json::from_str(&stripped)?)
}

/// Set each `managed` key at the top level of `original`, preserving all other
/// content (comments, untouched keys, formatting). Returns the new file text.
///
/// Idempotent: a key already present with an equal serialised value is left
/// untouched, so `apply` then `check` agree.
pub fn merge_settings(original: &str, managed: &Map<String, Value>) -> Result<String> {
	if managed.is_empty() {
		return Ok(original.to_string());
	}
	// Seed an empty/whitespace-only file with a bare object to insert into.
	let fresh = original.trim().is_empty();
	let base = if fresh { "{}".to_string() } else { original.to_string() };
	let b = base.as_bytes();
	let Some(scan) = scan_object(b) else {
		bail!("settings.json is not a JSON object idesync can patch (parse failed)");
	};

	let indent = detect_indent(&base, &scan);
	// (start, end, replacement) edits over existing keys, applied back-to-front.
	let mut edits: Vec<(usize, usize, String)> = Vec::new();
	let mut missing: Vec<(&String, &Value)> = Vec::new();
	for (k, v) in managed {
		match scan.props.iter().find(|p| &p.key == k) {
			Some(p) => {
				let new_val = serialize_value(v, &indent, prop_indent(&base, p));
				if base[p.value_start..p.value_end] != new_val {
					edits.push((p.value_start, p.value_end, new_val));
				}
			}
			None => missing.push((k, v)),
		}
	}

	if !missing.is_empty() {
		if let Some(last) = scan.props.last() {
			// Append after the final value: `,\n<indent>"k": v` for each new key.
			let base_indent = prop_indent(&base, last).to_string();
			let mut ins = String::new();
			for (k, v) in &missing {
				ins.push(',');
				ins.push('\n');
				ins.push_str(&base_indent);
				ins.push_str(&key_value(k, v, &indent, &base_indent));
			}
			edits.push((last.value_end, last.value_end, ins));
		} else {
			// Empty object `{}` (or `{ }`/`{\n}`): rebuild it with the new keys.
			let mut body = String::from("{\n");
			for (i, (k, v)) in missing.iter().enumerate() {
				if i > 0 {
					body.push_str(",\n");
				}
				body.push_str(&indent);
				body.push_str(&key_value(k, v, &indent, &indent));
			}
			body.push_str("\n}");
			edits.push((scan.open, scan.close + 1, body));
		}
	}

	if edits.is_empty() {
		return Ok(original.to_string());
	}

	// Apply edits high-offset-first so earlier spans stay valid.
	edits.sort_by(|a, b| b.0.cmp(&a.0));
	let mut out = base.clone();
	for (start, end, text) in edits {
		out.replace_range(start..end, &text);
	}
	// A freshly seeded file gets a trailing newline; an existing file keeps its own.
	if fresh && !out.ends_with('\n') {
		out.push('\n');
	}
	Ok(out)
}

/// `"key": value` with the value re-indented to `base_indent`.
fn key_value(key: &str, value: &Value, indent: &str, base_indent: &str) -> String {
	format!(
		"{}: {}",
		serde_json::to_string(key).unwrap(),
		serialize_value(value, indent, base_indent)
	)
}

/// Pretty-serialise a value with `indent` as the unit, re-indenting continuation
/// lines (objects/arrays) so they sit under a property at `base_indent`.
fn serialize_value(value: &Value, indent: &str, base_indent: &str) -> String {
	let mut buf = Vec::new();
	let fmt = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
	let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
	value.serialize(&mut ser).expect("serialize json value");
	let s = String::from_utf8(buf).expect("json is utf8");
	let mut out = String::with_capacity(s.len());
	for (i, line) in s.split('\n').enumerate() {
		if i > 0 {
			out.push('\n');
			out.push_str(base_indent);
		}
		out.push_str(line);
	}
	out
}

/// Leading whitespace of the line a property starts on (its indentation).
fn prop_indent<'a>(base: &'a str, p: &Prop) -> &'a str {
	let line_start = base[..p.key_start].rfind('\n').map(|i| i + 1).unwrap_or(0);
	&base[line_start..p.key_start]
}

/// The file's indent unit: the indentation of the first property, else 4 spaces.
fn detect_indent(base: &str, scan: &Scan) -> String {
	match scan.props.first() {
		Some(p) => {
			let ind = prop_indent(base, p);
			if ind.is_empty() {
				"    ".to_string()
			} else {
				ind.to_string()
			}
		}
		None => "    ".to_string(),
	}
}

// --- scanning ---------------------------------------------------------------

struct Prop {
	key: String,
	key_start: usize,
	value_start: usize,
	value_end: usize,
}

struct Scan {
	open: usize,
	close: usize,
	props: Vec<Prop>,
}

/// Scan the outermost top-level object: its `{`/`}` offsets and each top-level
/// property's key + value span. Returns `None` if the doc isn't an object or is
/// malformed enough that we shouldn't risk editing it.
fn scan_object(b: &[u8]) -> Option<Scan> {
	let mut i = skip_ws_comments(b, 0);
	if b.get(i) != Some(&b'{') {
		return None;
	}
	let open = i;
	i += 1;
	let mut props = Vec::new();
	loop {
		i = skip_ws_comments(b, i);
		match b.get(i) {
			Some(b'}') => return Some(Scan { open, close: i, props }),
			Some(b'"') => {
				let key_start = i;
				let key_end = scan_string(b, i);
				let key = serde_json::from_slice::<String>(&b[key_start..key_end]).ok()?;
				i = skip_ws_comments(b, key_end);
				if b.get(i) != Some(&b':') {
					return None;
				}
				i = skip_ws_comments(b, i + 1);
				let value_start = i;
				let value_end = scan_value(b, i);
				if value_end == value_start {
					return None;
				}
				props.push(Prop {
					key,
					key_start,
					value_start,
					value_end,
				});
				i = skip_ws_comments(b, value_end);
				if b.get(i) == Some(&b',') {
					i += 1;
				}
			}
			_ => return None,
		}
	}
}

/// Advance past JSON whitespace and `//` / `/* */` comments.
fn skip_ws_comments(b: &[u8], mut i: usize) -> usize {
	loop {
		while i < b.len() && b[i].is_ascii_whitespace() {
			i += 1;
		}
		if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'/' {
			i += 2;
			while i < b.len() && b[i] != b'\n' {
				i += 1;
			}
		} else if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
			i += 2;
			while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
				i += 1;
			}
			i = (i + 2).min(b.len());
		} else {
			return i;
		}
	}
}

/// `b[i]` is `"`; return the index just past the closing quote.
fn scan_string(b: &[u8], i: usize) -> usize {
	let mut j = i + 1;
	while j < b.len() {
		match b[j] {
			b'\\' => j += 2,
			b'"' => return j + 1,
			_ => j += 1,
		}
	}
	j
}

/// `b[i]` opens a value (already past whitespace); return the index just past it.
fn scan_value(b: &[u8], i: usize) -> usize {
	match b.get(i) {
		Some(b'"') => scan_string(b, i),
		Some(b'{') | Some(b'[') => scan_balanced(b, i),
		_ => {
			// Scalar (number / true / false / null): up to a delimiter or comment.
			let mut j = i;
			while j < b.len() {
				let c = b[j];
				if c.is_ascii_whitespace() || c == b',' || c == b'}' || c == b']' {
					break;
				}
				if c == b'/' && j + 1 < b.len() && (b[j + 1] == b'/' || b[j + 1] == b'*') {
					break;
				}
				j += 1;
			}
			j
		}
	}
}

/// `b[i]` is `{` or `[`; return the index just past the matching close, honoring
/// nested strings and comments.
fn scan_balanced(b: &[u8], i: usize) -> usize {
	let mut depth = 0i32;
	let mut j = i;
	while j < b.len() {
		match b[j] {
			b'"' => {
				j = scan_string(b, j);
				continue;
			}
			b'/' if j + 1 < b.len() && b[j + 1] == b'/' => {
				j += 2;
				while j < b.len() && b[j] != b'\n' {
					j += 1;
				}
				continue;
			}
			b'/' if j + 1 < b.len() && b[j + 1] == b'*' => {
				j += 2;
				while j + 1 < b.len() && !(b[j] == b'*' && b[j + 1] == b'/') {
					j += 1;
				}
				j = (j + 2).min(b.len());
				continue;
			}
			b'{' | b'[' => depth += 1,
			b'}' | b']' => {
				depth -= 1;
				if depth == 0 {
					return j + 1;
				}
			}
			_ => {}
		}
		j += 1;
	}
	j
}

/// Strip comments and trailing commas so `serde_json` can parse the remainder.
/// Comment bytes are replaced with spaces to keep byte offsets and string
/// contents intact.
fn strip_to_plain_json(text: &str) -> String {
	let b = text.as_bytes();
	let mut out: Vec<u8> = Vec::with_capacity(b.len());
	let mut i = 0;
	while i < b.len() {
		match b[i] {
			b'"' => {
				let end = scan_string(b, i);
				out.extend_from_slice(&b[i..end]);
				i = end;
			}
			b'/' if i + 1 < b.len() && b[i + 1] == b'/' => {
				while i < b.len() && b[i] != b'\n' {
					out.push(b' ');
					i += 1;
				}
			}
			b'/' if i + 1 < b.len() && b[i + 1] == b'*' => {
				while i < b.len() && !(b[i] == b'*' && i + 1 < b.len() && b[i + 1] == b'/') {
					out.push(if b[i] == b'\n' { b'\n' } else { b' ' });
					i += 1;
				}
				// the closing `*/`
				out.push(b' ');
				out.push(b' ');
				i = (i + 2).min(b.len());
			}
			b',' => {
				// Drop a comma that only precedes whitespace/comments then `}`/`]`.
				let next = skip_ws_comments(b, i + 1);
				if matches!(b.get(next), Some(b'}') | Some(b']')) {
					out.push(b' ');
				} else {
					out.push(b',');
				}
				i += 1;
			}
			c => {
				out.push(c);
				i += 1;
			}
		}
	}
	String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	fn managed(pairs: &[(&str, Value)]) -> Map<String, Value> {
		pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
	}

	#[test]
	fn parse_tolerates_comments_and_trailing_commas() {
		let text = r#"{
			// editor
			"editor.fontSize": 14, /* inline */
			"editor.tabSize": 2,
		}"#;
		let v = parse(text).unwrap();
		assert_eq!(v["editor.fontSize"], json!(14));
		assert_eq!(v["editor.tabSize"], json!(2));
	}

	#[test]
	fn empty_document_is_empty_object() {
		assert_eq!(parse("   \n").unwrap(), json!({}));
		assert_eq!(
			merge_settings("", &managed(&[("a", json!(1))])).unwrap(),
			"{\n    \"a\": 1\n}\n"
		);
	}

	#[test]
	fn replaces_existing_value_in_place_preserving_comments() {
		let text = "{\n\t// my font\n\t\"editor.fontSize\": 12,\n\t\"editor.tabSize\": 4\n}\n";
		let out = merge_settings(text, &managed(&[("editor.fontSize", json!(16))])).unwrap();
		assert_eq!(
			out,
			"{\n\t// my font\n\t\"editor.fontSize\": 16,\n\t\"editor.tabSize\": 4\n}\n"
		);
	}

	#[test]
	fn inserts_missing_key_after_last_and_keeps_indent() {
		let text = "{\n    \"a\": 1\n}\n";
		let out = merge_settings(text, &managed(&[("b", json!("x"))])).unwrap();
		assert_eq!(out, "{\n    \"a\": 1,\n    \"b\": \"x\"\n}\n");
	}

	#[test]
	fn idempotent_when_already_set() {
		let text = "{\n    \"a\": 1,\n    \"b\": 2\n}\n";
		let once = merge_settings(text, &managed(&[("a", json!(1))])).unwrap();
		assert_eq!(once, text, "no change when value already equal");
		let twice = merge_settings(&once, &managed(&[("a", json!(1))])).unwrap();
		assert_eq!(twice, once);
	}

	#[test]
	fn insert_then_reapply_is_stable() {
		let text = "{\n    \"a\": 1\n}\n";
		let once = merge_settings(text, &managed(&[("obj", json!({"x": 1, "y": [1, 2]}))])).unwrap();
		let twice = merge_settings(&once, &managed(&[("obj", json!({"x": 1, "y": [1, 2]}))])).unwrap();
		assert_eq!(once, twice, "second apply is a no-op");
		// the inserted nested value round-trips through the JSONC parser
		assert_eq!(parse(&once).unwrap()["obj"], json!({"x": 1, "y": [1, 2]}));
	}

	#[test]
	fn nested_object_value_indents_under_property() {
		// `{}` has no trailing newline, so the patched output keeps none either.
		let out = merge_settings("{}", &managed(&[("a", json!({"k": 1}))])).unwrap();
		assert_eq!(out, "{\n    \"a\": {\n        \"k\": 1\n    }\n}");
	}

	#[test]
	fn unmanaged_keys_and_dotted_keys_survive() {
		let text = "{\n  \"telemetry.enabled\": false,\n  \"editor.fontSize\": 12\n}\n";
		let out = merge_settings(text, &managed(&[("editor.fontSize", json!(13))])).unwrap();
		assert!(out.contains("\"telemetry.enabled\": false"));
		assert!(out.contains("\"editor.fontSize\": 13"));
	}

	#[test]
	fn trailing_comma_object_gets_new_key() {
		let text = "{\n    \"a\": 1,\n}\n";
		let out = merge_settings(text, &managed(&[("b", json!(2))])).unwrap();
		assert_eq!(parse(&out).unwrap(), json!({"a": 1, "b": 2}));
	}

	#[test]
	fn rejects_non_object_document() {
		assert!(merge_settings("[1, 2, 3]", &managed(&[("a", json!(1))])).is_err());
	}
}
