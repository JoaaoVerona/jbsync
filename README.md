# idesync

Declaratively apply IDE/editor settings from **per-editor JSON configs**, the
same way on every OS. **Pluggable** editor support — JetBrains and VSCode today —
built to replace each editor's built-in settings sync with a single source of
truth you keep in Git.

Each editor family is a plugin with its own CLI namespace, config format, and
`create`/`apply`/`check` flow:

```bash
idesync list                 # discover installed editors/IDEs (every family)
idesync jb  apply config.json     # JetBrains (IntelliJ, RustRover, … incl. Android Studio)
idesync vsc apply config.json     # VSCode family (VS Code, Insiders, VSCodium, Cursor, Windsurf)
```

Adding a new editor is a new crate implementing one `Editor` trait — see
[CLAUDE.md](CLAUDE.md).

## Why

Most editors' built-in sync (JetBrains Settings Sync, VS Code Settings Sync) is a
cloud, last-writer-wins model with several independent writers (each editor on
each machine). When machines diverge there is no real merge — one snapshot wins,
and per-OS artifacts like macOS keymaps get dropped or overwritten. `idesync`
instead makes JSON files the **single source of truth** and the editors
**read-only consumers**:

- One writer → no silent overwrites between machines.
- Reviewable diffs in version control.
- Per-OS keymaps are _generated_ from one canonical definition, so they can't
  drift or get lost independently.

## How it works

**The JetBrains plugin (`jb`):**

- `options/*.xml` toggles are **surgically patched** — only the bytes that
  change are rewritten; comments, whitespace, and unmodeled settings are
  preserved verbatim. Output is deterministic and diffs are minimal.
- Color schemes / code styles are treated as **opaque managed files** (copied
  in and activated); you don't hand-edit hex colors in JSON.
