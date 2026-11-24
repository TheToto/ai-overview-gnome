use crate::config::{host_command, is_flatpak};
use crate::i18n::tr;
use crate::provider::Provider;

const MEDIA_KEYS_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
const CUSTOM_BINDING_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutKind {
    Normal,
    Voice,
}

impl ShortcutKind {
    pub fn path(self) -> &'static str {
        match self {
            ShortcutKind::Normal => {
                "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/ai-overview/"
            }
            ShortcutKind::Voice => {
                "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/ai-overview-voice/"
            }
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            ShortcutKind::Normal => "AI Overview",
            ShortcutKind::Voice => tr("AI Overview (Voice)", "AI Overview (Vocal)"),
        }
    }
}

pub fn build_shortcut_command(kind: ShortcutKind, provider: Option<Provider>) -> String {
    let base_cmd = if is_flatpak() {
        "flatpak run io.github.toto.AiOverviewDesktop".to_string()
    } else {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.to_str().map(str::to_owned))
            .unwrap_or_else(|| "ai-overview-desktop".to_string());

        if exe.contains(' ') {
            format!("\"{exe}\"")
        } else {
            exe
        }
    };

    build_shortcut_command_for_exe(&base_cmd, kind, provider)
}

pub fn build_shortcut_command_for_exe(
    base_cmd: &str,
    kind: ShortcutKind,
    provider: Option<Provider>,
) -> String {
    let mut cmd = base_cmd.to_string();
    if kind == ShortcutKind::Voice {
        cmd.push_str(" --voice");
    }
    if let Some(p) = provider {
        cmd.push_str(" --provider ");
        cmd.push_str(p.cli_slug());
    }
    cmd
}

pub fn register_gnome_shortcut(
    kind: ShortcutKind,
    binding: &str,
    command: &str,
) -> Result<(), String> {
    let trimmed_binding = binding.trim();
    if trimmed_binding.is_empty() {
        return Err(tr("Shortcut cannot be empty.", "Le raccourci ne peut pas être vide.").to_string());
    }

    let get_out = host_command("gsettings")
        .args(["get", MEDIA_KEYS_SCHEMA, "custom-keybindings"])
        .output()
        .map_err(|e| format!("{}: {e}", tr("Failed to run gsettings", "Impossible d'exécuter gsettings")))?;

    if !get_out.status.success() {
        return Err(String::from_utf8_lossy(&get_out.stderr).trim().to_string());
    }

    let raw_list = String::from_utf8_lossy(&get_out.stdout);
    let mut paths = parse_gvariant_string_list(&raw_list);
    let path = kind.path();
    if !paths.iter().any(|p| p == path) {
        paths.push(path.to_string());
    }

    let formatted_list = format_gvariant_string_list(&paths);
    run_gsettings_set(&[MEDIA_KEYS_SCHEMA, "custom-keybindings", &formatted_list])?;

    let relocatable = format!("{CUSTOM_BINDING_SCHEMA}:{path}");
    run_gsettings_set(&[&relocatable, "name", kind.display_name()])?;
    run_gsettings_set(&[&relocatable, "command", command])?;
    run_gsettings_set(&[&relocatable, "binding", trimmed_binding])?;

    log_info!(
        "shortcut",
        "Registered {:?} shortcut '{}' -> '{}'",
        kind,
        trimmed_binding,
        command
    );
    Ok(())
}

pub fn set_gnome_shortcut_binding(kind: ShortcutKind, binding: &str) -> Result<(), String> {
    let relocatable = format!("{CUSTOM_BINDING_SCHEMA}:{}", kind.path());
    if let Err(e) = run_gsettings_set(&[&relocatable, "binding", binding]) {
        log_warn!("shortcut", "Failed to set binding '{binding}' for {:?}: {e}", kind);
        return Err(e);
    }
    Ok(())
}

pub fn open_gnome_keyboard_settings() -> Result<(), String> {
    log_info!("shortcut", "Opening GNOME keyboard settings");
    host_command("gnome-control-center")
        .arg("keyboard")
        .spawn()
        .map(|_| ())
        .map_err(|e| {
            log_err!("shortcut", "Unable to open gnome-control-center: {e}");
            format!(
                "{}: {e}",
                tr(
                    "Unable to open GNOME Settings",
                    "Impossible d'ouvrir les paramètres GNOME"
                )
            )
        })
}

const EXTENSION_JS: &str = include_str!("../gnome-extension/ai-overview-dock@toto/extension.js");
const EXTENSION_META: &str = include_str!("../gnome-extension/ai-overview-dock@toto/metadata.json");

