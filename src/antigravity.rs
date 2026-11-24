use std::io::{BufRead, BufReader, Read};
use std::process::Stdio;
use std::sync::{OnceLock, mpsc};
use std::thread;

use gtk4::glib;
use serde_json::Value;

use crate::i18n::tr;

static AGY_AVAILABLE: OnceLock<bool> = OnceLock::new();

/// Returns `true` if the Antigravity CLI (`agy`) is available on the host.
pub fn is_available() -> bool {
    *AGY_AVAILABLE.get_or_init(|| {
        let found = if crate::config::is_flatpak() {
            crate::config::host_command("sh")
                .args([
                    "-lc",
                    "command -v agy >/dev/null 2>&1 || [ -x \"$HOME/.local/bin/agy\" ]",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        } else {
            let on_path = std::env::var_os("PATH").is_some_and(|path_var| {
                std::env::split_paths(&path_var).any(|dir| dir.join("agy").is_file())
            });
            let in_local_bin = dirs::home_dir()
                .is_some_and(|home| home.join(".local/bin/agy").is_file());
            on_path || in_local_bin
        };
        log_info!(
            "antigravity",
            "CLI detection: available={found} (flatpak={})",
            crate::config::is_flatpak()
        );
        found
    })
}

const DESKTOP_GUARDRAILS_PROMPT: &str = "\
Tu es l'assistant bureau GNOME de l'utilisateur dans un lanceur Spotlight.
Règles :
- Réponds en français de façon très concise et directe.
- Exécute directement les petites actions réversibles sur le PC (volume via wpctl, luminosité, ouvrir une app ou URL via xdg-open/gio, lire un fichier, minuteur, presse-papiers, statut système, etc.).
- INTERDICTION absolue de supprimer des fichiers, modifier des fichiers système, ou exécuter sudo/su : refuse poliment et indique simplement la commande à lancer manuellement si l'utilisateur le demande.

Demande : ";

#[derive(Debug, Clone, PartialEq)]
pub struct AgyToolStep {
    pub step_index: u64,
    pub tool_name: String,
    pub state: String,
    pub summary: String,
    pub detail: String,
    pub duration_seconds: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgyEvent {
    Init(String),
    TextDelta(String),
    ToolStep(AgyToolStep),
    Done {
        response: String,
        conversation_id: Option<String>,
    },
    Error(String),
}

pub fn parse_stream_line(line: &str) -> Option<AgyEvent> {
    let trimmed = line.trim();
    if !trimmed.starts_with('{') {
        return None;
    }
    let val: Value = serde_json::from_str(trimmed).ok()?;
    let event = val.get("event")?.as_str()?;

    match event {
        "init" => {
            let id = val
                .get("conversation_id")
                .and_then(|v| v.as_str())
                .map(str::to_owned)?;
            Some(AgyEvent::Init(id))
        }
        "step_update" => {
            let su = val.get("step_update")?;
            if let Some(delta) = su.get("text_delta").and_then(|v| v.as_str())
                && !delta.is_empty()
            {
                return Some(AgyEvent::TextDelta(delta.to_string()));
            }
            if let Some(step_type) = su.get("step_type").and_then(|v| v.as_str())
                && step_type != "user_input"
                && step_type != "agent_response"
            {
                let step_index = su.get("step_index").and_then(|v| v.as_u64()).unwrap_or(0);
                let state = su
                    .get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or("ACTIVE")
                    .to_string();
                let duration_seconds = su.get("duration_seconds").and_then(|v| v.as_f64());
                let tool_info = su.get("tool_info");
                let tool_name = su
                    .get("tool_name")
                    .and_then(|v| v.as_str())
                    .or_else(|| tool_info.and_then(|ti| ti.get("name")).and_then(|v| v.as_str()))
                    .unwrap_or(step_type)
                    .to_string();
                let summary = format_tool_summary(&tool_name, tool_info);
                let detail = format_tool_detail(&tool_name, tool_info);
                return Some(AgyEvent::ToolStep(AgyToolStep {
                    step_index,
                    tool_name,
                    state,
                    summary,
                    detail,
                    duration_seconds,
                }));
            }
            None
        }
        "result" => {
            let res = val.get("result")?;
            let status = res.get("status").and_then(|v| v.as_str()).unwrap_or("");
            if status == "ERROR" {
                let err = res
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Erreur Antigravity")
                    .to_string();
                Some(AgyEvent::Error(err))
            } else {
                let response = res
                    .get("response")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let conversation_id = res
                    .get("conversation_id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                Some(AgyEvent::Done {
                    response,
                    conversation_id,
                })
            }
        }
        _ => None,
    }
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

fn basename_or_path(path: &str) -> &str {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or(path)
}

fn format_tool_summary(tool_name: &str, tool_info: Option<&Value>) -> String {
    let params = tool_info.and_then(|ti| ti.get("parameters"));
    match tool_name {
        "run_command" => {
            if let Some(cmd) = params
                .and_then(|p| p.get("CommandLine").or_else(|| p.get("command")))
                .and_then(|v| v.as_str())
            {
                let first_line = cmd.lines().next().unwrap_or(cmd).trim();
                return format!("{} : {}", tr("Run", "Exécuter"), truncate_chars(first_line, 60));
            }
            tr("Run command", "Exécuter une commande").to_string()
        }
        "view_file" | "read_file" => {
            if let Some(path) = params
                .and_then(|p| p.get("AbsolutePath").or_else(|| p.get("Path")))
                .and_then(|v| v.as_str())
            {
                return format!("{} : {}", tr("Read", "Lire"), basename_or_path(path));
            }
            tr("Read file", "Lire un fichier").to_string()
        }
        "write_to_file"
        | "replace_file_content"
        | "multi_replace_file_content"
        | "sed_file" => {
            if let Some(path) = params
                .and_then(|p| p.get("TargetFile").or_else(|| p.get("Path")))
                .and_then(|v| v.as_str())
            {
                return format!("{} : {}", tr("Edit", "Modifier"), basename_or_path(path));
            }
            tr("Edit file", "Modifier un fichier").to_string()
        }
        "list_dir" => {
            if let Some(path) = params
                .and_then(|p| p.get("DirectoryPath"))
                .and_then(|v| v.as_str())
            {
                return format!("{} : {}", tr("List", "Lister"), basename_or_path(path));
            }
            tr("List directory", "Lister un dossier").to_string()
        }
        "grep_search" | "find_by_name" => {
            if let Some(q) = params
                .and_then(|p| p.get("Query").or_else(|| p.get("Pattern")))
                .and_then(|v| v.as_str())
            {
                return format!("{} : {}", tr("Search", "Rechercher"), truncate_chars(q, 50));
            }
            tr("Search", "Rechercher").to_string()
        }
        "search_web" => {
            if let Some(q) = params.and_then(|p| p.get("query")).and_then(|v| v.as_str()) {
                return format!("{} : {}", tr("Web search", "Recherche web"), truncate_chars(q, 50));
            }
            tr("Web search", "Recherche web").to_string()
        }
        "open_browser_url" | "read_url_content" => {
            if let Some(url) = params.and_then(|p| p.get("Url")).and_then(|v| v.as_str()) {
                return format!("Web : {}", truncate_chars(url, 55));
            }
            tr("Open URL", "Ouvrir une URL").to_string()
        }
        _ => tool_name.to_string(),
    }
}

fn clean_tool_output(raw: &str) -> String {
    let mut lines = raw.lines().peekable();
    while let Some(line) = lines.peek() {
        let t = line.trim();
        if t.starts_with("Created At:") || t.starts_with("Completed At:") {
            lines.next();
        } else {
            break;
        }
    }
    let cleaned: Vec<&str> = lines.collect();
    cleaned.join("\n").trim().to_string()
}

fn format_tool_detail(tool_name: &str, tool_info: Option<&Value>) -> String {
    let mut parts = Vec::new();
    let params = tool_info.and_then(|ti| ti.get("parameters"));

    if let Some(p) = params {
        if tool_name == "run_command"
            && let Some(cmd) = p
                .get("CommandLine")
                .or_else(|| p.get("command"))
                .and_then(|v| v.as_str())
        {
            parts.push(format!("$ {cmd}"));
            if let Some(obj) = p.as_object() {
                let extras: serde_json::Map<String, Value> = obj
                    .iter()
                    .filter(|(k, _)| k.as_str() != "CommandLine" && k.as_str() != "command")
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                if !extras.is_empty()
                    && let Ok(pretty) = serde_json::to_string_pretty(&Value::Object(extras))
                {
                    parts.push(pretty);
                }
            }
        } else if let Ok(pretty) = serde_json::to_string_pretty(p)
            && pretty != "null"
            && pretty != "{}"
        {
            parts.push(pretty);
        }
    }

    if let Some(raw_output) = tool_info
        .and_then(|ti| ti.get("output"))
        .and_then(|v| v.as_str())
    {
        let cleaned = clean_tool_output(raw_output);
        if !cleaned.is_empty() {
            let truncated = if cleaned.chars().count() > 2500 {
                let prefix: String = cleaned.chars().take(2500).collect();
                format!("{prefix}\n{}", tr("… [output truncated]", "… [sortie tronquée]"))
            } else {
                cleaned
            };
            parts.push(format!("{}\n{truncated}", tr("Output :", "Sortie :")));
        }
    }

    let joined = parts.join("\n\n");
    if joined.chars().count() > 4000 {
        let prefix: String = joined.chars().take(4000).collect();
        format!("{prefix}\n… [détail tronqué]")
    } else {
        joined
    }
}

pub fn spawn_turn(
    prompt: &str,
    conversation_id: Option<String>,
    auto_approve: bool,
    model: Option<String>,
) -> mpsc::Receiver<AgyEvent> {
    let (tx, rx) = mpsc::channel();
    let full_prompt = if conversation_id.is_none() {
        format!("{DESKTOP_GUARDRAILS_PROMPT}{}", prompt.trim())
    } else {
        prompt.trim().to_string()
    };

    thread::spawn(move || {
        log_info!(
            "antigravity",
            "Spawning agy turn (model={:?}, auto_approve={}, resume={:?}, flatpak={})",
            model,
            auto_approve,
            conversation_id,
            crate::config::is_flatpak()
        );
        let mut cmd = crate::config::host_command("agy");
        if !crate::config::is_flatpak()
            && let Some(home) = dirs::home_dir()
        {
            cmd.current_dir(home);
        }
        cmd.args(["--output-format", "stream-json"]);
        if auto_approve {
            cmd.arg("--dangerously-skip-permissions");
        }
        if let Some(m) = model.as_deref().filter(|s| !s.trim().is_empty()) {
            cmd.args(["--model", m]);
        }
        if let Some(id) = conversation_id.as_deref().filter(|s| !s.trim().is_empty()) {
            cmd.args(["--resume", id]);
        }
        cmd.arg(format!("-p={full_prompt}"));
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                log_err!("antigravity", "Failed to spawn agy: {e}");
                let _ = tx.send(AgyEvent::Error(format!("Impossible de lancer agy : {e}")));
                return;
            }
        };

        let mut finished = false;
        if let Some(stdout) = child.stdout.take() {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if let Some(ev) = parse_stream_line(&line) {
                    if matches!(ev, AgyEvent::Done { .. } | AgyEvent::Error(_)) {
                        finished = true;
                    }
                    if tx.send(ev).is_err() {
                        let _ = child.kill();
                        let _ = child.wait();
                        return;
                    }
                }
            }
        }

        let stderr_str = child
            .stderr
            .take()
            .and_then(|mut err| {
                let mut s = String::new();
                err.read_to_string(&mut s).ok().map(|_| s)
            })
            .unwrap_or_default();

        let status = child.wait();
        if !finished {
            if !stderr_str.trim().is_empty() {
                log_err!("antigravity", "agy stderr: {}", stderr_str.trim());
            }
            let msg = match status {
                Ok(s) if s.success() => "Réponse vide de agy".to_string(),
                Ok(s) => format!("agy terminé avec le code {s}"),
                Err(e) => format!("Erreur d'attente agy : {e}"),
            };
            log_err!("antigravity", "agy ended unexpectedly: {msg}");
            let _ = tx.send(AgyEvent::Error(msg));
        }
    });

    rx
}

