//! Merge same-named JetBrains color schemes (`.icls`) and code styles across
//! IDEs into one cross-IDE file.
//!
//! Different IDEs flesh out the *same* named scheme with language-specific
//! pieces: WebStorm's "ABC" carries `JS.*` attributes, RustRover's carries
//! `org.rust.*`. Taking the union of those pieces (keyed by name) yields a
//! single scheme that highlights every language. Conflicts resolve to the first
//! source (the caller orders sources by a chosen primary IDE).

use anyhow::{anyhow, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;
use std::collections::BTreeMap;

/// Merge color schemes. All sources must share the same scheme `name`.
pub fn merge_color_schemes(sources: &[&str]) -> Result<String> {
	let mut iter = sources.iter();
	let first = iter.next().ok_or_else(|| anyhow!("no color schemes to merge"))?;
	let mut base = ColorScheme::parse(first)?;
	for src in iter {
		base.merge(&ColorScheme::parse(src)?);
	}
	Ok(base.to_xml())
}

/// Merge code styles (flat top-level children of `<code_scheme>`).
pub fn merge_code_styles(sources: &[&str]) -> Result<String> {
	let mut iter = sources.iter();
	let first = iter.next().ok_or_else(|| anyhow!("no code styles to merge"))?;
	let mut base = FlatScheme::parse(first, "code_scheme")?;
	for src in iter {
		base.merge(&FlatScheme::parse(src, "code_scheme")?);
	}
	Ok(base.to_xml())
}

/// The scheme name from a `.icls` (`<scheme name=>`) or code style
/// (`<code_scheme name=>`).
/// The `name` of the first element named `root` (e.g. "scheme" or "code_scheme").
pub fn scheme_name_of(xml: &str, root: &str) -> Option<String> {
	let events = read_all(xml).ok()?;
	events.iter().find_map(|e| match e {
		Event::Start(b) | Event::Empty(b) if name(b) == root => attr(b, "name"),
		_ => None,
	})
}

/// True if a color scheme carries no `<colors>` / `<attributes>` overrides — an
/// empty `partialSave` artifact (e.g. `_@user_Dark`) with nothing to sync.
pub fn color_scheme_is_empty(xml: &str) -> bool {
	match ColorScheme::parse(xml) {
		Ok(s) => s.colors.is_empty() && s.attributes.is_empty(),
		Err(_) => false,
	}
}

// ---------------------------------------------------------------------------
// color scheme model
// ---------------------------------------------------------------------------

struct ColorScheme {
	root_tag: String,
	extra: Vec<String>, // non-colors/attributes sections, e.g. <metaInfo>, from primary
	colors: BTreeMap<String, String>,
	attributes: BTreeMap<String, String>,
}

impl ColorScheme {
	fn parse(xml: &str) -> Result<ColorScheme> {
		let events = read_all(xml)?;
		let (root_i, root_empty) =
			find_root(&events, "scheme").ok_or_else(|| anyhow!("not a color scheme (no <scheme> element)"))?;
		let root_tag = open_tag(start_bytes(&events[root_i]));

		let mut colors = BTreeMap::new();
		let mut attributes = BTreeMap::new();
		let mut extra = Vec::new();

		let (_, scheme_end) = element_range(&events, root_i);
		let mut i = if root_empty { scheme_end } else { root_i + 1 };
		while i < scheme_end {
			if let Event::Start(b) | Event::Empty(b) = &events[i] {
				let tag = name(b);
				let (s, e) = element_range(&events, i);
				match tag.as_str() {
					"colors" => collect_options(&events, s, e, &mut colors),
					"attributes" => collect_options(&events, s, e, &mut attributes),
					_ => extra.push(serialize(&events[s..=e])),
				}
				i = e + 1;
			} else {
				i += 1;
			}
		}
		Ok(ColorScheme {
			root_tag,
			extra,
			colors,
			attributes,
		})
	}

	fn merge(&mut self, other: &ColorScheme) {
		for (k, v) in &other.colors {
			self.colors.entry(k.clone()).or_insert_with(|| v.clone());
		}
		for (k, v) in &other.attributes {
			self.attributes.entry(k.clone()).or_insert_with(|| v.clone());
		}
	}

	fn to_xml(&self) -> String {
		let mut out = String::new();
		out.push_str(&self.root_tag);
		out.push('\n');
		for ex in &self.extra {
			out.push_str("  ");
			out.push_str(ex);
			out.push('\n');
		}
		if !self.colors.is_empty() {
			out.push_str("  <colors>\n");
			for block in self.colors.values() {
				out.push_str("    ");
				out.push_str(block);
				out.push('\n');
			}
			out.push_str("  </colors>\n");
		}
		if !self.attributes.is_empty() {
			out.push_str("  <attributes>\n");
			for block in self.attributes.values() {
				out.push_str("    ");
				out.push_str(block);
				out.push('\n');
			}
			out.push_str("  </attributes>\n");
		}
		out.push_str("</scheme>");
		out
	}
}

/// Capture each direct `<option name=..>` subtree within a section, keyed by name.
fn collect_options(events: &[Event<'static>], s: usize, e: usize, out: &mut BTreeMap<String, String>) {
	let mut k = s + 1;
	while k < e {
		if let Event::Start(b) | Event::Empty(b) = &events[k] {
			if name(b) == "option" {
				let key = attr(b, "name").unwrap_or_default();
				let (os, oe) = element_range(events, k);
				out.entry(key).or_insert_with(|| serialize(&events[os..=oe]));
				k = oe + 1;
				continue;
			}
		}
		k += 1;
	}
}

// ---------------------------------------------------------------------------
// flat scheme model (code styles)
// ---------------------------------------------------------------------------

struct FlatScheme {
	root_tag: String,
	root_name: &'static str,
	children: BTreeMap<String, String>,
}

impl FlatScheme {
	fn parse(xml: &str, root: &'static str) -> Result<FlatScheme> {
		let events = read_all(xml)?;
		let (root_i, root_empty) =
			find_root(&events, root).ok_or_else(|| anyhow!("not a {root} (no <{root}> element)"))?;
		let root_tag = open_tag(start_bytes(&events[root_i]));

		let mut children = BTreeMap::new();
		let (_, end) = element_range(&events, root_i);
		let mut i = if root_empty { end } else { root_i + 1 };
		while i < end {
			if let Event::Start(b) | Event::Empty(b) = &events[i] {
				let key = flat_key(b);
				let (s, e) = element_range(&events, i);
				children.entry(key).or_insert_with(|| serialize(&events[s..=e]));
				i = e + 1;
			} else {
				i += 1;
			}
		}
		Ok(FlatScheme {
			root_tag,
			root_name: root,
			children,
		})
	}

	fn merge(&mut self, other: &FlatScheme) {
		for (k, v) in &other.children {
			self.children.entry(k.clone()).or_insert_with(|| v.clone());
		}
	}

	fn to_xml(&self) -> String {
		let mut out = String::new();
		out.push_str(&self.root_tag);
		out.push('\n');
		for block in self.children.values() {
			out.push_str("  ");
			out.push_str(block);
			out.push('\n');
		}
		out.push_str(&format!("</{}>", self.root_name));
		out
	}
}

/// Key a code-style child by tag plus its distinguishing attribute, so
/// `<codeStyleSettings language="JavaScript">` and `language="kotlin"` are
/// distinct but `<JavaCodeStyleSettings>` is unique by tag.
fn flat_key(b: &BytesStart) -> String {
	let tag = name(b);
	if let Some(n) = attr(b, "name") {
		format!("{tag}\u{1}name={n}")
	} else if let Some(l) = attr(b, "language") {
		format!("{tag}\u{1}language={l}")
	} else {
		tag
	}
}

// ---------------------------------------------------------------------------
// shared low-level XML helpers
// ---------------------------------------------------------------------------

fn read_all(xml: &str) -> Result<Vec<Event<'static>>> {
	let mut reader = Reader::from_str(xml);
	let mut events = Vec::new();
	loop {
		match reader.read_event() {
			Ok(Event::Eof) => break,
			Ok(ev) => events.push(ev.into_owned()),
			Err(e) => return Err(anyhow!("XML parse error: {e}")),
		}
	}
	Ok(events)
}

/// Serialize a slice of events, normalising self-closing tags to JetBrains'
/// `<.. />` style (space before `/>`).
fn serialize(events: &[Event<'static>]) -> String {
	let mut writer = Writer::new(Vec::new());
	for ev in events {
		let _ = writer.write_event(ev.clone());
	}
	let raw = String::from_utf8(writer.into_inner()).unwrap_or_default();
	raw.replace("\"/>", "\" />")
}

/// Find the root element by name, as `(index, is_empty)`. Handles both
/// `<root>...</root>` (Start) and an empty `<root .../>` (Empty).
fn find_root(events: &[Event<'static>], root: &str) -> Option<(usize, bool)> {
	events.iter().enumerate().find_map(|(i, e)| match e {
		Event::Start(b) if name(b) == root => Some((i, false)),
		Event::Empty(b) if name(b) == root => Some((i, true)),
		_ => None,
	})
}

fn start_bytes<'a>(e: &'a Event<'static>) -> &'a BytesStart<'static> {
	match e {
		Event::Start(b) | Event::Empty(b) => b,
		_ => unreachable!("start_bytes called on non-element event"),
	}
}

/// Render an opening tag `<name a="v" ...>` (never self-closing), so an empty
/// scheme can still serve as a container for merged children.
fn open_tag(b: &BytesStart) -> String {
	let mut s = format!("<{}", name(b));
	for a in b.attributes().with_checks(false).flatten() {
		let k = String::from_utf8_lossy(a.key.as_ref());
		let v = a.unescape_value().unwrap_or_default();
		s.push_str(&format!(" {k}=\"{}\"", escape_attr(&v)));
	}
	s.push('>');
	s
}

fn escape_attr(s: &str) -> String {
	s.replace('&', "&amp;")
		.replace('<', "&lt;")
		.replace('>', "&gt;")
		.replace('"', "&quot;")
}

fn name(b: &BytesStart) -> String {
	String::from_utf8_lossy(b.name().as_ref()).into_owned()
}

fn attr(b: &BytesStart, key: &str) -> Option<String> {
	for a in b.attributes().with_checks(false) {
		let a = a.ok()?;
		if a.key.as_ref() == key.as_bytes() {
			return Some(a.unescape_value().ok()?.into_owned());
		}
	}
	None
}

/// The event index range `[start, end]` of the element starting at `i`.
fn element_range(events: &[Event<'static>], i: usize) -> (usize, usize) {
	match &events[i] {
		Event::Start(b) => {
			let tag = name(b);
			let mut depth = 1usize;
			let mut j = i + 1;
			while j < events.len() {
				match &events[j] {
					Event::Start(x) if name(x) == tag => depth += 1,
					Event::End(x) if String::from_utf8_lossy(x.name().as_ref()) == tag => {
						depth -= 1;
						if depth == 0 {
							return (i, j);
						}
					}
					_ => {}
				}
				j += 1;
			}
			(i, events.len() - 1)
		}
		_ => (i, i),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const WEBSTORM: &str = r#"<scheme name="ABC" version="142" parent_scheme="Default">
  <metaInfo>
    <property name="ide">WebStorm</property>
  </metaInfo>
  <colors>
    <option name="CARET_COLOR" value="ffcc00" />
  </colors>
  <attributes>
    <option name="TEXT">
      <value>
        <option name="FOREGROUND" value="c8d3f5" />
      </value>
    </option>
    <option name="JS.LOCAL_VARIABLE">
      <value>
        <option name="FOREGROUND" value="aabbcc" />
      </value>
    </option>
  </attributes>
</scheme>"#;

	const RUSTROVER: &str = r#"<scheme name="ABC" version="142" parent_scheme="Default">
  <metaInfo>
    <property name="ide">RustRover</property>
  </metaInfo>
  <colors>
    <option name="CARET_COLOR" value="000000" />
    <option name="GUTTER_BACKGROUND" value="191a1c" />
  </colors>
  <attributes>
    <option name="TEXT">
      <value>
        <option name="FOREGROUND" value="ffffff" />
      </value>
    </option>
    <option name="org.rust.CRATE">
      <value>
        <option name="FOREGROUND" value="ddeeff" />
      </value>
    </option>
  </attributes>
</scheme>"#;

	#[test]
	fn merges_language_attributes_from_both_ides() {
		let merged = merge_color_schemes(&[WEBSTORM, RUSTROVER]).unwrap();
		// language-specific attrs from BOTH IDEs are present
		assert!(merged.contains(r#"<option name="JS.LOCAL_VARIABLE">"#));
		assert!(merged.contains(r#"<option name="org.rust.CRATE">"#));
		// shared TEXT present once
		assert_eq!(merged.matches(r#"<option name="TEXT">"#).count(), 1);
		// colors union
		assert!(merged.contains(r#"name="CARET_COLOR""#));
		assert!(merged.contains(r#"name="GUTTER_BACKGROUND""#));
		// metaInfo carried from the primary (WebStorm)
		assert!(merged.contains("WebStorm"));
		assert!(!merged.contains("RustRover"));
		// still a valid, re-parseable scheme
		assert_eq!(scheme_name_of(&merged, "scheme").as_deref(), Some("ABC"));
	}

	#[test]
	fn first_source_wins_on_conflict() {
		let merged = merge_color_schemes(&[WEBSTORM, RUSTROVER]).unwrap();
		// CARET_COLOR differs; WebStorm (primary) wins
		assert!(merged.contains(r#"name="CARET_COLOR" value="ffcc00""#));
		assert!(!merged.contains("value=\"000000\""));
		// TEXT foreground from WebStorm
		assert!(merged.contains(r#"value="c8d3f5""#));
		assert!(!merged.contains(r#"value="ffffff""#));
	}

	#[test]
	fn merges_code_style_language_sections() {
		let intellij = r#"<code_scheme name="Shura" version="173">
  <option name="RIGHT_MARGIN" value="120" />
  <JavaCodeStyleSettings>
    <option name="CLASS_COUNT_TO_USE_IMPORT_ON_DEMAND" value="999" />
  </JavaCodeStyleSettings>
  <codeStyleSettings language="JAVA">
    <option name="INDENT_SIZE" value="4" />
  </codeStyleSettings>
</code_scheme>"#;
		let webstorm = r#"<code_scheme name="Shura" version="173">
  <option name="RIGHT_MARGIN" value="100" />
  <TypeScriptCodeStyleSettings>
    <option name="USE_SEMICOLON_AFTER_STATEMENT" value="false" />
  </TypeScriptCodeStyleSettings>
  <codeStyleSettings language="TypeScript">
    <option name="INDENT_SIZE" value="2" />
  </codeStyleSettings>
</code_scheme>"#;
		let merged = merge_code_styles(&[intellij, webstorm]).unwrap();
		assert!(merged.contains("<JavaCodeStyleSettings>"));
		assert!(merged.contains("<TypeScriptCodeStyleSettings>"));
		assert!(merged.contains(r#"language="JAVA""#));
		assert!(merged.contains(r#"language="TypeScript""#));
		// RIGHT_MARGIN conflict: IntelliJ (primary) wins
		assert!(merged.contains(r#"name="RIGHT_MARGIN" value="120""#));
		assert!(!merged.contains(r#"value="100""#));
	}

	#[test]
	fn detects_empty_override_schemes() {
		// a `_@user_*` partialSave artifact with no colors/attributes
		let empty = r#"<scheme name="_@user_Dark" version="142" parent_scheme="Darcula">
  <metaInfo>
    <property name="originalScheme">Dark</property>
    <property name="partialSave">true</property>
  </metaInfo>
</scheme>"#;
		assert!(color_scheme_is_empty(empty));
		// a real override (some colors) is not empty
		let real = r#"<scheme name="_@user_Darcula" parent_scheme="Darcula">
  <colors>
    <option name="FILESTATUS_ADDED" value="80cbc4" />
  </colors>
</scheme>"#;
		assert!(!color_scheme_is_empty(real));
	}

	#[test]
	fn handles_self_closing_empty_root() {
		// A `<code_scheme .../>` (empty "Default" style) must merge, not error.
		let empty = r#"<code_scheme name="Default" version="173" />"#;
		let full = r#"<code_scheme name="Default" version="173">
  <JavaCodeStyleSettings>
    <option name="X" value="1" />
  </JavaCodeStyleSettings>
</code_scheme>"#;
		let merged = merge_code_styles(&[empty, full]).unwrap();
		assert!(merged.starts_with(r#"<code_scheme name="Default" version="173">"#));
		assert!(merged.contains("<JavaCodeStyleSettings>"));
		assert!(merged.trim_end().ends_with("</code_scheme>"));
	}
}
