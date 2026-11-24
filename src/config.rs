use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::provider::Provider;

pub const APP_ID: &str = "io.github.toto.AiOverviewDesktop";
const APP_DIR_NAME: &str = "ai-overview-desktop";

pub const ANTIGRAVITY_MODELS: &[(&str, &str)] = &[
    ("gemini-4-hydrogen-low", "Gemini 4 Hydrogen (Low)"),
    ("gemini-3.8-flash-low", "Gemini 3.8 Flash (Low)"),
    ("gemini-4-hydrogen-medium", "Gemini 4 Hydrogen (Medium)"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SpeechToText {
    None,
    #[default]
    Handy,
}

impl SpeechToText {
    pub fn all() -> &'static [SpeechToText] {
        &[SpeechToText::None, SpeechToText::Handy]
    }

    pub fn label(self) -> &'static str {
        match self {
            SpeechToText::None => crate::i18n::tr("None", "Aucun"),
            SpeechToText::Handy => "Handy",
        }
    }

    pub fn index(self) -> u32 {
        match self {
            SpeechToText::None => 0,
            SpeechToText::Handy => 1,
        }
    }

    pub fn from_index(idx: u32) -> Self {
        match idx {
            1 => SpeechToText::Handy,
            _ => SpeechToText::None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    pub provider: Provider,
    pub background_mode: bool,
    pub gemini_temporary_chat: bool,
    pub speech_to_text: SpeechToText,
    pub gnome_shortcut: String,
    pub gnome_shortcut_provider: Option<Provider>,
    pub gnome_voice_shortcut: String,
    pub gnome_voice_shortcut_provider: Option<Provider>,
    pub antigravity_auto_approve: bool,
    pub antigravity_model: Option<String>,
    pub width: i32,
    pub height: i32,
    pub clean_css: bool,
    pub user_agent: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            provider: Provider::GoogleAiMode,
            background_mode: false,
            gemini_temporary_chat: true,
            speech_to_text: SpeechToText::Handy,
            gnome_shortcut: "<Super><Alt>a".to_string(),
            gnome_shortcut_provider: None,
            gnome_voice_shortcut: String::new(),
            gnome_voice_shortcut_provider: None,
            antigravity_auto_approve: true,
            antigravity_model: Some("gemini-4-hydrogen-low".to_string()),
            width: 740,
            height: 540,
            clean_css: true,
            user_agent: None,
        }
    }
}

impl Config {
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(APP_DIR_NAME)
            .join("config.toml")
    }

    pub fn data_dir() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(APP_DIR_NAME)
    }

    pub fn cache_dir() -> PathBuf {
        dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(APP_DIR_NAME)
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        let Ok(raw) = fs::read_to_string(&path) else {
            return Self::default();
        };
        toml::from_str(&raw).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create config dir: {e}"))?;
        }
        let body = toml::to_string_pretty(self).map_err(|e| format!("serialize config: {e}"))?;
        fs::write(&path, body).map_err(|e| format!("write config: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_toml_roundtrip() {
        let cfg = Config {
            provider: Provider::Gemini,
            background_mode: true,
            gemini_temporary_chat: false,
            speech_to_text: SpeechToText::Handy,
            gnome_shortcut: "<Super>space".to_string(),
            gnome_shortcut_provider: Some(Provider::GoogleAiMode),
            gnome_voice_shortcut: "<Super><Alt>v".to_string(),
            gnome_voice_shortcut_provider: Some(Provider::Antigravity),
            antigravity_auto_approve: false,
            antigravity_model: Some("gemini-flash".to_string()),
            width: 920,
            height: 700,
            clean_css: false,
            user_agent: Some("CustomUA/1.0".to_string()),
        };
        let serialized = toml::to_string_pretty(&cfg).expect("serialize");
        let parsed: Config = toml::from_str(&serialized).expect("deserialize");
        assert_eq!(cfg, parsed);
    }
}
