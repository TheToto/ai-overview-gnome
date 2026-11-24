# AI Overview GNOME

A minimalist Spotlight-style AI launcher for GNOME, built with **GTK4**, **Libadwaita**, and **WebKitGTK 6.0**.

![AI Overview GNOME](docs/screenshot.png)

## Features

- **Google AI Mode**, **Gemini**, **Google Search**, and optional **Antigravity CLI** (`agy`) with a 2-level tool action viewer.
- Uses your existing Google session — no API key required.
- Voice dictation (`--voice`), clipboard (`--clipboard`), CLI / `stdin`, and background mode (`--background`).
- Dual GNOME keyboard shortcuts (Normal & Voice), each with an optional preset AI provider.
- Automatic **English / French** localization based on system locale.

## Install (Flatpak)

### Recommended (automatic updates via `flatpak update`)

[Download `io.github.toto.AiOverviewDesktop.flatpakref`](https://thetoto.github.io/ai-overview-gnome/io.github.toto.AiOverviewDesktop.flatpakref) and open it with GNOME Software, or run:

```bash
flatpak install --user https://thetoto.github.io/ai-overview-gnome/io.github.toto.AiOverviewDesktop.flatpakref
```

### Standalone `.flatpak` bundle

Download the latest `.flatpak` file from [GitHub Releases](https://github.com/TheToto/ai-overview-gnome/releases/latest):

```bash
flatpak install --user io.github.toto.AiOverviewDesktop-*-x86_64.flatpak
```

## GNOME Shell Extension (optional)

Centers the launcher flush at the top of the screen on GNOME Wayland:

```bash
mkdir -p ~/.local/share/gnome-shell/extensions
cp -r gnome-extension/ai-overview-dock@toto ~/.local/share/gnome-shell/extensions/
# Log out and back in on Wayland, then:
gnome-extensions enable ai-overview-dock@toto
```

## Build from source

```bash
cargo build --release
```
