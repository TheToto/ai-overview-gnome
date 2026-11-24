use serde::{Deserialize, Serialize};

pub const GOOGLE_LOGIN_URL: &str = "https://accounts.google.com";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    #[default]
    GoogleAiMode,
    Gemini,
    GoogleSearch,
    Antigravity,
}

impl Provider {
    pub fn all() -> &'static [Provider] {
        &[
            Provider::GoogleAiMode,
            Provider::Gemini,
            Provider::GoogleSearch,
            Provider::Antigravity,
        ]
    }

    pub fn available(has_antigravity: bool) -> &'static [Provider] {
        if has_antigravity {
            Self::all()
        } else {
            &[
                Provider::GoogleAiMode,
                Provider::Gemini,
                Provider::GoogleSearch,
            ]
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Provider::GoogleAiMode => "AI Mode",
            Provider::Gemini => "Gemini",
            Provider::GoogleSearch => "Google",
            Provider::Antigravity => "Antigravity",
        }
    }

    pub fn cli_slug(self) -> &'static str {
        match self {
            Provider::GoogleAiMode => "ai-mode",
            Provider::Gemini => "gemini",
            Provider::GoogleSearch => "google",
            Provider::Antigravity => "antigravity",
        }
    }

    pub fn index(self) -> u32 {
        match self {
            Provider::GoogleAiMode => 0,
            Provider::Gemini => 1,
            Provider::GoogleSearch => 2,
            Provider::Antigravity => 3,
        }
    }

    pub fn from_index(idx: u32) -> Self {
        match idx {
            1 => Provider::Gemini,
            2 => Provider::GoogleSearch,
            3 => Provider::Antigravity,
            _ => Provider::GoogleAiMode,
        }
    }

    pub fn from_str_loose(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "ai-mode" | "aimode" | "ai_mode" | "ai" => Some(Provider::GoogleAiMode),
            "gemini" => Some(Provider::Gemini),
            "google" | "search" | "google-search" => Some(Provider::GoogleSearch),
            "antigravity" | "agy" => Some(Provider::Antigravity),
            _ => None,
        }
    }

    pub fn query_url(self, query: &str) -> String {
        let encoded = urlencoding::encode(query.trim());
        match self {
            Provider::GoogleAiMode => format!("https://www.google.com/search?udm=50&q={encoded}"),
            Provider::GoogleSearch => format!("https://www.google.com/search?q={encoded}"),
            Provider::Gemini => "https://gemini.google.com/app".to_string(),
            Provider::Antigravity => String::new(),
        }
    }

    /// Generates a JS snippet that waits briefly for Gemini's prompt editor,
    /// optionally switches to Temporary Chat, fills the query, and triggers submission.
    pub fn gemini_inject_js(query: &str, temporary_chat: bool) -> String {
        let escaped = escape_js_string(query.trim());
        let temp_chat_flag = if temporary_chat { "true" } else { "false" };
        format!(
            r#"(function() {{
  const promptText = "{escaped}";
  if (!promptText) return;
  const wantTempChat = {temp_chat_flag};
  const dbg = (window.__aiOverviewDebug = window.__aiOverviewDebug || []);

  const editorSelectors = [
    'div.ql-editor[contenteditable="true"]',
    'rich-textarea [contenteditable="true"]',
    'textarea'
  ];

  const tempChatSelectors = [
    'button[aria-label*="discussions temporaires" i]',
    'button[aria-label*="temporary chat" i]',
    'button[data-test-id="temp-chat-button"]'
  ];

  const menuSelectors = [
    'button[data-test-id="main-menu-button"]',
    'button[aria-label*="Main menu" i]',
    'button[aria-label*="Menu principal" i]'
  ];

  function queryAny(selectors) {{
    for (const sel of selectors) {{
      const el = document.querySelector(sel);
      if (el) return el;
    }}
    return null;
  }}

  function isVisible(el) {{
    if (!el) return false;
    const rect = el.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  }}

  function findTempChatButton() {{
    const icon = document.querySelector(
      'mat-icon[fonticon="gemini_chat_temp"], mat-icon[data-mat-icon-name="gemini_chat_temp"]'
    );
    if (icon) {{
      return icon.closest('button, [role="button"], a') || icon;
    }}
    return queryAny(tempChatSelectors);
  }}

  function findMenuButton() {{
    const icon = document.querySelector(
      'mat-icon[fonticon="menu"], mat-icon[data-mat-icon-name="menu"]'
    );
    if (icon) {{
      const btn = icon.closest('button, [role="button"]');
      if (btn) return btn;
    }}
    return queryAny(menuSelectors);
  }}

  function findSendButton() {{
    return document.querySelector(
      'button[aria-label*="Send"], button[aria-label*="Envoyer"], button.send-button'
    );
  }}

  function fillAndSend() {{
    const editor = queryAny(editorSelectors);
    if (!editor) return false;

    editor.focus();
    if (editor.tagName === 'TEXTAREA') {{
      editor.value = promptText;
    }} else {{
      editor.textContent = promptText;
    }}
    editor.dispatchEvent(new Event('input', {{ bubbles: true }}));
    dbg.push('editor_filled');

    setTimeout(() => {{
      const btn = findSendButton();
      if (btn && !btn.disabled) {{
        btn.click();
        dbg.push('send_clicked');
      }} else {{
        dbg.push('send_btn_missing_or_disabled');
      }}
    }}, 200);
    return true;
  }}

  let attempts = 0;
  const maxAttempts = 50; // ~5s at 100ms
  let triedMenu = false;
  let editorSeenAt = null;

  const timer = setInterval(() => {{
    attempts++;

    if (!wantTempChat) {{
      if (fillAndSend() || attempts >= maxAttempts) {{
        clearInterval(timer);
      }}
      return;
    }}

    const tempBtn = findTempChatButton();
    if (tempBtn && isVisible(tempBtn)) {{
      clearInterval(timer);
      const label = tempBtn.getAttribute('aria-label') || tempBtn.className || tempBtn.tagName;
      dbg.push('temp_btn_found:' + label);
      if (tempBtn.getAttribute('aria-pressed') !== 'true') {{
        tempBtn.click();
        dbg.push('temp_btn_clicked');
        setTimeout(() => {{
          fillAndSend();
        }}, 500);
      }} else {{
        dbg.push('temp_btn_already_pressed');
        fillAndSend();
      }}
      return;
    }}

    const editor = queryAny(editorSelectors);
    if (editor) {{
      if (editorSeenAt === null) {{
        editorSeenAt = attempts;
      }}
      if (!triedMenu) {{
        triedMenu = true;
        const menuBtn = findMenuButton();
        if (menuBtn) {{
          menuBtn.click();
          dbg.push('menu_clicked:' + (menuBtn.getAttribute('aria-label') || menuBtn.tagName));
        }} else {{
          dbg.push('menu_btn_not_found');
        }}
      }}
      // Grace window (~1.5s) after editor appears to let sidebar/temp-chat mount, then fall back.
      if (attempts - editorSeenAt >= 15) {{
        clearInterval(timer);
        if (tempBtn) {{
          // Even if rect was 0, try clicking as last resort before fallback
          tempBtn.click();
          dbg.push('temp_btn_hidden_clicked');
          setTimeout(() => fillAndSend(), 500);
        }} else {{
          dbg.push('temp_btn_timeout_fallback');
          fillAndSend();
        }}
        return;
      }}
    }}

    if (attempts >= maxAttempts) {{
      clearInterval(timer);
      dbg.push('max_attempts_fallback');
      fillAndSend();
    }}
  }}, 100);
}})();"#,
        )
    }
}

