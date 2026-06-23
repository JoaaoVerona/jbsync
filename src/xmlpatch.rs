//! Surgical patching of JetBrains `options/*.xml` files.
//!
//! These files all share the shape:
//! ```xml
//! <application>
//!   <component name="X">
//!     <option name="Y" value="Z" />
//!   </component>
//! </application>
//! ```
//!
//! We parse to an event stream and only rewrite the bytes we actually touch —
//! every untouched element (including comments, CDATA and exact whitespace) is
//! passed through verbatim. That keeps diffs minimal and deterministic and
//! preserves any setting we don't model.

use anyhow::{anyhow, Result};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;

const SCAFFOLD: &str = "<application>\n</application>";

/// Ensure `<component name=component>` contains an element `elem` matching
/// `match_attr` (or the first such element if `None`) with `set_attrs` set.
/// Creates the element — and the component — if missing.
pub fn ensure(
	xml: &str,
	component: &str,
	elem: &str,
	match_attr: Option<(&str, &str)>,
	set_attrs: &[(&str, &str)],
) -> Result<String> {
	let base = if xml.trim().is_empty() { SCAFFOLD } else { xml };
	let mut events = read_owned(base)?;

	match find_component(&events, component) {
		None => insert_component(&mut events, component, elem, match_attr, set_attrs)?,
		Some(range) if range.empty => {
			expand_empty_component(&mut events, range.start, component, elem, match_attr, set_attrs)
		}
		Some(range) => {
			let found = (range.start + 1..range.end).find(|&k| {
				matches!(&events[k], Event::Start(bs) | Event::Empty(bs)
                    if elem_name(bs) == elem && match_ok(bs, match_attr))
			});
			match found {
				Some(k) => {
					let mut all: Vec<(&str, &str)> = Vec::new();
					if let Some(m) = match_attr {
						all.push(m);
					}
					all.extend_from_slice(set_attrs);
					events[k] = match &events[k] {
						Event::Start(bs) => {
							let (n, a) = (elem_name(bs), merged_attrs(bs, &all));
							Event::Start(start_tag(&n, &a))
						}
						Event::Empty(bs) => {
							let (n, a) = (elem_name(bs), merged_attrs(bs, &all));
							Event::Empty(empty_tag(&n, &a))
						}
						_ => unreachable!(),
					};
				}
				None => insert_child(&mut events, range.end, elem, match_attr, set_attrs),
			}
		}
	}
	serialize(&events)
}

/// Convenience: ensure `<option name=.. value=.. />` inside a component.
pub fn ensure_option(xml: &str, component: &str, name: &str, value: &str) -> Result<String> {
	ensure(xml, component, "option", Some(("name", name)), &[("value", value)])
}

/// Read the `value` of `<option name=name>` in a component, if present.
pub fn get_option(xml: &str, component: &str, name: &str) -> Option<String> {
	get_attr(xml, component, "option", Some(("name", name)), "value")
}

/// Read `attr_name` of the first element `elem` (matching `match_attr`) in a component.
pub fn get_attr(
	xml: &str,
	component: &str,
	elem: &str,
	match_attr: Option<(&str, &str)>,
	attr_name: &str,
) -> Option<String> {
	let events = read_owned(xml).ok()?;
	let range = find_component(&events, component)?;
	if range.empty {
		return None;
	}
	for ev in &events[range.start + 1..range.end] {
		if let Event::Start(bs) | Event::Empty(bs) = ev {
			if elem_name(bs) == elem && match_ok(bs, match_attr) {
				return attr(bs, attr_name);
			}
		}
	}
	None
}

// ---------------------------------------------------------------------------
// internals
// ---------------------------------------------------------------------------

struct CompRange {
	start: usize,
	end: usize,
	empty: bool,
}

fn read_owned(xml: &str) -> Result<Vec<Event<'static>>> {
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