- Keymaps are **generated per OS** from canonical bindings.
- `disabled_plugins.txt` is **merged** (never silently re-enables anything).
- Plugins can be **ensure-installed** via the IDE's `installPlugins` CLI — only
  the IDs not already present are installed (detected by reading installed
  plugins' descriptors), so `apply` stays idempotent.
- `*.vmoptions` is patched line-wise, preserving Toolbox-managed lines.
- A **`settings` registry** carries ~30 flat IDE options (editor, UI, general,
  updates, refactoring, terminal…) — one table drives both apply and capture.
- A **`files` list** copies whole self-contained settings files/dirs verbatim
  (menus/toolbars, live/file templates, inspection profiles, Grazie,
  notifications, parameter hints, file types, VCS/debugger/diff, advanced
  settings) — the catch-all for anything not worth a typed key.

**The VSCode plugin (`vsc`):** pure pass-through for VS Code, Insiders, VSCodium,
Cursor, and Windsurf — `settings.json` is **surgically merged** (comments +
unmanaged keys preserved), `keybindings.json` is owned like a generated keymap,
and extensions are ensure-installed via the editor CLI. See
[VSCode-family editors](#vscode-family-editors-vsc).

## Install

### Released binary (Linux / macOS / Windows)

```bash
# Linux / macOS
curl -fsSL https://github.com/JoaaoVerona/idesync/releases/latest/download/install.sh | sh

# Windows (PowerShell)
iwr https://github.com/JoaaoVerona/idesync/releases/latest/download/install.ps1 | iex
```

Installs the `idesync` binary to `~/.local/bin` (override with
`IDESYNC_INSTALL_DIR`); add that directory to your `PATH` if it isn't already.
Pin a version by passing a tag, e.g.
`curl -fsSL …/install.sh | sh -s v0.1.0`.

### With Cargo

```bash
cargo install --git https://github.com/JoaaoVerona/idesync idesync   # latest from main
cargo install --path crates/idesync                                  # from a local checkout
```

### From source

```bash
cargo build --release   # binary at target/release/idesync
```

This repo ships a [Runfile](Runfile.json) — `run build`, `run test`,
`run lint`, and `run install` (build a release binary and copy it to
`~/.local/bin`) — if you have the [`run`](https://github.com/Skiley/runfile)
task runner installed.

## Usage

`idesync list` is global; every other command lives under an editor namespace
(`jb` for JetBrains, `vsc` for VSCode). Each takes its **own** config file.

```bash
idesync list                          # discover installed editors/IDEs (every family)

# JetBrains (jb)
idesync jb create --out ./dotfiles    # snapshot current IDE settings -> portable config
idesync jb check  jb.json             # report drift (exit 1 if any), changes nothing
idesync jb apply  jb.json --dry-run   # show exact diffs, write nothing
idesync jb apply  jb.json             # apply (backs up overwritten files)
idesync jb keymap jb.json --out .     # generate per-OS keymaps to ./keymaps/

# VSCode (vsc)
idesync vsc create --out ./vscode     # snapshot settings/keybindings/extensions
idesync vsc check  vsc.json           # report drift
idesync vsc apply  vsc.json           # apply
```

Useful flags on `jb apply`/`jb check`: `--product`, `--version`, `--os`
(`linux|macos|windows`, default: host), and `--exclude <section>` (repeatable)
to skip a config section entirely — it is neither applied nor reported as drift.
Sections mirror the config keys: `editor`, `terminal`, `console`, `ui`,
`editor-behavior`, `settings`, `color-scheme`, `code-style`, `keymap`, `files`,
`plugins`, `vm-options` (e.g. `--exclude plugins --exclude keymap`). `apply` also
takes `--no-backup`. `vsc apply`/`vsc check` take `--product <editor>` (e.g.
`Code`, `Cursor`) and `--targets-only`.

> ⚠ The editor rewrites its config on exit and reads it on startup — **close the
> target editor before `apply`**, or it will clobber the changes.

### Snapshotting current settings (`create`)

`idesync jb create --out DIR` is the reverse of `apply`: it reads your installed
IDEs and writes a portable, self-contained config you can commit:

- `DIR/idesync.json` — extracted fonts, UI/editor toggles, registry settings,
  active scheme + code style, per-target plugins, vmoptions heap, and the active
  keymap reversed into bindings.
- `DIR/idesync-jetbrains.schema.json` — a copy of the schema (for editor autocomplete).
- `DIR/color-schemes/*.icls` and `DIR/code-styles/*.xml` — the scheme files
  themselves, **merged across IDEs**.
- `DIR/templates/…`, `DIR/fileTemplates/…`, `DIR/inspectionProfiles/…` — shared
  managed dirs from the primary (same user content for every IDE).
- `DIR/targets/<product>/options/…` — IDE-specific `options/*.xml` (menus, file
  types, window layouts, …), one copy per IDE, applied to that IDE only.

Everything but `idesync.json` / `idesync-jetbrains.schema.json` is organised into
subdirectories. `create` is strictly read-only with respect to your IDEs — it
only writes into `DIR`.

**Cross-IDE scheme merge.** Different IDEs flesh out the _same_ named scheme with
different language pieces — WebStorm's "ABC" carries `JS.*`/`CSS.*` attributes,
RustRover's carries `org.rust.*`, Android Studio's carries Kotlin/Compose. `create`
groups same-named schemes across all IDEs and unions their `<colors>` and
`<attributes>` (and, for code styles, their per-language sections), producing one
file that highlights every language. Conflicts on real colors resolve to the
**primary** IDE.

A merged `<attributes>` entry is either a real color (an `<option>` with a
`<value>`) or a bare `baseAttributes` _inheritance pointer_ (e.g.
`<option name="INSTANCE_FIELD_ATTRIBUTES" baseAttributes="DEFAULT_INSTANCE_FIELD" />`).
A real color always wins over a pointer, and among real colors the primary wins.
Bare pointers, though, are IDE-_specific defaults_, not portable colors: the same
attribute is `baseAttributes="DEFAULT_INSTANCE_FIELD"` in a Java IDE but
`baseAttributes=""` in WebStorm. When sources disagree on a pointer, `create`
**omits** it so each IDE falls back to its own bundled default on `apply` —
otherwise the primary's pointer would repaint that token in the other IDEs (this
was the cause of "wrong colors in WebStorm"). Identical pointers are kept.

Empty `_@user_*` override schemes — `partialSave` artifacts the IDE writes when
you merely select/tweak a read-only bundled scheme, with no actual
colors/attributes — are skipped as noise (non-empty overrides are kept).

The primary supplies all single-valued settings (fonts, toggles, active
scheme/style, heap, keymap) and breaks scheme-merge conflicts. It has **no
default** — when more than one IDE is found you must choose it with `--primary`:

```bash
idesync jb create --out ./dotfiles --primary IntelliJIdea  # all IDEs, IntelliJ leads
idesync jb create --out ./dotfiles --product WebStorm      # just one (no --primary needed)
idesync jb create --out ./dotfiles --primary IntelliJIdea --portable-keymap
```

`--portable-keymap` emits the host's primary keyboard modifier as **`mod`**
instead of literal Ctrl/Cmd, so the same config gives Ctrl shortcuts on
Linux/Windows and **Cmd shortcuts on macOS** when applied. Mouse shortcuts and
non-primary modifiers stay literal. Without the flag, shortcuts are captured
verbatim (e.g. `ctrl+1` stays Ctrl on every OS).

Notes / limits: scalar settings (fonts, toggles, active scheme name, heap) come
from the **primary** IDE (a single idesync config can't express per-IDE scalar
differences). **Plugins are emitted per-target** — each IDE keeps its own
`disabled`/`install` set, so applying never over-disables.

### Plugin installation

`plugins.install` lists Marketplace plugin IDs to ensure present. On `apply`,
idesync scans the installed-plugins dir (the data dir) to see what's already
there and runs the IDE's `installPlugins` CLI only for the missing ones:

```jsonc
"plugins": {
  "install": ["ru.adelf.idea.dotenv", "io.kotest.plugin.intellij"],
  "repositories": ["https://plugins.example.com/updatePlugins.xml"],  // optional
  "disabled": ["com.intellij.spring"]
}
```

**Per-target overrides.** A `plugins` block on a `targets[]` entry is **unioned**
with the top-level one for that IDE only — so you can disable Spring in IntelliJ
without touching WebStorm. (Union means a per-target block can _add_ to the
global set, not remove from it — keep IDE-specific entries out of the global
block.) `create` uses this to emit each IDE's real plugin set per-target.

```jsonc
"plugins": { "disabled": ["common.everywhere"] },          // applies to all
"targets": [
  { "product": "IntelliJIdea", "plugins": { "disabled": ["com.intellij.spring"] } },
  { "product": "WebStorm",     "plugins": { "install":  ["dev.blachut.svelte-intellij"] } }
]
```

A `targets[]` entry also takes its own **`files`** list for IDE-specific verbatim
files (window layouts, etc.) — sourced from `targets/<product>/<path>` and applied
to that IDE only, never shared. See [Flat settings & managed files](#flat-settings--managed-files).

Caveats specific to installs (they're the one networked, imperative step):

- Needs the **IDE launcher** — found via PATH, the Toolbox `scripts/` dir, or
  the `IDESYNC_JB_LAUNCHER` override. Close the IDE first; `installPlugins` runs it
  headless and downloads from Marketplace.
- **Install-only**: the CLI can't uninstall/disable. Use `plugins.disabled` to
  disable; idesync does not remove plugins.
- Versions aren't pinnable via this CLI — it installs the latest compatible
  build, so installs are less deterministic than the file patching.

### Flat settings & managed files

Beyond the typed sections, two general mechanisms cover the long tail:

```jsonc
"console": { "font": { "family": "JetBrains Mono", "size": 15 } },
"ui": { "theme": "ExperimentalDark" },          // LAF theme id (best-effort)

"settings": {                                    // flat IDE options (registry)
  "editor.codeVision": false,
  "editor.animatedScrolling": false,
  "general.processCloseConfirmation": "terminate",
  "terminal.optionAsMeta": true,
  "markdown.previewFontSize": 16
},

"files": [                                        // SHARED: copied into every IDE
  "templates", "fileTemplates", "inspectionProfiles"
],
"targets": [                                       // IDE-specific options/*.xml + layouts
  { "product": "WebStorm", "files": [
      "options/customization.xml",                // menus & toolbars (per IDE)
      "options/filetypes.xml", "options/vcs.xml",
      "options/window.layouts.xml"
  ] },
  { "product": "RustRover", "files": [
      "options/customization.xml", "options/window.layouts.xml"
  ] }
]
```

- **`settings`** keys are validated against a fixed registry (see
  [`settings.rs`](crates/jetbrains/src/settings.rs) / the schema's `settings` block); an unknown
  key is an error. Both `apply` and `create` are driven by that one table.
- **`files`** entries are config-relative paths (a directory is copied
  recursively) installed verbatim into **every** IDE. Don't list files that other
  sections option-patch (e.g. `editor.xml`, `ui.lnf.xml`). On `create`, only
  cross-IDE user content goes here — the `templates/`, `fileTemplates/`, and
  `inspectionProfiles/` dirs from the primary. The IDE-specific `options/*.xml`
  (menus/toolbars, file types, debugger, diff, notifications, parameter hints,
  VCS, advanced settings, window layouts) go on each `targets[]` entry's own
  `files` instead — stored under `targets/<product>/<path>` next to the config and
  applied to that IDE only, so one IDE's menus/file-types never overwrite
  another's.
    - **Live templates** get special handling on `create`: the `templates/` dir is
      often dominated by records of _disabled bundled_ templates (every entry
      `deactivated="true"`), not custom content, so `create` only copies template
      groups that contain at least one active template. Add `"templates"` to
      `files` manually if you want the full dir copied regardless.
    - **`options/*.xml` are IDE-specific.** Menus/toolbars (`customization.xml`),
      file types, debugger, diff, notifications, parameter hints, VCS, advanced
      settings, and window layouts (`window.layouts.xml`) all differ between IDEs —
      each has its own actions, tool windows, and languages. `create` snapshots
      each IDE's copy separately under `targets/<product>/` and records it on that
      IDE's `targets[].files` (not the shared `files`), so on `apply` each IDE gets
      its own back and none overwrites another's. It intentionally skips
      `options/window.state.xml`, which holds per-monitor pixel geometry
      (resolution/DPI-keyed) that should not follow you between machines.

### Environment overrides

The JetBrains config base is resolved per-OS (`~/.config/JetBrains`,
`~/Library/Application Support/JetBrains`, `%APPDATA%\JetBrains`); the
installed-plugins base is the data dir (`~/.local/share/JetBrains`, …).
**Android Studio** is a Google product, so it lives under the sibling `Google`
vendor dir (`~/.config/Google/AndroidStudio*`) — discovery handles this
automatically. Override the JetBrains base with `IDESYNC_JB_CONFIG_HOME`,
`IDESYNC_JB_DATA_HOME`, and `IDESYNC_JB_LAUNCHER` (used by the tests and handy for
dry-running against a copy); the Google base is derived as its sibling.

**VSCode** editors are resolved under the same per-OS config base
(`~/.config/<AppDir>`, `~/Library/Application Support/<AppDir>`,
`%APPDATA%\<AppDir>`, where `<AppDir>` is `Code`, `Cursor`, …). Override the base
with `IDESYNC_VSC_CONFIG_HOME` and the home dir used to find each editor's
`extensions/` with `IDESYNC_VSC_HOME`.

## Config (JetBrains)

Each editor has its own schema:
[`idesync-jetbrains.schema.json`](crates/jetbrains/schema/idesync-jetbrains.schema.json)
and [`idesync-vscode.schema.json`](crates/vscode/schema/idesync-vscode.schema.json).
The JetBrains sections below cover every `jb` field; the VSCode config is
described under [VSCode-family editors](#vscode-family-editors-vsc). `create`
copies the matching schema next to the config it writes, so reference it for
editor autocomplete:

```json
{ "$schema": "./idesync-jetbrains.schema.json", "...": "..." }
```

The quickest way to a real config is `idesync jb create --out ./dotfiles`, which
writes a complete one snapshotted from your own IDEs.

### Keymaps & the Ctrl/Cmd model

Keystrokes use modifiers + a key, joined by `+` or spaces:

| Token                                       | Resolves to                                                   |
| ------------------------------------------- | ------------------------------------------------------------- |
| `mod`                                       | **Ctrl** on Linux/Windows, **Cmd** on macOS (platform-native) |
| `ctrl`                                      | literal Control on **every** OS                               |
| `meta`/`cmd`/`win`, `alt`/`option`, `shift` | literal                                                       |

This captures the common real-world setup: native actions (`$Copy`, `$Paste`,
…) use `mod` so they become Cmd on macOS, while your custom muscle-memory
shortcuts use literal `ctrl` and stay Ctrl everywhere. A comma is a two-stroke
chord, e.g. `"ctrl+k, ctrl+s"`. A `buttonN` token makes it a **mouse shortcut**:
`"ctrl+button1"` (Ctrl+click), `"button1+doubleClick"` (double-click). An action
can have both (e.g. `["alt+enter", "button1+doubleClick"]`).

```jsonc
"keymap": {
  "name": "Verona",
  "bindings": {
    "$Copy": "mod+c",          // ctrl c on Linux, meta c on macOS
    "ReformatCode": "mod+1",   // ctrl 1 / meta 1
    "ActivateTerminalToolWindow": "ctrl+b", // ctrl b on BOTH
    "CopyElement": []          // remove the inherited shortcut
  }
}
```

### VSCode-family editors (`vsc`)

`idesync vsc …` syncs **VSCode-family editors** — VS Code, VS Code Insiders,
VSCodium, Cursor, and Windsurf (anything that reuses the VSCode config layout).
Because these share none of JetBrains' formats or vocabulary (settings keys,
command IDs), this is **pass-through, not translated**: a standalone VSCode config
provides raw VSCode values that idesync applies to every targeted editor.

```jsonc
// vsc.json — its own file, separate from the JetBrains config
{
  "$schema": "./idesync-vscode.schema.json",

  // Empty/omitted `targets` = apply to every discovered VSCode-family editor.
  "targets": ["Code", "Cursor"],

  // Merged into settings.json. Only THESE keys are (re)written — every other
  // key, plus your comments and formatting, is preserved.
  "settings": {
    "editor.fontSize": 15,
    "editor.fontFamily": "JetBrains Mono",
    "editor.fontLigatures": true,
    "workbench.colorTheme": "Default Dark Modern"
  },

  // The keybindings.json array. When present, idesync OWNS keybindings.json
  // (like a generated keymap) — seed it from your current bindings with `create`.
  // `mod` = the platform-primary modifier (Ctrl on Linux/Windows, Cmd on macOS).
  "keybindings": [
    { "key": "mod+d", "command": "editor.action.addSelectionToNextFindMatch" },
    { "key": "ctrl+shift+k", "command": "editor.action.deleteLines" }
  ],

  // Ensure-installed via the editor CLI (`code --install-extension …`); only
  // the missing ones are installed, so `apply` stays idempotent.
  "extensions": { "install": ["rust-lang.rust-analyzer", "esbenp.prettier-vscode"] }
}
```

How it differs from the JetBrains side:

- **`settings.json` is merged surgically** (the JSON counterpart of the XML
  patcher) — listed keys are set, everything else is left byte-for-byte intact.
- **`keybindings.json` is owned wholesale** — it holds only your bindings, so
  idesync regenerates it from `keybindings` the way `jb` generates a keymap.
- **The `mod` token** is the cross-platform Ctrl/Cmd convenience. VSCode keeps a
  single `keybindings.json` and uses native per-platform overrides (`mac`/`linux`/
  `win`) instead of per-OS files — so it has no "generate N files" problem — but
  it won't derive Cmd from Ctrl for *custom* bindings. idesync bridges that:
  `"key": "mod+d"` is **expanded on apply** to `"key": "ctrl+d"` + `"mac": "cmd+d"`
  (an explicit `mac` you write is always respected). `ctrl` stays literal Control
  everywhere. This mirrors the JetBrains `mod` token.
- **Extensions** are detected from the editor's `extensions.json` manifest and
  the missing IDs installed via the editor's CLI (looked up on `PATH`).
- It's a **separate config + crate** from JetBrains; nothing is shared or
  translated between the two.

`idesync vsc create --out DIR` snapshots your installed VSCode editors into
`DIR/idesync.json` (and `idesync list` shows which editors were discovered).
`--product Code` targets just that editor; `--portable-keymap` folds captured
`ctrl` keys with a matching `cmd` macOS override back into the `mod` token.

## Recommended workflow

1. Turn **off** each editor's built-in sync (JetBrains Settings Sync, VS Code
   Settings Sync) — otherwise you have two writers again.
2. Keep your `jb.json` / `vsc.json` (+ any `.icls`/code-style files) in a dotfiles
   Git repo.
3. `idesync jb check` / `idesync vsc check` in CI / a pre-commit hook to catch drift.
4. `idesync jb apply` / `idesync vsc apply` on each machine after pulling.

## Scope (prototype)

Modeled today: editor/terminal/UI fonts, a curated set of UI + editor-behavior
toggles, registry escape-hatch, color scheme + code style install/activate,
plugin install (ensure-present) + disable, vmoptions heap/extra lines, and full
per-OS keymap generation. Deliberately **not** modeled: plugin _uninstall_ (the
CLI can't), and every obscure `options/*.xml` flag — the philosophy is to
JSON-model only stable scalars and treat big artifacts as managed files.

**VSCode** support is deliberately pass-through: idesync syncs your raw
`settings.json` / `keybindings.json` / extension list across editors and
machines, but does **not** translate JetBrains keymaps or settings into VSCode
equivalents (the action/command vocabularies don't map cleanly). Extension
_uninstall_ is likewise out of scope.

## Tests

```bash
cargo test          # or: run test  (whole workspace)
```

Unit tests cover the XML patcher, the JSONC patcher, and the keymap transform;
integration tests drive the real compiled binary against seeded editor configs in
a temp dir, redirected via the `IDESYNC_JB_*` / `IDESYNC_VSC_*` env overrides
(surgical patching, idempotency, drift detection, dry-run safety, the Ctrl/Cmd
transform, scheme install, plugin merge, vmoptions; VSCode settings merge,
owned keybindings, and capture).
