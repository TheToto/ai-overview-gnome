use std::fs;

use webkit6::{
    CookiePersistentStorage, NetworkSession, Settings, UserContentInjectedFrames,
    UserContentManager, UserStyleLevel, UserStyleSheet, WebView,
};

use crate::config::Config;

const GOOGLE_SEARCH_CLEAN_CSS: &str = r#"
/* Subtle cleanup for Google Search / AI Mode inside Spotlight */
#gb, header[role="banner"], #taw, #botstuff, footer {
    opacity: 0.96;
}
"#;

pub fn create_webview(config: &Config) -> WebView {
    let data_dir = Config::data_dir().join("webkit-data");
    let cache_dir = Config::cache_dir().join("webkit-cache");

    let _ = fs::create_dir_all(&data_dir);
    let _ = fs::create_dir_all(&cache_dir);

    let data_str = data_dir.to_string_lossy();
    let cache_str = cache_dir.to_string_lossy();

    let session = NetworkSession::new(Some(&data_str), Some(&cache_str));
    if let Some(cookie_manager) = session.cookie_manager() {
        let cookie_file = data_dir.join("cookies.sqlite");
        cookie_manager.set_persistent_storage(
            &cookie_file.to_string_lossy(),
            CookiePersistentStorage::Sqlite,
        );
    }

    let content_manager = UserContentManager::new();
    if config.clean_css {
        // Scoped strictly to Google Search pages so accounts.google.com login is untouched.
        let sheet = UserStyleSheet::new(
            GOOGLE_SEARCH_CLEAN_CSS,
            UserContentInjectedFrames::TopFrame,
            UserStyleLevel::User,
            &["https://www.google.com/search*"],
            &[],
        );
        content_manager.add_style_sheet(&sheet);
    }

    let settings = Settings::builder()
        .enable_javascript(true)
        .enable_developer_extras(true)
        .build();

    if let Some(ua) = &config.user_agent {
        settings.set_user_agent(Some(ua));
    }

    WebView::builder()
        .network_session(&session)
        .user_content_manager(&content_manager)
        .settings(&settings)
        .vexpand(true)
        .hexpand(true)
        .build()
}
