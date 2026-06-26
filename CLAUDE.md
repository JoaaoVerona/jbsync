# jbsync — notes for future work

Rust CLI that applies JetBrains IDE settings from one JSON config, cross-platform.
Replaces JetBrains Settings Sync with a single Git-tracked source of truth.

## Build / test / lint

Use the Runfile (`run :list` to see all targets) — not raw cargo:

```bash
run build           # cargo build  (run build --release for optimized)
run build:release   # optimized release binary
run test            # unit + integration tests
run lint            # FORMAT (cargo fmt) + clippy -D warnings  (modifies files)
run check           # VERIFY (no changes): cargo check + fmt --check + clippy
run cli <args>      # run the binary, e.g. run cli list
run install         # build release + copy binary to ~/.local/bin (local dev install)
run release patch   # bump version (major|minor|patch), tag, push -> CI builds release
```

## Project conventions (mirrors the sibling `runfile` project)

- `rustfmt.toml` uses **hard tabs**, max width 120 — `run lint` formats + lints.
- `rust-toolchain.toml` pins the toolchain (clippy + rustfmt).
- `.editorconfig` / `.gitattributes` (LF) enforce the same in editors.
- `.github/workflows/ci.yml` — lint + test (ubuntu/windows) via `run`, using the
  `Skiley/runfile` setup action to install the `run` CLI.
- `.github/workflows/release.yml` — on a `v*.*.*` tag, calls the reusable
  `Skiley/rust-binary-publish` workflow to build cross-platform binaries + GitHub
  release (npm publish disabled). It also renders and attaches `install.sh` /
  `install.ps1` (env prefix `JBSYNC`, installs to `~/.local/bin`) — the
  `curl … | sh` install in the README. The `release` Runfile target creates the tag.

## Layout

- `src/xmlpatch.rs` — the core: surgical, byte-minimal patching of JetBrains
  `options/*.xml` via quick-xml event streams. **Most correctness risk lives here.**
- `src/appliers/` — map config slices to `FileChange`s (pure: old content → new content),
  plus `PluginInstall` actions. `build_plan` returns a `Plan { files, installs }`.
    - **`PatchSet`** (in `mod.rs`): option-patched files go through ONE accumulator so
      several appliers can touch the same file (e.g. `editor.xml` ← `editor_behavior` +
      `named_settings`) without the last write clobbering the others. Any new
      option-patching applier must use `ps.patch(...)`, NOT read the file independently.
    - `keymap.rs` — per-OS keymap generation + the `mod`/`ctrl` (Ctrl/Cmd) transform.
    - `plugins.rs` — disable (file) + ensure-install (detect installed IDs by reading
      `META-INF/plugin.xml` unpacked or inside `lib/*.jar`, then `installPlugins` the missing).
- `src/launcher.rs` — find the IDE launcher (override → PATH → Toolbox `scripts/`).
- `src/default_keymap.rs` — read JetBrains' *bundled* default keymaps (`keymaps/<name>.xml`
  inside a `lib/*.jar` of the IDE install, located via the launcher). Used only by
  `create --portable-keymap`: a user keymap file stores only *deviations* from its parent,
  so inherited bindings (Find = Ctrl+F) never appear and the Ctrl→Cmd port can't act on them.
  `extract::resolve_default_chain` walks the parent chain into a flat binding set and
  materialises the inherited **primary-modifier** bindings explicitly (only those change
  meaning across platforms — function keys / Alt-combos stay inherited). User overrides always
  win. NB: `keystroke_to_spec` matches the modifier *family* (`ctrl`/`control`, `meta`/`cmd`)
  since the jars use the long AWT spelling; `keymap::resolve_keystroke` preserves key case so
  named keys (`MINUS`, `F2`, `ENTER`) survive. Jar discovery tries the launcher first, then
  scans well-known install roots (`locate_keymap_jar`) — Windows Toolbox lives under
  `%LOCALAPPDATA%`, NOT the roaming dir `find_launcher` derives, so the root scan is what makes
  a Windows `create` work. If discovery still fails, `create` warns loudly and the user can set
  `JBSYNC_LAUNCHER`; only explicit overrides are captured in that case.
- `src/discovery.rs` — config base (`JBSYNC_CONFIG_HOME`) + data/plugins base (`JBSYNC_DATA_HOME`).
- `src/extract.rs` — `create`: reverse of apply. Reads scalars (via `xmlpatch::get_*`),
  reverses the keymap into bindings, unions installed plugins, and orchestrates scheme merge.
  Read-only w.r.t. IDEs; only writes the output dir. `Config` derives `Serialize` for this.
- `src/scheme_merge.rs` — merge same-named color schemes (union `<colors>`/`<attributes>` by
  option name) and code styles (union top-level children by tag + name/language). First source
  (primary) wins conflicts; handles self-closing/empty roots. **Attribute gotcha:** an
  `<attributes>` option is either a real color (`<value>`) or a bare `baseAttributes`
  inheritance pointer (an IDE-specific default, not a portable color). `resolve_attribute`
  prefers concrete colors over pointers, and *drops* a pointer when sources disagree so each
  IDE falls back to its own default — otherwise the primary's pointer repaints that token in
  other IDEs (was the "wrong colors in WebStorm" bug).
- `src/settings.rs` — registry of flat `<option>` settings (key → file/component/option/kind).
  Drives both apply (`appliers/named_settings.rs`) and capture (`extract::extract_settings`).
  **Adding a setting = one row here + one line in the schema's `settings` block.**
- `src/appliers/files.rs` — verbatim copy of `config.files` (shared, into every IDE) PLUS
  per-target `Target.files` (IDE-specific) sourced from `targets/<product>/<path>` and applied
  to that IDE only. On `create`: shared `MANAGED_FILES` = the user-content dirs only
  (`templates`, `fileTemplates`, `inspectionProfiles`) from the primary; per-target
  `PER_TARGET_FILES` = every `options/*.xml` (menus/`customization.xml`, file types, debugger,
  diff, notifications, parameter hints, VCS, advanced settings, `window.layouts.xml`) snapshotted
  from each IDE separately — they differ per IDE (actions, tool windows, languages) so the
  primary's copy must not be imposed on the others. `window.state.xml` (per-monitor geometry) is
  excluded entirely.
- `src/cli.rs` — `apply` / `check` / `create` / `list` / `keymap`; diff, backup, atomic write, run installs.
- `schema/jbsync.schema.json` — user-facing JSON Schema (keep in sync with `src/config.rs`).
- `tests/fixtures/` — real (sanitized) JetBrains files used by integration tests.

## Conventions / gotchas

- Self-closing tags must be `<.. />` (space before `/>`) to match JetBrains; see `empty_tag`.
- Preserve a file's existing trailing-newline style (vmoptions/plugins) to stay idempotent.
- Appliers must be idempotent: `apply` then `check` must report in-sync.
- Keymap filename sanitisation mirrors JetBrains: non `[A-Za-z0-9 _.-]` → `_`.

## Known non-goals (prototype)

Plugin _uninstall_ (installPlugins CLI can't) and exhaustive `options/*.xml`
coverage. Add new settings by curating a stable
scalar mapping in an applier + schema; treat big artifacts as managed files.
