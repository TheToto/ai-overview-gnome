use gtk4::glib;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct InlineSpan {
    pub kind: String,
    pub text: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ContentBlock {
    pub kind: String,
    pub spans: Vec<InlineSpan>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ExtractedAnswer {
    pub blocks: Vec<ContentBlock>,
    #[serde(default)]
    pub suggestions: Vec<String>,
    pub raw_len: usize,
}

impl ExtractedAnswer {
    pub fn from_json(json: &str) -> Option<Self> {
        let parsed: Self = serde_json::from_str(json).ok()?;
        let total_chars: usize = parsed
            .blocks
            .iter()
            .flat_map(|b| b.spans.iter())
            .map(|s| s.text.chars().count())
            .sum();
        if parsed.blocks.is_empty() || total_chars < 10 {
            return None;
        }
        Some(parsed)
    }

    pub fn to_pango_markup(&self) -> String {
        let mut out = String::new();
        let mut prev_was_bullet = false;

        for block in &self.blocks {
            let mut inner = String::new();
            for span in &block.spans {
                let escaped = glib::markup_escape_text(&span.text);
                if escaped.is_empty() {
                    continue;
                }
                match span.kind.as_str() {
                    "bold" => {
                        inner.push_str("<b>");
                        inner.push_str(&escaped);
                        inner.push_str("</b>");
                    }
                    "italic" => {
                        inner.push_str("<i>");
                        inner.push_str(&escaped);
                        inner.push_str("</i>");
                    }
                    "highlight" => {
                        inner.push_str("<span background=\"#3584e438\" weight=\"semibold\">");
                        inner.push_str(&escaped);
                        inner.push_str("</span>");
                    }
                    "code" => {
                        inner.push_str("<tt>");
                        inner.push_str(&escaped);
                        inner.push_str("</tt>");
                    }
                    "link" => {
                        if let Some(url) = &span.url {
                            let escaped_url = glib::markup_escape_text(url);
                            inner.push_str(" <a href=\"");
                            inner.push_str(&escaped_url);
                            inner.push_str("\"><span background=\"#3584e424\" size=\"small\"> 🔗 ");
                            inner.push_str(&escaped);
                            inner.push_str(" </span></a>");
                        } else {
                            inner.push_str(&escaped);
                        }
                    }
                    _ => inner.push_str(&escaped),
                }
            }

            let trimmed = inner.trim();
            if trimmed.is_empty() {
                continue;
            }

            if !out.is_empty() {
                if prev_was_bullet && block.kind == "bullet" {
                    out.push('\n');
                } else {
                    out.push_str("\n\n");
                }
            }

            match block.kind.as_str() {
                "heading" => {
                    out.push_str("<b><span size=\"large\">");
                    out.push_str(trimmed);
                    out.push_str("</span></b>");
                    prev_was_bullet = false;
                }
                "bullet" => {
                    out.push_str("  <span alpha=\"65%\">•</span>  ");
                    out.push_str(trimmed);
                    prev_was_bullet = true;
                }
                "code" => {
                    out.push_str("<tt>");
                    out.push_str(trimmed);
                    out.push_str("</tt>");
                    prev_was_bullet = false;
                }
                _ => {
                    out.push_str(trimmed);
                    prev_was_bullet = false;
                }
            }
        }

        out
    }

    pub fn to_plain_text(&self) -> String {
        let mut out = String::new();
        let mut prev_was_bullet = false;

        for block in &self.blocks {
            let mut line = String::new();
            for span in &block.spans {
                if span.kind == "link" {
                    if let Some(url) = &span.url {
                        line.push_str(&format!(" [{}]({})", span.text, url));
                    } else {
                        line.push_str(&span.text);
                    }
                } else {
                    line.push_str(&span.text);
                }
            }
            let text = line.trim();
            if text.is_empty() {
                continue;
            }
            if !out.is_empty() {
                if prev_was_bullet && block.kind == "bullet" {
                    out.push('\n');
                } else {
                    out.push_str("\n\n");
                }
            }
            if block.kind == "bullet" {
                out.push_str("• ");
                prev_was_bullet = true;
            } else {
                prev_was_bullet = false;
            }
            out.push_str(text);
        }

        out
    }
}

/// Builds a resilient, semantic-only JS extractor for Google AI Mode (`udm=50`)
/// preserving bold/italic/highlight/code spans and inline clickable source links.
pub fn ai_mode_extract_js(queries: &[String]) -> String {
    let latest_query = queries.last().map(|s| s.trim()).unwrap_or("");
    let query_json = serde_json::to_string(latest_query).unwrap_or_else(|_| "\"\"".to_string());
    let is_multi_turn = if queries.len() > 1 { "true" } else { "false" };
    format!(
        r#"(function() {{
  const latestQuery = {query_json}.toLowerCase();
  const isMultiTurn = {is_multi_turn};
  const root = document.querySelector('[role="main"]') || document.querySelector('main') || document.body;
  if (!root) return JSON.stringify({{ blocks: [], raw_len: 0 }});

  const clone = root.cloneNode(true);
  const stripSel = [
    'script', 'style', 'noscript', 'svg', 'img', 'video', 'audio', 'iframe',
    'form', 'textarea', 'input', 'button', 'nav', 'header', 'footer', 'aside',
    '[role="navigation"]', '[role="search"]', '[role="banner"]',
    '[role="contentinfo"]', '[role="dialog"]', '[role="menu"]', '[aria-hidden="true"]',
    '[style*="display: none"]', '[style*="display:none"]',
    '[style*="visibility: hidden"]', '[hidden]',
    '[data-ignore-copy]', '[role="status"]', '[role="alert"]', '[role="progressbar"]'
  ].join(',');
  clone.querySelectorAll(stripSel).forEach(el => el.remove());

  const uiNoise = new Set([
    'mode ia', 'ai mode', 'posez une question', 'ask anything',
    'tous', 'all', 'images', 'vidéos', 'videos', 'actualités', 'news', 'plus', 'more',
    'se connecter', 'sign in', 'j\'accepte', 'tout accepter', 'tout refuser',
    'partagé', 'shared',
    'voir l\'historique de la recherche', 'personnalisation',
    'applis connectées', 'gérer le filigrane des contenus multimédias',
    'search history', 'personalization', 'connected apps',
    'transcription...', 'transcription…', 'transcribing...', 'transcribing…',
    'réflexion...', 'réflexion…', 'thinking...', 'thinking…',
    'recherche en cours...', 'recherche en cours…', 'searching...', 'searching…'
  ]);

  const inlineTags = new Set([
    'SPAN', 'STRONG', 'B', 'EM', 'I', 'A', 'CODE', 'MARK', 'SUB', 'SUP', 'BR', 'SMALL', 'U'
  ]);

  function cleanText(raw) {{
    return (raw || '')
      .replace(/\[(?:Début|Fin) de la mise en surbrillance\]/gi, '')
      .replace(/\[(?:Highlight start|Highlight end)\]/gi, '')
      .replace(/\s+/g, ' ')
      .replace(/\s+([.,;:!?])/g, '$1');
  }}

  function isEndMarker(lower) {{
    return (
      lower === 'partagé' ||
      lower === 'shared' ||
      lower.startsWith('les réponses de l\'ia peuvent') ||
      lower.startsWith('vérifiez les réponses de l\'ia') ||
      lower.startsWith('ai responses may include') ||
      lower.startsWith('check important info')
    );
  }}

  function isNoiseText(lower) {{
    if (uiNoise.has(lower)) return true;
    if (/^\d+\s+(fichiers?|files?)$/i.test(lower)) return true;
    if (/^[+\d\s|·•]+$/.test(lower)) return true;
    if (lower.startsWith('réponse du mode ia')) return true;
    if (lower.startsWith('ai mode response')) return true;
    if (lower.startsWith('conversation en mode ia')) return true;
    if (lower.startsWith('ai mode conversation')) return true;
    if (lower.startsWith('copié dans le presse-papiers')) return true;
    if (lower.startsWith('une copie de cette discussion')) return true;
    if (lower.startsWith('merci de nous avoir informés')) return true;
    if (lower.startsWith('google peut utiliser les données')) return true;
    if (lower.startsWith('google may use')) return true;
    return false;
  }}

  function isLatestQueryMatch(lower) {{
    if (!latestQuery) return false;
    if (lower === latestQuery) return true;
    if (latestQuery.length >= 6 && lower.endsWith(latestQuery) && lower.length <= latestQuery.length + 35) {{
      return true;
    }}
    return false;
  }}

  function hasOnlyInlineChildren(el) {{
    if (el.querySelector('span[role="link"]')) return false;
    for (const child of el.children) {{
      if (!inlineTags.has(child.tagName)) return false;
    }}
    return true;
  }}

  function extractSpans(node, activeKind) {{
    const spans = [];
    const startRe = /\[(?:Début de la mise en surbrillance|Highlight start)\]/gi;
    const endRe = /\[(?:Fin de la mise en surbrillance|Highlight end)\]/gi;

    function visit(n, kind) {{
      if (!n) return;
      if (n.nodeType === 3) {{
        const val = n.nodeValue || '';
        if (!val) return;
        spans.push({{ kind: kind || 'text', text: val, url: null }});
        return;
      }}
      if (n.nodeType !== 1) return;
      const tag = n.tagName;
      if (tag === 'BR') {{
        spans.push({{ kind: 'text', text: '\n', url: null }});
        return;
      }}
      if (tag === 'A' && n.getAttribute('href')) {{
        const href = n.href || '';
        try {{
          const u = new URL(href);
          const host = u.hostname.replace(/^www\./, '');
          if (!host.endsWith('google.com') && !host.endsWith('gstatic.com') && !host.endsWith('googleapis.com')) {{
            let label = cleanText(n.textContent).trim() || cleanText(n.getAttribute('aria-label') || '').trim();
            if (!label || label.length < 2 || /^https?:\/\//i.test(label)) {{
              label = host;
            }}
            label = label.slice(0, 42);
            spans.push({{ kind: 'link', text: label, url: href }});
            return;
          }}
        }} catch (_) {{}}
      }}

      let nextKind = kind;
      if (tag === 'MARK') nextKind = 'highlight';
      else if (tag === 'STRONG' || tag === 'B') nextKind = 'bold';
      else if (tag === 'EM' || tag === 'I') nextKind = 'italic';
      else if (tag === 'CODE') nextKind = 'code';

      for (const child of n.childNodes) {{
        visit(child, nextKind);
      }}
    }}

    visit(node, activeKind || 'text');

    // Convert [Début de la mise en surbrillance] ... [Fin de la mise en surbrillance] into highlight spans
    const processed = [];
    let inHighlight = false;
    const tokenRe = /(\[(?:Début|Fin) de la mise en surbrillance\]|\[(?:Highlight start|Highlight end)\])/gi;

    for (const sp of spans) {{
      if (sp.kind === 'link') {{
        processed.push(sp);
        continue;
      }}
      const parts = sp.text.split(tokenRe);
      for (const part of parts) {{
        if (!part) continue;
        if (startRe.test(part)) {{
          startRe.lastIndex = 0;
          inHighlight = true;
          continue;
        }}
        if (endRe.test(part)) {{
          endRe.lastIndex = 0;
          inHighlight = false;
          continue;
        }}
        const cleaned = part.replace(/\s+/g, ' ');
        if (!cleaned) continue;
        processed.push({{
          kind: inHighlight ? 'highlight' : sp.kind,
          text: cleaned,
          url: null
        }});
      }}
    }}

    // Merge adjacent identical non-link spans and tidy punctuation spacing
    const merged = [];
    for (const sp of processed) {{
      if (merged.length > 0 && sp.kind !== 'link') {{
        const prev = merged[merged.length - 1];
        if (prev.kind === sp.kind && !prev.url && !sp.url) {{
          prev.text = (prev.text + sp.text).replace(/\s+([.,;:!?])/g, '$1');
          continue;
        }}
      }}
      if (sp.kind !== 'link') {{
        sp.text = sp.text.replace(/\s+([.,;:!?])/g, '$1');
      }}
      merged.push(sp);
    }}

    if (merged.length > 0 && merged[0].kind !== 'link') {{
      merged[0].text = merged[0].text.replace(/^\s+/, '');
    }}
    if (merged.length > 0 && merged[merged.length - 1].kind !== 'link') {{
      merged[merged.length - 1].text = merged[merged.length - 1].text.replace(/\s+$/, '');
    }}

    return merged.filter(s => s.text.length > 0);
  }}

  const allBlocks = [];
  const seenTexts = new Set();

  function pushBlock(kind, node) {{
    const spans = extractSpans(node, 'text');
    const text = cleanText(spans.map(s => s.text).join('')).trim();
    if (text.length < 3) return;
    const lower = text.toLowerCase();
    if (isLatestQueryMatch(lower)) {{
      allBlocks.push({{ kind: '__QUERY__', spans: [] }});
      return;
    }}
    if (isEndMarker(lower)) {{
      allBlocks.push({{ kind: '__END__', spans: [] }});
      return;
    }}
    if (isNoiseText(lower)) return;
    if (seenTexts.has(text)) return;
    seenTexts.add(text);
    allBlocks.push({{ kind, spans }});
  }}

  function walk(node) {{
    if (!node || node.nodeType !== 1) return;
    const tag = node.tagName;

    if (tag === 'SPAN' && node.getAttribute('role') === 'link') {{
      const sText = cleanText(node.textContent).trim();
      if (sText.length >= 4) {{
        allBlocks.push({{ kind: '__SUGGESTION__', spans: [{{ kind: 'text', text: sText, url: null }}] }});
      }}
      return;
    }}
    if (tag === 'H1' || tag === 'H2' || tag === 'H3' || tag === 'H4' || node.getAttribute('role') === 'heading') {{
      pushBlock('heading', node);
      return;
    }}
    if (tag === 'LI') {{
      pushBlock('bullet', node);
      return;
    }}
    if (tag === 'PRE') {{
      pushBlock('code', node);
      return;
    }}
    if (tag === 'P') {{
      pushBlock('paragraph', node);
      return;
    }}
    if (tag === 'DIV' && hasOnlyInlineChildren(node)) {{
      pushBlock('paragraph', node);
      return;
    }}

    for (const child of node.children) {{
      walk(child);
    }}
  }}

  walk(clone);

  let startIdx = 0;
  let foundQueryMarker = false;
  if (latestQuery) {{
    for (let i = allBlocks.length - 1; i >= 0; i--) {{
      if (allBlocks[i].kind === '__QUERY__') {{
        startIdx = i + 1;
        foundQueryMarker = true;
        break;
      }}
    }}
  }}

  if (isMultiTurn && !foundQueryMarker) {{
    return JSON.stringify({{ blocks: [], suggestions: [], raw_len: 0 }});
  }}

  const blocks = [];
  const suggestions = [];
  const seenSuggestions = new Set();
  let hitEnd = false;
  for (let i = startIdx; i < allBlocks.length; i++) {{
    const b = allBlocks[i];
    if (b.kind === '__QUERY__') continue;
    if (b.kind === '__SUGGESTION__') {{
      const sText = (b.spans[0]?.text || '').trim();
      if (sText.length >= 4 && !seenSuggestions.has(sText) && suggestions.length < 4) {{
        seenSuggestions.add(sText);
        suggestions.push(sText);
      }}
      continue;
    }}
    if (b.kind === '__END__') {{
      if (blocks.length > 0) hitEnd = true;
      continue;
    }}
    if (!hitEnd) {{
      blocks.push(b);
    }}
  }}

  const raw_len =
    blocks.reduce((acc, b) => acc + b.spans.reduce((s, sp) => s + sp.text.length, 0), 0) +
    suggestions.reduce((acc, s) => acc + s.length, 0);
  return JSON.stringify({{ blocks, suggestions, raw_len }});
}})();"#
    )
}

