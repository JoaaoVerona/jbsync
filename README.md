# jbsync

Declaratively apply JetBrains IDE settings from **one JSON config**, the same
way on every OS. Built to replace JetBrains "Settings Sync" / "Backup and Sync"
with a single source of truth you keep in Git.

## Why

JetBrains Settings Sync is a cloud, last-writer-wins model with several
independent writers (each IDE on each machine). When machines diverge there is
no real merge — one snapshot wins, and per-OS artifacts like macOS keymaps get
dropped or overwritten. `jbsync` instead makes a JSON file the **single source
of truth** and the IDEs **read-only consumers**:

- One writer → no silent overwrites between machines.
- Reviewable diffs in version control.
- Per-OS keymaps are *generated* from one canonical definition, so they can't
  drift or get lost independently.

## How it works

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

## Install / build

```bash
cargo build --release
# binary at target/release/jbsync
```

This repo also ships a [Runfile](Runfile.json) — `run build`, `run test`,
`run lint`, `run ci` — if you have the [`run`](https://github.com/Skiley/runfile)
task runner installed.

## Usage

```bash
jbsync list                       # discover installed JetBrains IDEs
jbsync create --out ./dotfiles    # snapshot current IDE settings -> portable config
jbsync check  config.json         # report drift (exit 1 if any), changes nothing
jbsync apply  config.json --dry-run   # show exact diffs, write nothing
jbsync apply  config.json         # apply (backs up overwritten files)
jbsync keymap config.json --out . # generate per-OS keymaps to ./keymaps/
```

Useful flags on `apply`/`check`: `--product`, `--version`, `--os`
(`linux|macos|windows`, default: host). `apply` also takes `--no-backup`.

> ⚠ The IDE rewrites its config on exit and reads it on startup — **close the
> target IDE before `apply`**, or it will clobber the changes.

### Snapshotting current settings (`create`)

`jbsync create --out DIR` is the reverse of `apply`: it reads your installed
IDEs and writes a portable, self-contained config you can commit:

- `DIR/jbsync.json` — extracted fonts, UI/editor toggles, registry settings,
  active scheme + code style, per-target plugins, vmoptions heap, and the active
  keymap reversed into bindings.
- `DIR/jbsync.schema.json` — a copy of the schema (for editor autocomplete).
- `DIR/color-schemes/*.icls` and `DIR/code-styles/*.xml` — the scheme files
  themselves, **merged across IDEs**.
- `DIR/options/…`, `DIR/templates/…`, … — managed files, kept at their
  IDE-relative paths (so `apply` mirrors them back to the right place).

Everything but `jbsync.json` / `jbsync.schema.json` is organised into
subdirectories. `create` is strictly read-only with respect to your IDEs — it
only writes into `DIR`.

**Cross-IDE scheme merge.** Different IDEs flesh out the *same* named scheme with
different language pieces — WebStorm's "ABC" carries `JS.*`/`CSS.*` attributes,
RustRover's carries `org.rust.*`, Android Studio's carries Kotlin/Compose. `create`
groups same-named schemes across all IDEs and unions their `<colors>` and
`<attributes>` (and, for code styles, their per-language sections), producing one
file that highlights every language. Conflicts resolve to the **primary** IDE.
Empty `_@user_*` override schemes — `partialSave` artifacts the IDE writes when
you merely select/tweak a read-only bundled scheme, with no actual
colors/attributes — are skipped as noise (non-empty overrides are kept).

The primary supplies all single-valued settings (fonts, toggles, active
scheme/style, heap, keymap) and breaks scheme-merge conflicts. It has **no
default** — when more than one IDE is found you must choose it with `--primary`:

```bash
jbsync create --out ./dotfiles --primary IntelliJIdea  # all IDEs, IntelliJ leads
jbsync create --out ./dotfiles --product WebStorm      # just one (no --primary needed)
jbsync create --out ./dotfiles --primary IntelliJIdea --portable-keymap
```

`--portable-keymap` emits the host's primary keyboard modifier as **`mod`**
instead of literal Ctrl/Cmd, so the same config gives Ctrl shortcuts on
Linux/Windows and **Cmd shortcuts on macOS** when applied. Mouse shortcuts and
non-primary modifiers stay literal. Without the flag, shortcuts are captured
verbatim (e.g. `ctrl+1` stays Ctrl on every OS).

Notes / limits: scalar settings (fonts, toggles, active scheme name, heap) come
from the **primary** IDE (a single jbsync config can't express per-IDE scalar
differences). **Plugins are emitted per-target** — each IDE keeps its own
`disabled`/`install` set, so applying never over-disables.

### Plugin installation

`plugins.install` lists Marketplace plugin IDs to ensure present. On `apply`,
jbsync scans the installed-plugins dir (the data dir) to see what's already
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
without touching WebStorm. (Union means a per-target block can *add* to the
global set, not remove from it — keep IDE-specific entries out of the global
block.) `create` uses this to emit each IDE's real plugin set per-target.

```jsonc
"plugins": { "disabled": ["common.everywhere"] },          // applies to all
"targets": [
  { "product": "IntelliJIdea", "plugins": { "disabled": ["com.intellij.spring"] } },
  { "product": "WebStorm",     "plugins": { "install":  ["dev.blachut.svelte-intellij"] } }
]
```

Caveats specific to installs (they're the one networked, imperative step):

- Needs the **IDE launcher** — found via PATH, the Toolbox `scripts/` dir, or
  the `JBSYNC_LAUNCHER` override. Close the IDE first; `installPlugins` runs it
  headless and downloads from Marketplace.
- **Install-only**: the CLI can't uninstall/disable. Use `plugins.disabled` to
  disable; jbsync does not remove plugins.
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

"files": [                                        // copied verbatim into the IDE
  "options/customization.xml",                    // menus & toolbars
  "options/grazie_global.xml",                    // spelling/grammar
  "options/filetypes.xml", "options/vcs.xml",
  "templates", "fileTemplates", "inspectionProfiles"
]
```

- **`settings`** keys are validated against a fixed registry (see
  [`settings.rs`](src/settings.rs) / the schema's `settings` block); an unknown
  key is an error. Both `apply` and `create` are driven by that one table.
- **`files`** entries are config-relative paths (a directory is copied
  recursively) installed verbatim. Don't list files that other sections
  option-patch (e.g. `editor.xml`, `ui.lnf.xml`). `create` auto-collects a
  curated managed set from the primary IDE.
  - **Live templates** get special handling on `create`: the `templates/` dir is
    often dominated by records of *disabled bundled* templates (every entry
    `deactivated="true"`), not custom content, so `create` only copies template
    groups that contain at least one active template. Add `"templates"` to
    `files` manually if you want the full dir copied regardless.

### Environment overrides

The JetBrains config base is resolved per-OS (`~/.config/JetBrains`,
`~/Library/Application Support/JetBrains`, `%APPDATA%\JetBrains`); the
installed-plugins base is the data dir (`~/.local/share/JetBrains`, …).
**Android Studio** is a Google product, so it lives under the sibling `Google`
vendor dir (`~/.config/Google/AndroidStudio*`) — discovery handles this
automatically. Override the JetBrains base with `JBSYNC_CONFIG_HOME`,
`JBSYNC_DATA_HOME`, and `JBSYNC_LAUNCHER` (used by the tests and handy for
dry-running against a copy); the Google base is derived as its sibling.

## Config

See [`schema/jbsync.schema.json`](schema/jbsync.schema.json) for the full,
auto-completing schema (the sections below cover every field). Reference the
schema from your config for editor support:

```json
{ "$schema": "./schema/jbsync.schema.json", "...": "..." }
```

The quickest way to a real config is `jbsync create --out ./dotfiles`, which
writes a complete one snapshotted from your own IDEs.

### Keymaps & the Ctrl/Cmd model

Keystrokes use modifiers + a key, joined by `+` or spaces:

| Token | Resolves to |
|-------|-------------|
| `mod` | **Ctrl** on Linux/Windows, **Cmd** on macOS (platform-native) |
| `ctrl` | literal Control on **every** OS |
| `meta`/`cmd`/`win`, `alt`/`option`, `shift` | literal |

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

## Recommended workflow

1. Turn **off** JetBrains Settings Sync (otherwise you have two writers again).
2. Keep `config.json` (+ any `.icls`/code-style files) in a dotfiles Git repo.
3. `jbsync check` in CI / a pre-commit hook to catch drift.
4. `jbsync apply` on each machine after pulling.

## Scope (prototype)

Modeled today: editor/terminal/UI fonts, a curated set of UI + editor-behavior
toggles, registry escape-hatch, color scheme + code style install/activate,
plugin install (ensure-present) + disable, vmoptions heap/extra lines, and full
per-OS keymap generation. Deliberately **not** modeled: plugin *uninstall* (the
CLI can't), and every obscure `options/*.xml` flag — the philosophy is to
JSON-model only stable scalars and treat big artifacts as managed files.

## Tests

```bash
cargo test
```

Unit tests cover the XML patcher and keymap transform; integration tests drive
the real compiled binary against seeded copies of real JetBrains files in a temp
dir (surgical patching, idempotency, drift detection, dry-run safety, the
Ctrl/Cmd transform, scheme install, plugin merge, vmoptions).