fn serialize(events: &[Event<'static>]) -> Result<String> {
	let mut writer = Writer::new(Vec::new());
	for ev in events {
		writer
			.write_event(ev.clone())
			.map_err(|e| anyhow!("XML write error: {e}"))?;
	}
	Ok(String::from_utf8(writer.into_inner())?)
}

fn elem_name(bs: &BytesStart) -> String {
	String::from_utf8_lossy(bs.name().as_ref()).into_owned()
}

fn end_name(e: &BytesEnd) -> String {
	String::from_utf8_lossy(e.name().as_ref()).into_owned()
}

fn attr(bs: &BytesStart, key: &str) -> Option<String> {
	for a in bs.attributes().with_checks(false) {
		let a = a.ok()?;
		if a.key.as_ref() == key.as_bytes() {
			return Some(a.unescape_value().ok()?.into_owned());
		}
	}
	None
}

fn match_ok(bs: &BytesStart, match_attr: Option<(&str, &str)>) -> bool {
	match match_attr {
		None => true,
		Some((k, v)) => attr(bs, k).as_deref() == Some(v),
	}
}

/// Merge an existing element's attributes with overrides. Existing order is
/// preserved; new attributes are appended.
fn merged_attrs(bs: &BytesStart, set: &[(&str, &str)]) -> Vec<(String, String)> {
	let mut out: Vec<(String, String)> = Vec::new();
	let mut seen: Vec<String> = Vec::new();
	for a in bs.attributes().with_checks(false) {
		let a = match a {
			Ok(a) => a,
			Err(_) => continue,
		};
		let key = String::from_utf8_lossy(a.key.as_ref()).into_owned();
		let val = a.unescape_value().map(|c| c.into_owned()).unwrap_or_default();
		let v = set
			.iter()
			.find(|(k, _)| *k == key)
			.map(|(_, v)| v.to_string())
			.unwrap_or(val);
		out.push((key.clone(), v));
		seen.push(key);
	}
	for (k, v) in set {
		if !seen.iter().any(|s| s == k) {
			out.push((k.to_string(), v.to_string()));
		}
	}
	out
}

fn collect_new(match_attr: Option<(&str, &str)>, set_attrs: &[(&str, &str)]) -> Vec<(String, String)> {
	let mut out = Vec::new();
	if let Some((k, v)) = match_attr {
		out.push((k.to_string(), v.to_string()));
	}
	for (k, v) in set_attrs {
		out.push((k.to_string(), v.to_string()));
	}
	out
}

/// A self-closing element with a trailing space, matching JetBrains' style:
/// `<option name="x" value="y" />`. quick-xml writes `Event::Empty` content
/// verbatim, so we escape values ourselves here.
fn empty_tag(name: &str, attrs: &[(String, String)]) -> BytesStart<'static> {
	let mut content = String::from(name);
	for (k, v) in attrs {
		content.push(' ');
		content.push_str(k);
		content.push_str("=\"");
		content.push_str(&attr_escape(v));
		content.push('"');
	}
	content.push(' ');
	let name_len = name.len();
	BytesStart::from_content(content, name_len)
}

/// An opening tag (no trailing space): `<component name="X">`.
fn start_tag(name: &str, attrs: &[(String, String)]) -> BytesStart<'static> {
	let mut bs = BytesStart::new(name.to_string());
	for (k, v) in attrs {
		bs.push_attribute((k.as_str(), v.as_str()));
	}
	bs
}

fn attr_escape(s: &str) -> String {
	s.replace('&', "&amp;")
		.replace('<', "&lt;")
		.replace('>', "&gt;")
		.replace('"', "&quot;")
}

fn text(s: &str) -> Event<'static> {
	Event::Text(BytesText::from_escaped(s.to_string()))
}

