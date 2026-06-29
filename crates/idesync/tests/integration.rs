//! End-to-end tests that drive the real compiled binary against seeded copies
//! of real JetBrains config files in a temp directory. Nothing here touches the
//! user's actual config — everything is redirected via IDESYNC_JB_CONFIG_HOME.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_idesync")
}

fn write(path: PathBuf, content: &str) {
	if let Some(p) = path.parent() {
		fs::create_dir_all(p).unwrap();
	}
	fs::write(path, content).unwrap();
}

/// Seed a fake "~/.config/JetBrains" with one IntelliJ install from fixtures.
fn seed_ide(base: &Path) -> PathBuf {
	let ide = base.join("IntelliJIdea2026.1");
	write(
		ide.join("options/editor-font.xml"),
		include_str!("fixtures/editor-font.xml"),
	);
	write(ide.join("options/ui.lnf.xml"), include_str!("fixtures/ui.lnf.xml"));
	write(
		ide.join("options/ide.general.xml"),
		include_str!("fixtures/ide.general.xml"),
	);
	write(
		ide.join("options/colors.scheme.xml"),
		include_str!("fixtures/colors.scheme.xml"),
	);
	write(
		ide.join("disabled_plugins.txt"),
		include_str!("fixtures/disabled_plugins.txt"),
	);
	write(ide.join("idea64.vmoptions"), include_str!("fixtures/idea64.vmoptions"));
	ide
}

const CONFIG: &str = r#"{
  "editor": { "font": { "family": "JetBrains Mono", "size": 15, "lineSpacing": 1.25, "ligatures": true, "regularWeight": "Medium", "boldWeight": "ExtraBold" } },
  "ui": { "compactTreeIndents": true, "mergeMainMenuIntoToolbar": true, "contrastScrollbars": true, "experimentalUi": true },
  "editorBehavior": { "softWrap": true, "showBreadcrumbs": false },
  "colorScheme": { "name": "Verona Dark", "file": "Verona Dark.icls" },
  "plugins": { "disabled": ["com.intellij.spring", "com.intellij.javaee"] },
  "vmOptions": { "heapSizeMb": 3072 },
  "keymap": { "name": "Verona", "bindings": { "ReformatCode": "mod+1", "ActivateTerminalToolWindow": "ctrl+b", "$Copy": "mod+c", "CopyElement": [] } }
}"#;

/// Write the config + referenced scheme file into a temp "source" dir.
fn write_config(dir: &Path) -> PathBuf {
	let cfg = dir.join("idesync.json");
	write(cfg.clone(), CONFIG);
	write(dir.join("Verona Dark.icls"), include_str!("fixtures/Verona Dark.icls"));
	cfg
}

fn run(base: &Path, args: &[&str]) -> Output {
	Command::new(bin())
		.args(args)
		.env("IDESYNC_JB_CONFIG_HOME", base)
		.output()
		.expect("failed to run idesync")
}

fn run_env(base: &Path, extra: &[(&str, &str)], args: &[&str]) -> Output {
	let mut c = Command::new(bin());
	c.args(args).env("IDESYNC_JB_CONFIG_HOME", base);
	for (k, v) in extra {
		c.env(k, v);
	}
	c.output().expect("failed to run idesync")
}

fn read(p: PathBuf) -> String {
	fs::read_to_string(p).unwrap()
}

fn setup() -> (TempDir, PathBuf, PathBuf) {
	let tmp = tempfile::tempdir().unwrap();
	let base = tmp.path().join("JetBrains");
	let ide = seed_ide(&base);
	let cfg = write_config(&tmp.path().join("dotfiles"));
	(tmp, base, {
		let _ = ide;
		cfg
	})
}

fn apply_linux(base: &Path, cfg: &Path) -> Output {
	run(
		base,
		&[
			"jb",
			"apply",
			cfg.to_str().unwrap(),
			"--product",
			"IntelliJIdea",
			"--version",
			"2026.1",
			"--os",
			"linux",
		],
	)
}