/// Generates a JS snippet that types a follow-up question into Google AI Mode's
/// bottom input box and submits it.
pub fn ai_mode_followup_js(query: &str) -> String {
    let query_json = serde_json::to_string(query.trim()).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        r#"(function() {{
  const followText = {query_json};
  if (!followText) return;
  const dbg = (window.__aiOverviewDebug = window.__aiOverviewDebug || []);

  function isVisible(el) {{
    if (!el) return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  }}

  function findInput() {{
    const candidates = Array.from(
      document.querySelectorAll('textarea, [contenteditable="true"], input[type="text"]')
    ).filter(isVisible);

    const preferred = candidates.find(el => {{
      const hint = (
        (el.getAttribute('placeholder') || '') + ' ' +
        (el.getAttribute('aria-label') || '')
      ).toLowerCase();
      return (
        hint.includes('question') ||
        hint.includes('ask') ||
        hint.includes('follow') ||
        hint.includes('demander')
      );
    }});

    return preferred || candidates[candidates.length - 1] || null;
  }}

  function findSubmitButton(input) {{
    const form = input.closest('form');
    const scope = form || input.parentElement?.parentElement || document;
    const btns = Array.from(scope.querySelectorAll('button')).filter(isVisible);
    return btns.find(b => {{
      if (b.disabled || b.getAttribute('aria-disabled') === 'true') return false;
      const type = (b.getAttribute('type') || '').toLowerCase();
      const label = (b.getAttribute('aria-label') || '').toLowerCase();
      return (
        type === 'submit' ||
        label.includes('envoyer') ||
        label.includes('send') ||
        label.includes('rechercher') ||
        label.includes('search')
      );
    }}) || null;
  }}

  const input = findInput();
  if (!input) {{
    dbg.push('followup_input_not_found');
    return;
  }}

  input.focus();
  if (input.tagName === 'TEXTAREA' || input.tagName === 'INPUT') {{
    const proto = input.tagName === 'TEXTAREA'
      ? window.HTMLTextAreaElement.prototype
      : window.HTMLInputElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set;
    if (setter) {{
      setter.call(input, followText);
    }} else {{
      input.value = followText;
    }}
  }} else {{
    input.textContent = followText;
  }}

  input.dispatchEvent(new Event('input', {{ bubbles: true }}));
  input.dispatchEvent(new Event('change', {{ bubbles: true }}));
  dbg.push('followup_filled:' + input.tagName);

  setTimeout(() => {{
    const btn = findSubmitButton(input);
    if (btn) {{
      btn.click();
      dbg.push('followup_btn_clicked');
    }} else {{
      const opts = {{ key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true, cancelable: true }};
      input.dispatchEvent(new KeyboardEvent('keydown', opts));
      input.dispatchEvent(new KeyboardEvent('keypress', opts));
      input.dispatchEvent(new KeyboardEvent('keyup', opts));
      dbg.push('followup_enter_dispatched');
    }}
  }}, 150);
}})();"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rich_spans_and_escapes_pango() {
        let json = r#"{
            "blocks": [
                {
                    "kind": "paragraph",
                    "spans": [
                        {"kind": "text", "text": "La capitale est ", "url": null},
                        {"kind": "highlight", "text": "Paris", "url": null},
                        {"kind": "text", "text": ".", "url": null},
                        {"kind": "link", "text": "fr.wikipedia.org", "url": "https://fr.wikipedia.org/wiki/Paris?a=1&b=2"}
                    ]
                },
                {
                    "kind": "bullet",
                    "spans": [
                        {"kind": "bold", "text": "Architecture & Fluidité:", "url": null},
                        {"kind": "text", "text": " Wayland < X11.", "url": null}
                    ]
                },
                {
                    "kind": "bullet",
                    "spans": [
                        {"kind": "text", "text": "Isolation native.", "url": null}
                    ]
                }
            ],
            "suggestions": ["L'histoire des capitales de France"],
            "raw_len": 110
        }"#;

        let ans = ExtractedAnswer::from_json(json).expect("valid answer");
        assert_eq!(ans.suggestions, vec!["L'histoire des capitales de France"]);
        let markup = ans.to_pango_markup();
        assert!(markup.contains("<span background=\"#3584e438\" weight=\"semibold\">Paris</span>"));
        assert!(markup.contains("<a href=\"https://fr.wikipedia.org/wiki/Paris?a=1&amp;b=2\"><span background=\"#3584e424\" size=\"small\"> 🔗 fr.wikipedia.org </span></a>"));
        assert!(markup.contains("•</span>  <b>Architecture &amp; Fluidité:</b> Wayland &lt; X11.\n  <span alpha=\"65%\">•</span>  Isolation native."));

        let plain = ans.to_plain_text();
        assert!(plain.contains("La capitale est Paris. [fr.wikipedia.org](https://fr.wikipedia.org/wiki/Paris?a=1&b=2)"));
    }

    #[test]
    fn rejects_empty_or_tiny_answers() {
        let tiny = r#"{"blocks":[{"kind":"paragraph","spans":[{"kind":"text","text":"Hi","url":null}]}],"raw_len":2}"#;
        assert!(ExtractedAnswer::from_json(tiny).is_none());
    }

    #[test]
    fn followup_js_escapes_text() {
        let js = ai_mode_followup_js("Et sa \"population\" ?");
        assert!(js.contains(r#"Et sa \"population\" ?"#));
    }
}