pub fn markdown_to_pango(md: &str) -> String {
    let mut out = String::new();
    let mut in_code_block = false;

    for raw_line in md.lines() {
        let trimmed = raw_line.trim_end();
        if trimmed.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        if !out.is_empty() {
            out.push('\n');
        }

        if in_code_block {
            out.push_str("<tt>");
            out.push_str(&glib::markup_escape_text(trimmed));
            out.push_str("</tt>");
            continue;
        }

        let stripped = trimmed.trim_start();
        if let Some(heading) = stripped
            .strip_prefix("#### ")
            .or_else(|| stripped.strip_prefix("### "))
            .or_else(|| stripped.strip_prefix("## "))
            .or_else(|| stripped.strip_prefix("# "))
        {
            out.push_str("<b><span size=\"large\">");
            out.push_str(&inline_md_to_pango(heading));
            out.push_str("</span></b>");
        } else if let Some(bullet) = stripped
            .strip_prefix("- ")
            .or_else(|| stripped.strip_prefix("* "))
        {
            out.push_str("  <span alpha=\"65%\">•</span>  ");
            out.push_str(&inline_md_to_pango(bullet));
        } else {
            out.push_str(&inline_md_to_pango(trimmed));
        }
    }

    out
}

fn inline_md_to_pango(input: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // Inline `code`
        if chars[i] == '`'
            && let Some(end) = find_closing(&chars, i + 1, '`')
        {
            let slice: String = chars[i + 1..end].iter().collect();
            out.push_str("<tt>");
            out.push_str(&glib::markup_escape_text(&slice));
            out.push_str("</tt>");
            i = end + 1;
            continue;
        }

        // Bold **text**
        if i + 1 < chars.len()
            && chars[i] == '*'
            && chars[i + 1] == '*'
            && let Some(end) = find_double_closing(&chars, i + 2, '*')
        {
            let slice: String = chars[i + 2..end].iter().collect();
            out.push_str("<b>");
            out.push_str(&glib::markup_escape_text(&slice));
            out.push_str("</b>");
            i = end + 2;
            continue;
        }

        // Link [label](url)
        if chars[i] == '['
            && let Some(close_bracket) = find_closing(&chars, i + 1, ']')
            && close_bracket + 1 < chars.len()
            && chars[close_bracket + 1] == '('
            && let Some(close_paren) = find_closing(&chars, close_bracket + 2, ')')
        {
            let label: String = chars[i + 1..close_bracket].iter().collect();
            let url: String = chars[close_bracket + 2..close_paren].iter().collect();
            if url.starts_with("http://") || url.starts_with("https://") {
                out.push_str("<a href=\"");
                out.push_str(&glib::markup_escape_text(&url));
                out.push_str("\">🔗 ");
                out.push_str(&glib::markup_escape_text(&label));
                out.push_str("</a>");
                i = close_paren + 1;
                continue;
            }
        }

        // Italic *text*
        if chars[i] == '*'
            && let Some(end) = find_closing(&chars, i + 1, '*')
            && end > i + 1
        {
            let slice: String = chars[i + 1..end].iter().collect();
            out.push_str("<i>");
            out.push_str(&glib::markup_escape_text(&slice));
            out.push_str("</i>");
            i = end + 1;
            continue;
        }

        let ch_str: String = [chars[i]].iter().collect();
        out.push_str(&glib::markup_escape_text(&ch_str));
        i += 1;
    }

    out
}

