use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
}

static CURRENT_LANG: OnceLock<Lang> = OnceLock::new();

pub fn detect_lang_from_str(raw: &str) -> Lang {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.starts_with("fr") {
        Lang::Fr
    } else {
        Lang::En
    }
}

pub fn current_lang() -> Lang {
    *CURRENT_LANG.get_or_init(|| {
        for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(val) = std::env::var(var)
                && !val.trim().is_empty()
            {
                return detect_lang_from_str(&val);
            }
        }
        Lang::En
    })
}

pub fn tr(en: &'static str, fr: &'static str) -> &'static str {
    match current_lang() {
        Lang::Fr => fr,
        Lang::En => en,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_french_and_english_locales() {
        assert_eq!(detect_lang_from_str("fr_FR.UTF-8"), Lang::Fr);
        assert_eq!(detect_lang_from_str("fr"), Lang::Fr);
        assert_eq!(detect_lang_from_str("en_US.UTF-8"), Lang::En);
        assert_eq!(detect_lang_from_str("C"), Lang::En);
        assert_eq!(detect_lang_from_str(""), Lang::En);
    }
}