fn escape_js_string(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_parsing_and_urls() {
        assert_eq!(Provider::from_str_loose("ai-mode"), Some(Provider::GoogleAiMode));
        assert_eq!(Provider::from_str_loose("GEMINI"), Some(Provider::Gemini));
        assert_eq!(Provider::from_str_loose("google"), Some(Provider::GoogleSearch));
        assert_eq!(Provider::from_str_loose("agy"), Some(Provider::Antigravity));
        assert_eq!(Provider::from_str_loose("unknown"), None);
        assert!(!Provider::available(false).contains(&Provider::Antigravity));
        assert!(Provider::available(true).contains(&Provider::Antigravity));

        assert_eq!(
            Provider::GoogleAiMode.query_url("rust gtk4"),
            "https://www.google.com/search?udm=50&q=rust%20gtk4"
        );
        assert_eq!(
            Provider::GoogleSearch.query_url("a+b"),
            "https://www.google.com/search?q=a%2Bb"
        );
        assert_eq!(
            Provider::Gemini.query_url("hello"),
            "https://gemini.google.com/app"
        );
    }

    #[test]
    fn gemini_js_escapes_quotes_and_newlines() {
        let js_temp = Provider::gemini_inject_js("hello \"world\"\nline 2", true);
        assert!(js_temp.contains(r#"hello \"world\"\nline 2"#));
        assert!(js_temp.contains("const wantTempChat = true;"));
        assert!(js_temp.contains("gemini_chat_temp"));
        assert!(js_temp.contains("temp-chat-button"));

        let js_normal = Provider::gemini_inject_js("hello", false);
        assert!(js_normal.contains("const wantTempChat = false;"));
    }
}