fn find_closing(chars: &[char], start: usize, target: char) -> Option<usize> {
    (start..chars.len()).find(|&idx| chars[idx] == target)
}

fn find_double_closing(chars: &[char], start: usize, target: char) -> Option<usize> {
    (start..chars.len().saturating_sub(1))
        .find(|&idx| chars[idx] == target && chars[idx + 1] == target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_agy_stream_events() {
        let init = r#"{"event":"init","conversation_id":"abc-123"}"#;
        assert_eq!(
            parse_stream_line(init),
            Some(AgyEvent::Init("abc-123".to_string()))
        );

        let delta = r#"{"event":"step_update","step_update":{"step_type":"agent_response","text_delta":"Salut"}}"#;
        assert_eq!(
            parse_stream_line(delta),
            Some(AgyEvent::TextDelta("Salut".to_string()))
        );

        let result = r#"{"event":"result","result":{"conversation_id":"abc-123","status":"SUCCESS","response":"OK"}}"#;
        assert_eq!(
            parse_stream_line(result),
            Some(AgyEvent::Done {
                response: "OK".to_string(),
                conversation_id: Some("abc-123".to_string()),
            })
        );
    }

    #[test]
    fn parses_agy_tool_steps() {
        let active = r#"{"event":"step_update","step_update":{"step_index":2,"state":"ACTIVE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"echo hello_stream_test"}}}}"#;
        match parse_stream_line(active) {
            Some(AgyEvent::ToolStep(step)) => {
                assert_eq!(step.step_index, 2);
                assert_eq!(step.tool_name, "run_command");
                assert_eq!(step.state, "ACTIVE");
                assert!(step.summary.ends_with(": echo hello_stream_test"));
                assert_eq!(step.detail, "$ echo hello_stream_test");
                assert_eq!(step.duration_seconds, None);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let done = r#"{"event":"step_update","step_update":{"step_index":2,"state":"DONE","step_type":"tool","tool_name":"run_command","duration_seconds":0.025,"tool_info":{"name":"run_command","parameters":{"CommandLine":"echo hello_stream_test"},"output":"Created At: 2026-11-24T16:31:12+01:00\nCompleted At: 2026-11-24T16:31:12+01:00\nCommand exited with code 0.\n\nCommand output:\nhello_stream_test"}}}"#;
        match parse_stream_line(done) {
            Some(AgyEvent::ToolStep(step)) => {
                assert_eq!(step.step_index, 2);
                assert_eq!(step.state, "DONE");
                assert_eq!(step.duration_seconds, Some(0.025));
                assert!(step.detail.starts_with("$ echo hello_stream_test\n\n"));
                assert!(step.detail.contains("hello_stream_test"));
                assert!(!step.detail.contains("Created At:"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn converts_markdown_to_safe_pango() {
        let md = "# Titre & Info\n- **Volume** à `50%`\nVoir [Site](https://example.com/?a=1&b=2)";
        let pango = markdown_to_pango(md);
        assert!(pango.contains("<b><span size=\"large\">Titre &amp; Info</span></b>"));
        assert!(pango.contains("<b>Volume</b> à <tt>50%</tt>"));
        assert!(pango.contains("<a href=\"https://example.com/?a=1&amp;b=2\">🔗 Site</a>"));
    }
}
