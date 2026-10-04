# Localization

Settings, tray menus/tooltips, and daemon-owned toast and dialog text support `en`
and `zh-CN`. Select **Settings → Appearance → Language**, or set
`[appearance] language = "zh-CN"` in `config.toml` and reload. English is the default;
unsupported values become `en` with one config-validation warning. No OS-language
detection or `auto` setting is provided. The open Settings window and tray menu
change language without restarting the daemon.

**Simplified Chinese is a machine draft awaiting native-speaker review.** Its
catalog header records this status; completeness checks do not establish translation
quality. Please review terminology, clarity, punctuation, and layout before claiming
human review.

## Files and lookup

- `crates/daemon/locales/en.toml`: English source of truth.
- `crates/daemon/locales/zh-CN.toml`: Simplified Chinese machine draft.
- `crates/daemon/src/locale.rs`: parsing, supported identifiers, bundled registration,
  English fallback, and named-placeholder substitution.
- `crates/daemon/src/settings/html.rs`: page consumers; the host injects the resolved
  table. Static text uses `data-i18n` and attribute-specific key bindings; generated
  HTML escapes translations with `escHtml`/`escAttr`.

Each locale is one **flat TOML table** of strings. Quote dotted keys so TOML does not
interpret them as nested tables:

```toml
"settings.language.label" = "Language"
"notification.tiling_paused.body" = "Window placement did not finish within {seconds} seconds, so tiling was automatically paused. Choose Resume Tiling from the tray after the windows settle."
```

Keys are stable dotted identifiers grouped by surface (`settings`, `tray`,
`notification`, `dialog`). Use semantic names such as
`settings.layout.gap.description`; retain existing keys when changing wording.
Do not put English fallbacks into Rust or JavaScript: add the English entry here.
Values are plain text, not HTML. Formatting markup belongs to the page, never to a
translation. Brand names, author names, URLs, license identifiers, numeric examples,
and command/key tokens retain their literal identity. The language selector uses
autonyms: `English` and `简体中文`.

Named placeholders use `{name}` (ASCII letters/underscores followed by letters,
underscores, or digits). Preserve each key's placeholder names exactly. Substitution
is single-pass: inserted values are never reinterpreted as placeholders. Unknown
placeholders remain visible, making missing arguments diagnosable. Do not introduce
HTML markup or translate placeholder names. Plural-sensitive sentences use separate
keys where needed; this is not a general pluralization engine.

A missing translated key falls back to English at runtime. A missing English key is
a product bug, logged and displayed as its key rather than causing a panic. Invalid
TOML, duplicate keys, nested tables, and non-string values return loader errors. A
broken bundled non-English catalog falls back to English with a warning. Bundled
catalog tests still reject missing translations and orphan keys, so runtime fallback
is not a substitute for completing a translation.

All catalogs use `include_str!`: **editing a locale file requires rebuilding the
daemon**. Locale files are not loaded from the installed filesystem; installer/WiX
changes are unnecessary.

## Adding or editing a language

1. Edit the existing UTF-8 catalog, or copy `en.toml` to a new locale filename and
   translate every value. Preserve the flat structure and stable keys.
2. For a new language, register its exact identifier in `SUPPORTED_LANGUAGES` and
   its embedded file/lookup in `locale.rs`. Add its autonym and identifier to the
   Settings Language combobox. Do not add OS detection or normalize identifiers.
3. Extend the bundled-catalog key/placeholder tests to cover the new registration.
   Update the config-template supported-value comment and user documentation.
4. Run from the repository root:

   ```powershell
   cargo test -p leopardwm-daemon locale::tests
   cargo test -p leopardwm-daemon settings::html::tests
   cargo fmt --all -- --check
   git diff --check
   pwsh -NoProfile -File tools/check.ps1
   ```

   The Settings JavaScript rendering test requires Node.js on `PATH`. It executes
   the embedded page script without a browser or WebView and exercises literal
   text/attribute insertion, escaped generated markup, and language relabeling.
   Config tests cover the default, legacy configs, round-trip, and warning path.
5. Rebuild the daemon. Arrange separately authorized native UI acceptance for long
   labels, text scaling, high contrast, and non-ASCII WebView/tray rendering. Unit
   tests do not prove native visual or physical-input behavior.

## Deliberately untranslated surfaces

These remain English in this change:

- CLI output and help.
- Logs, including config-validation and locale-loader warnings.
- IPC/config keys and values, command identifiers, and key-chord tokens.
- Shared hotkey command labels/descriptions in Settings, including gesture-command
  choices: they come from `ipc::hotkeys::hotkey_catalog()` shared with CLI, IPC, and
  PowerToys Shortcut Guide. Page-owned headers, tooltips, warnings, and the
  no-action choice are localized.
- The watchdog process toast.
- The installer.

Operating-system-owned dialog buttons use Windows' language, not this setting.
Error details originating in IPC/Win32 may remain English inside an otherwise
localized daemon dialog; the daemon-owned explanation is localized.