pub fn is_gnome_extension_installed() -> bool {
    if is_flatpak() {
        host_command("sh")
            .args([
                "-c",
                "[ -f \"$HOME/.local/share/gnome-shell/extensions/ai-overview-dock@toto/extension.js\" ]",
            ])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    } else {
        dirs::data_local_dir()
            .map(|d| d.join("gnome-shell/extensions/ai-overview-dock@toto/extension.js").is_file())
            .unwrap_or(false)
    }
}

/// Installs/updates the GNOME Shell extension in `~/.local/share/gnome-shell/extensions/`
/// and returns `Ok(true)` if `gnome-extensions enable` succeeded immediately,
/// or `Ok(false)` if a GNOME session restart is needed to discover it.
pub fn install_gnome_extension() -> Result<bool, String> {
    let cache_dir = crate::config::Config::cache_dir();
    std::fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("create cache dir failed: {e}"))?;

    let tmp_js = cache_dir.join("extension.js");
    let tmp_meta = cache_dir.join("metadata.json");
    std::fs::write(&tmp_js, EXTENSION_JS).map_err(|e| format!("write extension.js: {e}"))?;
    std::fs::write(&tmp_meta, EXTENSION_META).map_err(|e| format!("write metadata.json: {e}"))?;

    let copy_out = host_command("sh")
        .args([
            "-c",
            "dest=\"$HOME/.local/share/gnome-shell/extensions/ai-overview-dock@toto\" && mkdir -p \"$dest\" && cp \"$1\" \"$dest/extension.js\" && cp \"$2\" \"$dest/metadata.json\"",
            "sh",
            &tmp_js.to_string_lossy(),
            &tmp_meta.to_string_lossy(),
        ])
        .output()
        .map_err(|e| format!("install extension failed: {e}"))?;

    let _ = std::fs::remove_file(&tmp_js);
    let _ = std::fs::remove_file(&tmp_meta);

    if !copy_out.status.success() {
        let err = String::from_utf8_lossy(&copy_out.stderr).trim().to_string();
        log_err!("extension", "Failed to copy extension files: {err}");
        return Err(if err.is_empty() {
            "Copie de l'extension échouée".to_string()
        } else {
            err
        });
    }

    let enable_ok = host_command("gnome-extensions")
        .args(["enable", "ai-overview-dock@toto"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    log_info!(
        "extension",
        "Installed ai-overview-dock@toto (enabled_now={enable_ok})"
    );
    Ok(enable_ok)
}

fn run_gsettings_set(args: &[&str]) -> Result<(), String> {
    let out = host_command("gsettings")
        .arg("set")
        .args(args)
        .output()
        .map_err(|e| format!("gsettings set {}: {e}", tr("failed", "échoué")))?;

    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(())
}

pub fn parse_gvariant_string_list(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed == "@as []" || trimmed == "[]" || trimmed.is_empty() {
        return Vec::new();
    }
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);

    inner
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_matches('\'').trim_matches('"').to_string())
        .collect()
}

pub fn format_gvariant_string_list(items: &[String]) -> String {
    if items.is_empty() {
        return "@as []".to_string();
    }
    let quoted: Vec<String> = items
        .iter()
        .map(|s| format!("'{}'", s.replace('\'', "\\'")))
        .collect();
    format!("[{}]", quoted.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_gvariant_lists() {
        assert!(parse_gvariant_string_list("@as []").is_empty());
        assert_eq!(
            parse_gvariant_string_list(
                "['/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/']"
            ),
            vec!["/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/"]
        );
        let multi = vec!["/a/b/".to_string(), "/c/d/".to_string()];
        let formatted = format_gvariant_string_list(&multi);
        assert_eq!(formatted, "['/a/b/', '/c/d/']");
        assert_eq!(parse_gvariant_string_list(&formatted), multi);
    }

    #[test]
    fn builds_dual_shortcut_commands() {
        assert_eq!(
            build_shortcut_command_for_exe("ai-overview-desktop", ShortcutKind::Normal, None),
            "ai-overview-desktop"
        );
        assert_eq!(
            build_shortcut_command_for_exe(
                "ai-overview-desktop",
                ShortcutKind::Normal,
                Some(Provider::Antigravity)
            ),
            "ai-overview-desktop --provider antigravity"
        );
        assert_eq!(
            build_shortcut_command_for_exe("ai-overview-desktop", ShortcutKind::Voice, None),
            "ai-overview-desktop --voice"
        );
        assert_eq!(
            build_shortcut_command_for_exe(
                "ai-overview-desktop",
                ShortcutKind::Voice,
                Some(Provider::Antigravity)
            ),
            "ai-overview-desktop --voice --provider antigravity"
        );
    }
}
