#[macro_use]
mod logging;

mod antigravity;
mod config;
mod extractor;
mod i18n;
mod preferences;
mod provider;
mod shortcut;
mod voice;
mod webview;
mod window;

use std::cell::RefCell;
use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::gdk;
use gtk4::gio;
use gtk4::glib;
use libadwaita as adw;

use crate::config::{APP_ID, Config};
use crate::provider::Provider;
use crate::window::SpotlightController;

fn main() -> glib::ExitCode {
    logging::init();
    log_info!(
        "main",
        "Starting {} v{} (flatpak={}, log={})",
        APP_ID,
        env!("CARGO_PKG_VERSION"),
        config::is_flatpak(),
        logging::log_path().display()
    );

    let args = prepare_args_with_stdin();
    let is_screenshot = args
        .iter()
        .any(|a| a == "--screenshot" || a.starts_with("--screenshot="));

    let mut flags = gio::ApplicationFlags::HANDLES_COMMAND_LINE;
    if is_screenshot {
        flags |= gio::ApplicationFlags::NON_UNIQUE;
    }

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(flags)
        .build();

    register_cli_options(&app);

    app.connect_startup(|_| {
        load_css();
    });

    let controller_slot: Rc<RefCell<Option<SpotlightController>>> = Rc::new(RefCell::new(None));

    app.connect_command_line(move |app, cmd| {
        let mut slot = controller_slot.borrow_mut();
        let controller = slot
            .get_or_insert_with(|| SpotlightController::new(app, Config::load()))
            .clone();

        let opts = cmd.options_dict();

        if opts.contains("background") {
            controller.set_background_mode(true, false);
        } else if opts.contains("no-background") {
            controller.set_background_mode(false, false);
        }

        let provider_override = opts
            .lookup_value("provider", Some(glib::VariantTy::STRING))
            .and_then(|v| v.str().map(str::to_owned))
            .and_then(|s| Provider::from_str_loose(&s));

        if let Some(screenshot_path) = opts
            .lookup_value("screenshot", Some(glib::VariantTy::STRING))
            .and_then(|v| v.str().map(PathBuf::from))
        {
            controller.schedule_screenshot(screenshot_path);
        }

        if opts.contains("login") {
            log_info!("cli", "Opening Google login");
            controller.open_login();
            return 0.into();
        }

        if opts.contains("preferences") {
            log_info!("cli", "Opening preferences");
            controller.open_preferences();
            return 0.into();
        }

        if opts.contains("clipboard") {
            log_info!("cli", "Submitting query from clipboard");
            controller.submit_from_clipboard();
            return 0.into();
        }

        if opts.contains("voice") {
            log_info!("cli", "Opening Spotlight in voice mode (provider={provider_override:?})");
            controller.present_and_trigger_voice(provider_override);
            return 0.into();
        }

        let query_opt = opts
            .lookup_value("query", Some(glib::VariantTy::STRING))
            .and_then(|v| v.str().map(str::to_owned));

        let followup_opt = opts
            .lookup_value("followup", Some(glib::VariantTy::STRING))
            .and_then(|v| v.str().map(str::to_owned));

        let positional_query: Option<String> = {
            let raw_args = cmd.arguments();
            let parts: Vec<String> = raw_args
                .iter()
                .skip(1)
                .filter_map(|os| os.to_str().map(str::to_owned))
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(" "))
            }
        };

        if let Some(q) = query_opt.or(positional_query) {
            controller.submit_query(&q, provider_override);
            if let Some(followup) = followup_opt {
                controller.schedule_followup_for_test(followup);
            }
        } else {
            controller.present_and_focus(provider_override);
        }

        0.into()
    });

    app.run_with_args(&args)
}

fn register_cli_options(app: &adw::Application) {
    app.add_main_option(
        "query",
        glib::Char::from(b'q'),
        glib::OptionFlags::NONE,
        glib::OptionArg::String,
        "Query to search immediately",
        Some("TEXT"),
    );
    app.add_main_option(
        "provider",
        glib::Char::from(b'p'),
        glib::OptionFlags::NONE,
        glib::OptionArg::String,
        "Provider to use (ai-mode, gemini, google, antigravity)",
        Some("NAME"),
    );
    app.add_main_option(
        "clipboard",
        glib::Char::from(b'c'),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Use current clipboard text as the query",
        None,
    );
    app.add_main_option(
        "background",
        glib::Char::from(b'b'),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Keep application running in background when closed",
        None,
    );
    app.add_main_option(
        "no-background",
        glib::Char::from(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Quit when window is closed",
        None,
    );
    app.add_main_option(
        "login",
        glib::Char::from(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Open Google sign-in page",
        None,
    );
    app.add_main_option(
        "stdin",
        glib::Char::from(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Read query from standard input",
        None,
    );
    app.add_main_option(
        "voice",
        glib::Char::from(b'v'),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Open Spotlight and trigger speech-to-text",
        None,
    );
    app.add_main_option(
        "preferences",
        glib::Char::from(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Open Preferences dialog",
        None,
    );
    app.add_main_option(
        "screenshot",
        glib::Char::from(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::String,
        "Save a WebView PNG screenshot after loading and exit",
        Some("PATH"),
    );
    app.add_main_option(
        "followup",
        glib::Char::from(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::String,
        "Send a follow-up message after the initial query (for testing)",
        Some("TEXT"),
    );
}

/// Reads stdin before `Application::run_with_args` so piped input works even
/// when forwarding to an already-running D-Bus instance.
fn prepare_args_with_stdin() -> Vec<String> {
    let raw_args: Vec<String> = std::env::args().collect();
    let has_stdin_flag = raw_args.iter().any(|a| a == "--stdin");
    let has_query_flag = raw_args
        .iter()
        .any(|a| a == "-q" || a == "--query" || a.starts_with("--query="));
    let has_help_flag = raw_args.iter().any(|a| a == "-h" || a == "--help");
    let stdin_is_piped = !std::io::stdin().is_terminal();

    let mut filtered_args: Vec<String> = raw_args
        .into_iter()
        .filter(|a| a != "--stdin")
        .collect();

    if !has_help_flag && !has_query_flag && (has_stdin_flag || stdin_is_piped) {
        let mut buf = String::new();
        if std::io::stdin().read_to_string(&mut buf).is_ok() {
            let trimmed = buf.trim();
            if !trimmed.is_empty() {
                filtered_args.push("--query".to_string());
                filtered_args.push(trimmed.to_string());
            }
        }
    }

    filtered_args
}

fn load_css() {
    let provider = gtk4::CssProvider::new();
    provider.load_from_string(include_str!("../data/style.css"));
    if let Some(display) = gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