#[test]
fn apply_patches_options_surgically() {
	let (_tmp, base, cfg) = setup();
	let out = apply_linux(&base, &cfg);
	assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

	let ide = base.join("IntelliJIdea2026.1");

	let font = read(ide.join("options/editor-font.xml"));
	assert!(font.contains(r#"<option name="FONT_SIZE" value="15" />"#));
	assert!(font.contains(r#"<option name="FONT_SIZE_2D" value="15.0" />"#));
	assert!(font.contains(r#"<option name="FONT_FAMILY" value="JetBrains Mono" />"#));
	assert!(font.contains(r#"<option name="LINE_SPACING" value="1.25" />"#));
	assert!(font.contains(r#"<option name="USE_LIGATURES" value="true" />"#));
	// existing weight overwritten Regular -> Medium; VERSION preserved
	assert!(font.contains(r#"<option name="FONT_REGULAR_SUB_FAMILY" value="Medium" />"#));
	assert!(font.contains(r#"<option name="FONT_BOLD_SUB_FAMILY" value="ExtraBold" />"#));
	assert!(font.contains(r#"<option name="VERSION" value="1" />"#));
	assert!(!font.contains("Regular"));

	let ui = read(ide.join("options/ui.lnf.xml"));
	assert!(ui.contains(r#"<option name="compactTreeIndents" value="true" />"#));
	assert!(ui.contains(r#"<option name="DND_WITH_PRESSED_ALT_ONLY" value="true" />"#)); // preserved
	assert!(ui.contains(r#"<option name="CONTRAST_SCROLLBARS" value="true" />"#));
	assert!(ui.contains(r#"<option name="SHOW_MAIN_MENU_MODE" value="MERGED_WITH_MAIN_TOOLBAR" />"#));

	let general = read(ide.join("options/ide.general.xml"));
	assert!(general.contains(r#"<entry key="ide.experimental.ui" value="true" />"#));
	assert!(general.contains(r#"key="vcs.log.index.enable""#)); // preserved

	let editor = read(ide.join("options/editor.xml"));
	assert!(editor.contains(r#"<option name="USE_SOFT_WRAPS" value="MAIN_EDITOR" />"#));
	assert!(editor.contains(r#"<option name="SOFT_WRAP_FILE_MASKS" value="*" />"#));
	assert!(editor.contains(r#"<option name="SHOW_BREADCRUMBS" value="false" />"#));
}

#[test]
fn apply_installs_and_activates_color_scheme() {
	let (_tmp, base, cfg) = setup();
	assert!(apply_linux(&base, &cfg).status.success());
	let ide = base.join("IntelliJIdea2026.1");

	let installed = read(ide.join("colors/Verona Dark.icls"));
	assert!(installed.contains(r#"<scheme name="Verona Dark""#));

	let selector = read(ide.join("options/colors.scheme.xml"));
	assert!(selector.contains(r#"<global_color_scheme name="Verona Dark" />"#));
	assert!(!selector.contains("Darcula"));
}

#[test]
fn apply_merges_disabled_plugins_and_patches_vmoptions() {
	let (_tmp, base, cfg) = setup();
	assert!(apply_linux(&base, &cfg).status.success());
	let ide = base.join("IntelliJIdea2026.1");

	let disabled = read(ide.join("disabled_plugins.txt"));
	let lines: Vec<&str> = disabled.lines().collect();
	// union of seeded {copyright, training} and config {spring, javaee}, sorted
	assert_eq!(
		lines,
		vec![
			"com.intellij.copyright",
			"com.intellij.javaee",
			"com.intellij.spring",
			"training",
		]
	);

	let vm = read(ide.join("idea64.vmoptions"));
	assert!(vm.contains("-Xmx3072m"));
	assert!(!vm.contains("-Xmx2048m"));
	assert!(vm.contains("toolbox.notification.token=PLACEHOLDER-TOKEN")); // preserved
	assert!(vm.contains("ide.managed.by.toolbox")); // preserved
}

#[test]
fn exclude_skips_named_sections_in_apply_and_check() {
	let (_tmp, base, cfg) = setup();
	let ide = base.join("IntelliJIdea2026.1");
	let before_disabled = read(ide.join("disabled_plugins.txt"));

	// Exclude two sections at once; `keymaps` uses the plural alias.
	let out = run(
		&base,
		&[
			"jb",
			"apply",
			cfg.to_str().unwrap(),
			"--product",
			"IntelliJIdea",
			"--version",
			"2026.1",
			"--os",
			"linux",
			"--exclude",
			"plugins",
			"--exclude",
			"keymaps",
		],
	);
	assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

	// Excluded sections are untouched: disabled_plugins.txt unchanged, no keymap written...
	assert_eq!(read(ide.join("disabled_plugins.txt")), before_disabled);
	assert!(!ide.join("keymaps/Verona _Linux_.xml").exists());
	// ...while non-excluded settings still apply.
	let font = read(ide.join("options/editor-font.xml"));
	assert!(font.contains(r#"<option name="FONT_FAMILY" value="JetBrains Mono" />"#));

	// `check` with the same exclusions reports the fresh apply as in sync.
	let chk = run(
		&base,
		&[
			"jb",
			"check",
			cfg.to_str().unwrap(),
			"--product",
			"IntelliJIdea",
			"--version",
			"2026.1",
			"--os",
			"linux",
			"--exclude",
			"plugins",
			"--exclude",
			"keymap",
		],
	);
	assert_eq!(chk.status.code(), Some(0), "in sync once sections are excluded");
	assert!(String::from_utf8_lossy(&chk.stdout).contains("in sync"));

	// An unknown section is rejected by clap (exit 2).
	let bad = run(
		&base,
		&[
			"jb",
			"check",
			cfg.to_str().unwrap(),
			"--product",
			"IntelliJIdea",
			"--os",
			"linux",
			"--exclude",
			"nonsense",
		],
	);
	assert_eq!(bad.status.code(), Some(2), "unknown section should be a usage error");
}

#[test]
fn apply_generates_linux_keymap_with_mod_as_ctrl() {
	let (_tmp, base, cfg) = setup();
	assert!(apply_linux(&base, &cfg).status.success());
	let ide = base.join("IntelliJIdea2026.1");

	let km = read(ide.join("keymaps/Verona _Linux_.xml"));
	assert!(km.contains(r#"name="Verona (Linux)""#));
	assert!(km.contains(r#"<action id="ReformatCode">"#));
	assert!(km.contains(r#"first-keystroke="ctrl 1""#)); // mod -> ctrl
	assert!(km.contains(r#"first-keystroke="ctrl b""#)); // literal ctrl
	assert!(km.contains(r#"first-keystroke="ctrl c""#)); // $Copy mod -> ctrl on linux
	assert!(km.contains(r#"<action id="CopyElement" />"#)); // empty binding removes shortcut

	// active-keymap pointer lives in the per-OS settings subdir
	let active = read(ide.join("options/linux/keymap.xml"));
	assert!(active.contains(r#"<active_keymap name="Verona (Linux)" />"#));
}

#[test]
fn mac_keymap_swaps_mod_to_cmd_but_keeps_literal_ctrl() {
	let (_tmp, base, cfg) = setup();
	let out = run(
		&base,
		&[
			"jb",
			"apply",
			cfg.to_str().unwrap(),
			"--product",
			"IntelliJIdea",
			"--version",
			"2026.1",
			"--os",
			"macos",
		],
	);
	assert!(out.status.success());
	let ide = base.join("IntelliJIdea2026.1");

	let km = read(ide.join("keymaps/Verona _macOS_.xml"));
	assert!(km.contains(r#"name="Verona (macOS)""#));
	assert!(km.contains(r#"first-keystroke="meta 1""#)); // mod -> Cmd
	assert!(km.contains(r#"first-keystroke="meta c""#)); // $Copy -> Cmd
	assert!(km.contains(r#"first-keystroke="ctrl b""#)); // literal ctrl preserved, NOT swapped
}

#[test]
fn check_reports_drift_then_in_sync_after_apply() {
	let (_tmp, base, cfg) = setup();

	let pre = run(
		&base,
		&[
			"jb",
			"check",
			cfg.to_str().unwrap(),
			"--product",
			"IntelliJIdea",
			"--version",
			"2026.1",
			"--os",
			"linux",
		],
	);
	assert_eq!(pre.status.code(), Some(1), "fresh config should report drift");
	assert!(String::from_utf8_lossy(&pre.stdout).contains("would change"));

	assert!(apply_linux(&base, &cfg).status.success());

	let post = run(
		&base,
		&[
			"jb",
			"check",
			cfg.to_str().unwrap(),
			"--product",
			"IntelliJIdea",
			"--version",
			"2026.1",
			"--os",
			"linux",
		],
	);
	assert_eq!(
		post.status.code(),
		Some(0),
		"after apply it should be in sync (idempotent)"
	);
	assert!(String::from_utf8_lossy(&post.stdout).contains("in sync"));
}

#[test]
fn dry_run_writes_nothing() {
	let (_tmp, base, cfg) = setup();
	let ide = base.join("IntelliJIdea2026.1");
	let before = read(ide.join("options/editor-font.xml"));

	let out = run(
		&base,
		&[
			"jb",
			"apply",
			cfg.to_str().unwrap(),
			"--product",
			"IntelliJIdea",
			"--version",
			"2026.1",
			"--os",
			"linux",
			"--dry-run",
		],
	);
	assert!(out.status.success());
	assert!(String::from_utf8_lossy(&out.stdout).contains("would change"));

	let after = read(ide.join("options/editor-font.xml"));
	assert_eq!(before, after, "dry-run must not modify files");
	assert!(
		!ide.join("options/editor.xml").exists(),
		"dry-run must not create files"
	);
}

#[test]
fn vmoptions_with_correct_heap_and_no_trailing_newline_is_idempotent() {
	// Regression: a file with no final newline whose heap already matches must
	// not be reported as drift (no spurious trailing-newline change).
	let tmp = tempfile::tempdir().unwrap();
	let base = tmp.path().join("JetBrains");
	let ide = base.join("IntelliJIdea2026.1");
	write(ide.join("idea64.vmoptions"), "-Xmx2048m\n-Dfoo=bar"); // no trailing newline
	let cfg = tmp.path().join("c.json");
	write(
		cfg.clone(),
		r#"{ "targets": [{"product":"IntelliJIdea","version":"2026.1"}], "vmOptions": { "heapSizeMb": 2048 } }"#,
	);
	let out = run(&base, &["jb", "check", cfg.to_str().unwrap(), "--os", "linux"]);
	assert_eq!(
		out.status.code(),
		Some(0),
		"should be in sync: {}",
		String::from_utf8_lossy(&out.stdout)
	);
}

#[test]
fn per_target_plugins_merge_with_global_at_apply() {
	// Global disables one plugin; the IntelliJ target adds another. Applying to
	// IntelliJ must disable BOTH; a different target would get only the global.
	let tmp = tempfile::tempdir().unwrap();
	let base = tmp.path().join("JetBrains");
	write(base.join("IntelliJIdea2026.1/disabled_plugins.txt"), "preexisting\n");
	write(base.join("WebStorm2026.1/disabled_plugins.txt"), "preexisting\n");

	let cfg = tmp.path().join("c.json");
	write(
		cfg.clone(),
		r#"{
          "plugins": { "disabled": ["common.everywhere"] },
          "targets": [
            { "product": "IntelliJIdea", "version": "2026.1",
              "plugins": { "disabled": ["only.intellij"] } },
            { "product": "WebStorm", "version": "2026.1" }
          ]
        }"#,
	);

	assert!(run(&base, &["jb", "apply", cfg.to_str().unwrap(), "--os", "linux"])
		.status
		.success());

	let idea = read(base.join("IntelliJIdea2026.1/disabled_plugins.txt"));
	let idea_lines: Vec<&str> = idea.lines().collect();
	assert_eq!(idea_lines, vec!["common.everywhere", "only.intellij", "preexisting"]);

	// WebStorm gets the global only — NOT the IntelliJ-specific one.
	let ws = read(base.join("WebStorm2026.1/disabled_plugins.txt"));
	assert!(ws.contains("common.everywhere"));
	assert!(!ws.contains("only.intellij"));
}

#[cfg(unix)]
#[test]
fn plugin_install_flow_skips_present_and_is_idempotent() {
	use std::os::unix::fs::PermissionsExt;

	let tmp = tempfile::tempdir().unwrap();
	let base = tmp.path().join("JetBrains");
	write(base.join("IntelliJIdea2026.1/options/.keep"), ""); // config dir exists
	let data = tmp.path().join("data");
	fs::create_dir_all(&data).unwrap();

	// A fake launcher that mimics `installPlugins` by writing each plugin's
	// descriptor into the data dir (no network). It reads IDESYNC_JB_DATA_HOME,
	// which idesync inherits and passes through to this child process.
	let launcher = tmp.path().join("idea-fake.sh");
	write(
		launcher.clone(),
		"#!/usr/bin/env bash\nset -e\n[ \"$1\" = installPlugins ] || exit 2\nshift\n\
         dir=\"$IDESYNC_JB_DATA_HOME/IntelliJIdea2026.1\"\n\
         for id in \"$@\"; do\n  case \"$id\" in http*) continue;; esac\n  \
         mkdir -p \"$dir/$id/META-INF\"\n  \
         printf '<idea-plugin><id>%s</id></idea-plugin>' \"$id\" > \"$dir/$id/META-INF/plugin.xml\"\n\
         done\n",
	);
	let mut perms = fs::metadata(&launcher).unwrap().permissions();
	perms.set_mode(0o755);
	fs::set_permissions(&launcher, perms).unwrap();

	// Pre-install one plugin to prove it is detected and skipped.
	write(
		data.join("IntelliJIdea2026.1/already/META-INF/plugin.xml"),
		"<idea-plugin><id>com.already.here</id></idea-plugin>",
	);

	let cfg = tmp.path().join("c.json");
	write(
		cfg.clone(),
		r#"{ "targets":[{"product":"IntelliJIdea","version":"2026.1"}],
            "plugins": { "install": ["com.foo.bar", "com.already.here"] } }"#,
	);

	let extra = [
		("IDESYNC_JB_DATA_HOME", data.to_str().unwrap()),
		("IDESYNC_JB_LAUNCHER", launcher.to_str().unwrap()),
	];

	// before: only com.foo.bar is missing; the already-present one is skipped
	let pre = run_env(&base, &extra, &["jb", "check", cfg.to_str().unwrap(), "--os", "linux"]);
	assert_eq!(pre.status.code(), Some(1));
	let pre_out = String::from_utf8_lossy(&pre.stdout);
	assert!(pre_out.contains("install 1 plugin(s): com.foo.bar"), "got: {pre_out}");
	assert!(
		!pre_out.contains("com.already.here"),
		"must not re-install present plugin"
	);

	// apply: runs the fake launcher, which installs com.foo.bar
	let ap = run_env(&base, &extra, &["jb", "apply", cfg.to_str().unwrap(), "--os", "linux"]);
	assert!(ap.status.success(), "stderr: {}", String::from_utf8_lossy(&ap.stderr));
	assert!(data.join("IntelliJIdea2026.1/com.foo.bar/META-INF/plugin.xml").exists());

	// after: both present -> in sync (idempotent)
	let post = run_env(&base, &extra, &["jb", "check", cfg.to_str().unwrap(), "--os", "linux"]);
	assert_eq!(
		post.status.code(),
		Some(0),
		"out: {}",
		String::from_utf8_lossy(&post.stdout)
	);
}

#[cfg(unix)]
#[test]
fn missing_launcher_errors_clearly_on_apply() {
	let tmp = tempfile::tempdir().unwrap();
	let base = tmp.path().join("JetBrains");
	write(base.join("IntelliJIdea2026.1/options/.keep"), "");
	let data = tmp.path().join("data");
	fs::create_dir_all(&data).unwrap();
	let cfg = tmp.path().join("c.json");
	write(
		cfg.clone(),
		r#"{ "targets":[{"product":"IntelliJIdea","version":"2026.1"}], "plugins": { "install": ["com.foo.bar"] } }"#,
	);
	// IDESYNC_JB_LAUNCHER points nowhere, PATH search won't find "idea"
	let extra = [
		("IDESYNC_JB_DATA_HOME", data.to_str().unwrap()),
		("IDESYNC_JB_LAUNCHER", "/nonexistent/idea"),
		("PATH", "/nonexistent"),
	];
	let out = run_env(&base, &extra, &["jb", "apply", cfg.to_str().unwrap(), "--os", "linux"]);
	assert!(!out.status.success());
	assert!(String::from_utf8_lossy(&out.stderr).contains("cannot find the IDE launcher"));
}

#[test]
fn create_snapshots_settings_and_merges_schemes_across_ides() {
	// Two IDEs share a scheme named "ABC" with different language attributes.
	let tmp = tempfile::tempdir().unwrap();
	let base = tmp.path().join("JetBrains"); // IDESYNC_JB_CONFIG_HOME
	let data = tmp.path().join("data");
	fs::create_dir_all(&data).unwrap();

	let ws = base.join("WebStorm2026.1");
	write(
        ws.join("colors/ABC.icls"),
        "<scheme name=\"ABC\" version=\"1\" parent_scheme=\"Default\">\n  <attributes>\n    <option name=\"TEXT\"><value><option name=\"FOREGROUND\" value=\"c8d3f5\" /></value></option>\n    <option name=\"JS.LOCAL_VARIABLE\"><value><option name=\"FOREGROUND\" value=\"aabbcc\" /></value></option>\n  </attributes>\n</scheme>",
    );
	write(
		ws.join("options/editor-font.xml"),
		include_str!("fixtures/editor-font.xml"),
	);
	write(
        ws.join("options/colors.scheme.xml"),
        "<application>\n  <component name=\"EditorColorsManagerImpl\">\n    <global_color_scheme name=\"ABC\" />\n  </component>\n</application>",
    );

	let rr = base.join("RustRover2026.1");
	write(
        rr.join("colors/ABC.icls"),
        "<scheme name=\"ABC\" version=\"1\" parent_scheme=\"Default\">\n  <attributes>\n    <option name=\"TEXT\"><value><option name=\"FOREGROUND\" value=\"ffffff\" /></value></option>\n    <option name=\"org.rust.CRATE\"><value><option name=\"FOREGROUND\" value=\"ddeeff\" /></value></option>\n  </attributes>\n</scheme>",
    );

	let out = tmp.path().join("out");
	let res = run_env(
		&base,
		&[("IDESYNC_JB_DATA_HOME", data.to_str().unwrap())],
		&["jb", "create", "--out", out.to_str().unwrap(), "--primary", "WebStorm"],
	);
	assert!(res.status.success(), "stderr: {}", String::from_utf8_lossy(&res.stderr));

	// Cross-IDE merged scheme: attributes from BOTH IDEs in one file.
	let merged = read(out.join("color-schemes/ABC.icls"));
	assert!(
		merged.contains(r#"<option name="JS.LOCAL_VARIABLE">"#),
		"missing JS attr"
	);
	assert!(
		merged.contains(r#"<option name="org.rust.CRATE">"#),
		"missing Rust attr"
	);
	// WebStorm is primary, so its TEXT foreground wins the conflict.
	assert!(merged.contains(r#"value="c8d3f5""#));
	assert!(!merged.contains(r#"value="ffffff""#));

	// The config snapshots both targets, the font, and the active scheme reference.
	let cfg = read(out.join("idesync.json"));
	assert!(cfg.contains(r#""product": "WebStorm""#));
	assert!(cfg.contains(r#""product": "RustRover""#));
	assert!(cfg.contains(r#""name": "ABC""#));
	assert!(cfg.contains(r#""file": "color-schemes/ABC.icls""#));
	assert!(out.join("idesync-jetbrains.schema.json").exists());
}

#[test]
fn apply_writes_registry_settings_to_right_files() {
	let tmp = tempfile::tempdir().unwrap();
	let base = tmp.path().join("JetBrains");
	write(base.join("IntelliJIdea2026.1/options/.keep"), "");
	let cfg = tmp.path().join("c.json");
	write(
		cfg.clone(),
		r#"{
          "targets": [{ "product": "IntelliJIdea", "version": "2026.1" }],
          "settings": {
            "editor.codeVision": false,
            "general.processCloseConfirmation": "terminate",
            "ui.editorTabLimit": 30,
            "appearance.presentationModeScale": 1.75
          }
        }"#,
	);
	assert!(run(&base, &["jb", "apply", cfg.to_str().unwrap(), "--os", "linux"])
		.status
		.success());
	let ide = base.join("IntelliJIdea2026.1");
	assert!(read(ide.join("options/editor.xml")).contains(r#"<option name="enabled" value="false" />"#));
	assert!(read(ide.join("options/ide.general.xml"))
		.contains(r#"<option name="processCloseConfirmation" value="TERMINATE" />"#));
	assert!(read(ide.join("options/ui.lnf.xml")).contains(r#"<option name="EDITOR_TAB_LIMIT" value="30" />"#));
	assert!(read(ide.join("options/other.xml")).contains(r#"<option name="presentationModeIdeScale" value="1.75" />"#));

	// An unknown setting key is a hard error.
	let bad = tmp.path().join("bad.json");
	write(
		bad.clone(),
		r#"{ "targets": [{ "product": "IntelliJIdea", "version": "2026.1" }], "settings": { "nope.bad": true } }"#,
	);
	let out = run(&base, &["jb", "apply", bad.to_str().unwrap(), "--os", "linux"]);
	assert!(!out.status.success());
	assert!(String::from_utf8_lossy(&out.stderr).contains("unknown setting key 'nope.bad'"));
}

#[test]
fn editor_behavior_and_settings_compose_on_editor_xml() {
	// editorBehavior (typed) and settings (registry) both patch editor.xml.
	// Both sets of edits must survive — no clobbering.
	let tmp = tempfile::tempdir().unwrap();
	let base = tmp.path().join("JetBrains");
	write(base.join("IntelliJIdea2026.1/options/.keep"), "");
	let cfg = tmp.path().join("c.json");
	write(
		cfg.clone(),
		r#"{
          "targets": [{ "product": "IntelliJIdea", "version": "2026.1" }],
          "editorBehavior": { "softWrap": true, "showBreadcrumbs": false },
          "settings": { "editor.codeVision": false, "editor.animatedScrolling": false }
        }"#,
	);
	// editor.xml must appear exactly ONCE in the plan (composed, not duplicated).
	let dry = run(
		&base,
		&["jb", "apply", cfg.to_str().unwrap(), "--os", "linux", "--dry-run"],
	);
	let out = String::from_utf8_lossy(&dry.stdout);
	assert_eq!(
		out.matches("── options/editor.xml").count(),
		1,
		"editor.xml listed once: {out}"
	);

	assert!(run(&base, &["jb", "apply", cfg.to_str().unwrap(), "--os", "linux"])
		.status
		.success());
	let editor = read(base.join("IntelliJIdea2026.1/options/editor.xml"));
	// from editorBehavior:
	assert!(
		editor.contains(r#"<option name="USE_SOFT_WRAPS" value="MAIN_EDITOR" />"#),
		"{editor}"
	);
	assert!(
		editor.contains(r#"<option name="SHOW_BREADCRUMBS" value="false" />"#),
		"{editor}"
	);
	// from settings registry — must NOT have clobbered the above:
	assert!(
		editor.contains(r#"<option name="IS_ANIMATED_SCROLLING" value="false" />"#),
		"{editor}"
	);
	assert!(
		editor.contains(r#"<option name="enabled" value="false" />"#),
		"{editor}"
	);
}

#[test]
fn apply_copies_managed_files_verbatim() {
	let tmp = tempfile::tempdir().unwrap();
	let base = tmp.path().join("JetBrains");
	write(base.join("IntelliJIdea2026.1/options/.keep"), "");
	let dot = tmp.path().join("dot");
	write(
		dot.join("options/customization.xml"),
		"<application>\n  <component name=\"x\" />\n</application>",
	);
	write(dot.join("templates/React.xml"), "<templateSet group=\"React\" />");
	let cfg = dot.join("idesync.json");
	write(
		cfg.clone(),
		r#"{ "targets": [{ "product": "IntelliJIdea", "version": "2026.1" }],
            "files": ["options/customization.xml", "templates"] }"#,
	);
	assert!(run(&base, &["jb", "apply", cfg.to_str().unwrap(), "--os", "linux"])
		.status
		.success());
	let ide = base.join("IntelliJIdea2026.1");
	assert_eq!(
		read(ide.join("options/customization.xml")),
		"<application>\n  <component name=\"x\" />\n</application>"
	);
	assert!(ide.join("templates/React.xml").exists());
}

#[test]
fn apply_copies_per_target_files_to_each_ide_only() {
	// Window layouts are IDE-specific: each target's file lives under
	// targets/<product>/ and must land only in that IDE — never cross-contaminate.
	let tmp = tempfile::tempdir().unwrap();
	let base = tmp.path().join("JetBrains");
	write(base.join("IntelliJIdea2026.1/options/.keep"), "");
	write(base.join("WebStorm2026.1/options/.keep"), "");
	let dot = tmp.path().join("dot");
	write(dot.join("targets/IntelliJIdea/options/window.layouts.xml"), "IJ-LAYOUT");
	write(dot.join("targets/WebStorm/options/window.layouts.xml"), "WS-LAYOUT");
	let cfg = dot.join("idesync.json");
	write(
		cfg.clone(),
		r#"{ "targets": [
            { "product": "IntelliJIdea", "version": "2026.1", "files": ["options/window.layouts.xml"] },
            { "product": "WebStorm", "version": "2026.1", "files": ["options/window.layouts.xml"] }
        ] }"#,
	);
	assert!(run(&base, &["jb", "apply", cfg.to_str().unwrap(), "--os", "linux"])
		.status
		.success());
	// each IDE gets ITS OWN layout, not the other's
	assert_eq!(
		read(base.join("IntelliJIdea2026.1/options/window.layouts.xml")),
		"IJ-LAYOUT"
	);
	assert_eq!(
		read(base.join("WebStorm2026.1/options/window.layouts.xml")),
		"WS-LAYOUT"
	);
}

#[test]
fn create_captures_settings_theme_and_managed_files() {
	let tmp = tempfile::tempdir().unwrap();
	let base = tmp.path().join("JetBrains");
	let data = tmp.path().join("data");
	fs::create_dir_all(&data).unwrap();
	let rr = base.join("RustRover2026.1");
	write(
        rr.join("options/editor.xml"),
        "<application>\n  <component name=\"CodeVisionSettings\">\n    <option name=\"enabled\" value=\"false\" />\n  </component>\n</application>",
    );
	write(
        rr.join("options/laf.xml"),
        "<application>\n  <component name=\"LafManager\">\n    <laf themeId=\"ExperimentalDark\" />\n  </component>\n</application>",
    );
	write(rr.join("options/customization.xml"), "<application />");
	// Named tool-window layouts: portable, should be captured.
	write(
		rr.join("options/window.layouts.xml"),
		"<application>\n  <component name=\"ToolWindowLayout\"><![CDATA[{\"layouts\":{\"Custom\":{}}}]]></component>\n</application>",
	);
	// Per-monitor window geometry: machine-specific, must NOT be captured.
	write(
		rr.join("options/window.state.xml"),
		"<application>\n  <component name=\"DimensionService\" />\n</application>",
	);
	write(
		rr.join("templates/React.xml"),
		"<templateSet group=\"React\">\n  <template name=\"rcc\" value=\"x\" />\n</templateSet>",
	);

	let out = tmp.path().join("out");
	let res = run_env(
		&base,
		&[("IDESYNC_JB_DATA_HOME", data.to_str().unwrap())],
		&["jb", "create", "--out", out.to_str().unwrap()],
	);
	assert!(res.status.success(), "stderr: {}", String::from_utf8_lossy(&res.stderr));

	let cfg = read(out.join("idesync.json"));
	assert!(cfg.contains(r#""editor.codeVision": false"#), "settings: {cfg}");
	assert!(cfg.contains(r#""theme": "ExperimentalDark""#), "theme: {cfg}");
	assert!(cfg.contains(r#""options/customization.xml""#), "files: {cfg}");
	assert!(cfg.contains(r#""options/window.layouts.xml""#), "files: {cfg}");
	assert!(cfg.contains(r#""templates""#));
	// directories (live templates, …) are shared: copied at the IDE-relative path
	assert!(out.join("templates/React.xml").exists());
	// options/*.xml are IDE-specific: under targets/<product>/, NOT the shared path
	assert!(out.join("targets/RustRover/options/customization.xml").exists());
	assert!(out.join("targets/RustRover/options/window.layouts.xml").exists());
	assert!(!out.join("options/customization.xml").exists());
	assert!(!out.join("options/window.layouts.xml").exists());
	// per-monitor geometry is deliberately excluded (neither shared nor per-target)
	assert!(
		!cfg.contains("window.state.xml"),
		"window.state.xml must not be synced: {cfg}"
	);
	assert!(!out.join("options/window.state.xml").exists());
	assert!(!out.join("targets/RustRover/options/window.state.xml").exists());
}

#[test]
fn create_skips_empty_user_override_schemes() {
	let tmp = tempfile::tempdir().unwrap();
	let base = tmp.path().join("JetBrains");
	let data = tmp.path().join("data");
	fs::create_dir_all(&data).unwrap();
	let rr = base.join("RustRover2026.1");

	// Empty `_@user_` partialSave artifact (no colors/attributes) -> skipped.
	write(
        rr.join("colors/_@user_Dark.icls"),
        "<scheme name=\"_@user_Dark\" version=\"142\" parent_scheme=\"Darcula\">\n  <metaInfo>\n    <property name=\"partialSave\">true</property>\n  </metaInfo>\n</scheme>",
    );
	// Real override with a color -> kept.
	write(
        rr.join("colors/_@user_Darcula.icls"),
        "<scheme name=\"_@user_Darcula\" parent_scheme=\"Darcula\">\n  <colors>\n    <option name=\"FILESTATUS_ADDED\" value=\"80cbc4\" />\n  </colors>\n</scheme>",
    );

	let out = tmp.path().join("out");
	let res = run_env(
		&base,
		&[("IDESYNC_JB_DATA_HOME", data.to_str().unwrap())],
		&["jb", "create", "--out", out.to_str().unwrap()],
	);
	assert!(res.status.success(), "stderr: {}", String::from_utf8_lossy(&res.stderr));

	assert!(
		!out.join("color-schemes/_@user_Dark.icls").exists(),
		"empty override should be skipped"
	);
	assert!(
		out.join("color-schemes/_@user_Darcula.icls").exists(),
		"real override should be kept"
	);
}

#[test]
fn create_skips_bundled_only_template_groups() {
	let tmp = tempfile::tempdir().unwrap();
	let base = tmp.path().join("JetBrains");
	let data = tmp.path().join("data");
	fs::create_dir_all(&data).unwrap();
	let rr = base.join("RustRover2026.1");

	// A group that is purely disabled bundled templates (no custom content).
	write(
        rr.join("templates/Disabled.xml"),
        "<templateSet group=\"Disabled\">\n  <template name=\"a\" value=\"x\" deactivated=\"true\" />\n  <template name=\"b\" value=\"y\" deactivated=\"true\" />\n</templateSet>",
    );
	// A group with a real custom (active) template.
	write(
		rr.join("templates/Custom.xml"),
		"<templateSet group=\"Custom\">\n  <template name=\"mine\" value=\"hello\" />\n</templateSet>",
	);

	let out = tmp.path().join("out");
	let res = run_env(
		&base,
		&[("IDESYNC_JB_DATA_HOME", data.to_str().unwrap())],
		&["jb", "create", "--out", out.to_str().unwrap()],
	);
	assert!(res.status.success(), "stderr: {}", String::from_utf8_lossy(&res.stderr));

	// Only the custom group is copied; the all-disabled one is skipped.
	assert!(out.join("templates/Custom.xml").exists());
	assert!(!out.join("templates/Disabled.xml").exists());
	assert!(read(out.join("idesync.json")).contains(r#""templates""#));
}

#[test]
fn create_requires_primary_when_multiple_ides() {
	let tmp = tempfile::tempdir().unwrap();
	let base = tmp.path().join("JetBrains");
	write(base.join("WebStorm2026.1/options/.keep"), "");
	write(base.join("RustRover2026.1/options/.keep"), "");
	let out = tmp.path().join("out");

	// No --primary with two IDEs -> error that names --primary and the options.
	let res = run(&base, &["jb", "create", "--out", out.to_str().unwrap()]);
	assert!(!res.status.success());
	let err = String::from_utf8_lossy(&res.stderr);
	assert!(err.contains("--primary"), "stderr: {err}");
	assert!(err.contains("WebStorm") && err.contains("RustRover"), "stderr: {err}");

	// A bogus --primary is rejected too.
	let bad = run(
		&base,
		&["jb", "create", "--out", out.to_str().unwrap(), "--primary", "GoLand"],
	);
	assert!(!bad.status.success());
	assert!(String::from_utf8_lossy(&bad.stderr).contains("not among the IDEs"));
}

#[test]
fn create_allows_no_primary_for_single_ide() {
	let tmp = tempfile::tempdir().unwrap();
	let base = tmp.path().join("JetBrains");
	write(
		base.join("RustRover2026.1/options/editor-font.xml"),
		include_str!("fixtures/editor-font.xml"),
	);
	let out = tmp.path().join("out");
	let data = tmp.path().join("data");
	let res = run_env(
		&base,
		&[("IDESYNC_JB_DATA_HOME", data.to_str().unwrap())],
		&["jb", "create", "--out", out.to_str().unwrap()],
	);
	assert!(res.status.success(), "stderr: {}", String::from_utf8_lossy(&res.stderr));
	assert!(out.join("idesync.json").exists());
}

#[test]
fn list_discovers_android_studio_under_google_vendor() {
	// Android Studio is a Google product: ~/.config/Google/AndroidStudio*,
	// resolved as the sibling of the JetBrains config dir.
	let tmp = tempfile::tempdir().unwrap();
	let jb = tmp.path().join("JetBrains");
	write(jb.join("IntelliJIdea2026.1/options/.keep"), "");
	write(tmp.path().join("Google/AndroidStudio2026.1.1/options/.keep"), "");

	let out = run(&jb, &["list"]); // IDESYNC_JB_CONFIG_HOME = the JetBrains dir
	assert!(out.status.success());
	let s = String::from_utf8_lossy(&out.stdout);
	assert!(s.contains("IntelliJIdea2026.1"), "got: {s}");
	assert!(s.contains("AndroidStudio2026.1.1"), "missing Android Studio: {s}");
}

#[test]
fn list_discovers_seeded_ide() {
	let (_tmp, base, _cfg) = setup();
	let out = run(&base, &["list"]);
	assert!(out.status.success());
	assert!(String::from_utf8_lossy(&out.stdout).contains("IntelliJIdea2026.1"));
}

#[test]
fn keymap_command_generates_all_three_os_variants() {
	let (tmp, _base, cfg) = setup();
	let out_dir = tmp.path().join("generated");
	let out = run(
		&tmp.path().join("nonexistent-base"),
		&[
			"jb",
			"keymap",
			cfg.to_str().unwrap(),
			"--out",
			out_dir.to_str().unwrap(),
		],
	);
	assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
	assert!(out_dir.join("keymaps/Verona _Linux_.xml").exists());
	assert!(out_dir.join("keymaps/Verona _macOS_.xml").exists());
	assert!(out_dir.join("keymaps/Verona _Windows_.xml").exists());
}

// --- interactive mode guards -------------------------------------------------
// `Command::output()` captures stdout, so it is never a TTY → the interactive
// wizard never triggers here; these assert the off-terminal fallbacks instead of
// hanging on a prompt.

#[test]
fn apply_without_config_off_tty_errors_not_hangs() {
	let (_tmp, base, _cfg) = setup();
	let out = run(&base, &["jb", "apply"]);
	assert!(!out.status.success(), "missing config off a TTY must error");
	assert!(
		String::from_utf8_lossy(&out.stderr).contains("config path required"),
		"stderr: {}",
		String::from_utf8_lossy(&out.stderr)
	);
}

#[test]
fn interactive_flag_off_tty_errors_clearly() {
	let (_tmp, base, _cfg) = setup();
	let out = run(&base, &["jb", "apply", "--interactive"]);
	assert!(!out.status.success(), "-i off a TTY must error, not hang");
	assert!(
		String::from_utf8_lossy(&out.stderr).contains("requires a terminal"),
		"stderr: {}",
		String::from_utf8_lossy(&out.stderr)
	);
}