fn find_component(events: &[Event<'static>], component: &str) -> Option<CompRange> {
	let mut i = 0;
	while i < events.len() {
		let is_comp = matches!(&events[i], Event::Start(bs) | Event::Empty(bs)
            if elem_name(bs) == "component" && attr(bs, "name").as_deref() == Some(component));
		if is_comp {
			if matches!(&events[i], Event::Empty(_)) {
				return Some(CompRange {
					start: i,
					end: i,
					empty: true,
				});
			}
			let mut depth = 1usize;
			let mut j = i + 1;
			while j < events.len() {
				match &events[j] {
					Event::Start(b) if elem_name(b) == "component" => depth += 1,
					Event::End(e) if end_name(e) == "component" => {
						depth -= 1;
						if depth == 0 {
							return Some(CompRange {
								start: i,
								end: j,
								empty: false,
							});
						}
					}
					_ => {}
				}
				j += 1;
			}
			return Some(CompRange {
				start: i,
				end: events.len() - 1,
				empty: false,
			});
		}
		i += 1;
	}
	None
}

fn application_end(events: &[Event<'static>]) -> Option<usize> {
	(0..events.len())
		.rev()
		.find(|&i| matches!(&events[i], Event::End(e) if end_name(e) == "application"))
}

/// Whitespace text node immediately before index `before`, if it contains a newline.
fn ws_before(events: &[Event<'static>], before: usize) -> Option<String> {
	if before == 0 {
		return None;
	}
	if let Event::Text(t) = &events[before - 1] {
		let s = String::from_utf8_lossy(t.as_ref()).into_owned();
		if s.contains('\n') {
			return Some(s);
		}
	}
	None
}

fn insert_child(
	events: &mut Vec<Event<'static>>,
	comp_end: usize,
	elem: &str,
	match_attr: Option<(&str, &str)>,
	set_attrs: &[(&str, &str)],
) {
	let closing = ws_before(events, comp_end);
	let indent = closing
		.as_ref()
		.map(|w| format!("{w}  "))
		.unwrap_or_else(|| "\n    ".to_string());
	let at = if closing.is_some() { comp_end - 1 } else { comp_end };
	let newel = Event::Empty(empty_tag(elem, &collect_new(match_attr, set_attrs)));
	events.splice(at..at, [text(&indent), newel]);
}

fn expand_empty_component(
	events: &mut Vec<Event<'static>>,
	idx: usize,
	component: &str,
	elem: &str,
	match_attr: Option<(&str, &str)>,
	set_attrs: &[(&str, &str)],
) {
	let comp_indent = ws_before(events, idx).unwrap_or_else(|| "\n  ".to_string());
	let child_indent = format!("{comp_indent}  ");
	let mut comp = BytesStart::new("component");
	comp.push_attribute(("name", component));
	let replacement = vec![
		Event::Start(comp),
		text(&child_indent),
		Event::Empty(empty_tag(elem, &collect_new(match_attr, set_attrs))),
		text(&comp_indent),
		Event::End(BytesEnd::new("component")),
	];
	events.splice(idx..idx + 1, replacement);
}

fn insert_component(
	events: &mut Vec<Event<'static>>,
	component: &str,
	elem: &str,
	match_attr: Option<(&str, &str)>,
	set_attrs: &[(&str, &str)],
) -> Result<()> {
	let app_end = application_end(events).ok_or_else(|| anyhow!("missing <application> root"))?;
	let comp_indent = "\n  ";
	let child_indent = "\n    ";
	let mut comp = BytesStart::new("component");
	comp.push_attribute(("name", component));
	let block = vec![
		text(comp_indent),
		Event::Start(comp),
		text(child_indent),
		Event::Empty(empty_tag(elem, &collect_new(match_attr, set_attrs))),
		text(comp_indent),
		Event::End(BytesEnd::new("component")),
	];
	let closing = ws_before(events, app_end);
	let at = if closing.is_some() { app_end - 1 } else { app_end };
	events.splice(at..at, block);
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn roundtrip_preserves_whitespace_and_structure() {
		let xml = "<application>\n  <component name=\"A\">\n    <option name=\"X\" value=\"1\" />\n  </component>\n</application>";
		let events = read_owned(xml).unwrap();
		assert_eq!(serialize(&events).unwrap(), xml);
	}

	#[test]
	fn modifies_existing_option_only() {
		let xml = "<application>\n  <component name=\"DefaultFont\">\n    <option name=\"FONT_SIZE\" value=\"13\" />\n    <option name=\"LINE_SPACING\" value=\"1.0\" />\n  </component>\n</application>";
		let out = ensure_option(xml, "DefaultFont", "FONT_SIZE", "15").unwrap();
		assert!(out.contains("<option name=\"FONT_SIZE\" value=\"15\" />"));
		// sibling untouched
		assert!(out.contains("<option name=\"LINE_SPACING\" value=\"1.0\" />"));
	}

	#[test]
	fn inserts_missing_option_into_existing_component() {
		let xml = "<application>\n  <component name=\"DefaultFont\">\n    <option name=\"FONT_SIZE\" value=\"15\" />\n  </component>\n</application>";
		let out = ensure_option(xml, "DefaultFont", "USE_LIGATURES", "true").unwrap();
		assert!(out.contains("<option name=\"FONT_SIZE\" value=\"15\" />"));
		assert!(out.contains("<option name=\"USE_LIGATURES\" value=\"true\" />"));
	}

	#[test]
	fn inserts_missing_component() {
		let xml = "<application>\n  <component name=\"Other\">\n    <option name=\"A\" value=\"1\" />\n  </component>\n</application>";
		let out = ensure_option(xml, "EmmetOptions", "emmetEnabled", "false").unwrap();
		assert!(out.contains("<component name=\"Other\">"));
		assert!(out.contains("<component name=\"EmmetOptions\">"));
		assert!(out.contains("<option name=\"emmetEnabled\" value=\"false\" />"));
		// still valid: reparse round-trips
		read_owned(&out).unwrap();
	}

	#[test]
	fn creates_file_from_empty() {
		let out = ensure_option("", "PostfixTemplatesSettings", "postfixTemplatesEnabled", "false").unwrap();
		assert_eq!(
            out,
            "<application>\n  <component name=\"PostfixTemplatesSettings\">\n    <option name=\"postfixTemplatesEnabled\" value=\"false\" />\n  </component>\n</application>"
        );
	}

	#[test]
	fn idempotent() {
		let xml = "<application>\n</application>";
		let once = ensure_option(xml, "C", "K", "V").unwrap();
		let twice = ensure_option(&once, "C", "K", "V").unwrap();
		assert_eq!(once, twice);
	}

	#[test]
	fn non_option_element_by_name() {
		// colors.scheme.xml style: a single <global_color_scheme name=".."/>
		let xml = "<application>\n  <component name=\"EditorColorsManagerImpl\">\n    <global_color_scheme name=\"Old\" />\n  </component>\n</application>";
		let out = ensure(
			xml,
			"EditorColorsManagerImpl",
			"global_color_scheme",
			None,
			&[("name", "Verona Dark")],
		)
		.unwrap();
		assert!(out.contains("<global_color_scheme name=\"Verona Dark\" />"));
	}

	#[test]
	fn preserves_unrelated_components_and_comments() {
		let xml = "<application>\n  <!-- keep me -->\n  <component name=\"Keep\">\n    <option name=\"deep\">\n      <map>\n        <entry key=\"a\" value=\"b\" />\n      </map>\n    </option>\n  </component>\n</application>";
		let out = ensure_option(xml, "New", "k", "v").unwrap();
		assert!(out.contains("<!-- keep me -->"));
		assert!(out.contains("<entry key=\"a\" value=\"b\" />"));
		assert!(out.contains("<component name=\"New\">"));
	}
}
