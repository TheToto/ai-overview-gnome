# AGENTS.md

## Overview
`ai-overview-gnome` (`ai-overview-desktop` crate) is a Rust + GTK4 + Libadwaita + WebKitGTK 6.0 Spotlight-style overlay for GNOME Linux. It lets the user query Google AI Mode (`udm=50`), Gemini Web, Google Search, or Antigravity CLI (`agy`) using a persistent Google account session (no API key required), supporting keyboard/voice-typed input, CLI arguments, piped `stdin`, and clipboard input.

## Repository Map
- `Cargo.toml` — Crate manifest (`gtk4 = 0.11`, `libadwaita = 0.9`, `webkit6 = 0.6`). All three must share the same `gtk4` major/minor generation.
- `src/main.rs` — `adw::Application` setup (`HANDLES_COMMAND_LINE`), D-Bus single-instance dispatch, pre-GTK `stdin` handling, and CSS loading.
- `src/i18n.rs` — System locale (`LC_ALL`/`LC_MESSAGES`/`LANG`) EN/FR translation helper (`tr`).
- `src/config.rs` — `APP_ID` and TOML config (`~/.config/ai-overview-desktop/config.toml`), XDG data/cache dir helpers.
- `src/provider.rs` — `Provider` enum (`GoogleAiMode`, `Gemini`, `GoogleSearch`, `Antigravity`), availability filtering, URL builders, and Gemini DOM injection JS.
- `src/extractor.rs` — Native reader extraction JS and Pango markup converter for Google AI Mode responses and follow-ups.
- `src/webview.rs` — `webkit6::WebView` with persistent `NetworkSession`, SQLite cookie storage, and scoped `UserStyleSheet`.
- `src/voice.rs` — Audio capture and speech-to-text integration (`handy` / WAV).
- `src/antigravity.rs` — `agy` CLI availability check, `stream-json` event parser (`AgyToolStep` + text deltas), and Markdown-to-Pango renderer.
- `src/shortcut.rs` — Dual GNOME custom keyboard shortcut registration (`gsettings`, Flatpak-aware via `flatpak-spawn --host`).
- `src/preferences.rs` — Libadwaita `PreferencesDialog` (default provider, background mode, voice dictation, dual shortcuts, Antigravity settings).
- `src/window.rs` — Borderless Spotlight `adw::ApplicationWindow`, `SpotlightController`, native `PopoverMenu`, 2-level Antigravity tool viewer, and keyboard shortcuts (`Esc`, `Ctrl+G`, `Ctrl+L`, `Ctrl+Q`, `Ctrl+,`).
- `data/` — `style.css` (embedded via `include_str!`), `.desktop`, `.metainfo.xml`, and `.svg` icon.
- `gnome-extension/ai-overview-dock@toto/` — Companion GNOME Shell extension for top-center docking on Wayland.
- `flatpak/io.github.toto.AiOverviewDesktop.json` & `.github/workflows/build-and-publish.yml` — Flatpak manifest and CI publishing OSTree repo + `.flatpakref` to GitHub Pages and `.flatpak` bundle to GitHub Releases.

## Build & Verify
```bash
cargo check --all-targets
cargo test
cargo build
./target/debug/ai-overview-desktop --help
```

## Key Constraints
- **GNOME Wayland**: Applications cannot force exact `(x, y)` coordinates on Wayland. Use a borderless `adw::ApplicationWindow` with `WindowHandle` for dragging.
- **Google OAuth**: Never inject custom CSS or JS into `accounts.google.com`. Keep `UserStyleSheet` allowlists scoped to `https://www.google.com/search*`.
- **D-Bus Single Instance**: `stdin` is read before `app.run_with_args()` in `src/main.rs` and converted into `--query` so piped input works even when forwarding to an already-running background instance.
- **Git**: Never run `git push` directly after making changes unless the user explicitly asks to push.
