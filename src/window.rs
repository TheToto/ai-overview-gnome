use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use adw::prelude::*;
use gtk4::gdk;
use gtk4::gio;
use gtk4::glib;
use gtk4::graphene;
use libadwaita as adw;
use webkit6::prelude::*;
use webkit6::{LoadEvent, WebView};

use crate::antigravity::{self, AgyEvent};
use crate::config::{Config, SpeechToText};
use crate::extractor::{ExtractedAnswer, ai_mode_extract_js, ai_mode_followup_js};
use crate::preferences::present_preferences;
use crate::provider::{GOOGLE_LOGIN_URL, Provider};
use crate::voice::{self, WAVEFORM_BARS, VoiceRecorder};
use crate::webview::create_webview;

fn format_accel_label(accel: &str) -> Option<String> {
    let trimmed = accel.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (key, mods) = gtk4::accelerator_parse(trimmed)?;
    let label = gtk4::accelerator_get_label(key, mods).to_string();
    (!label.is_empty()).then_some(label)
}

#[derive(Clone)]
pub struct SpotlightController {
    inner: Rc<SpotlightInner>,
}

struct SpotlightInner {
    app: adw::Application,
    window: adw::ApplicationWindow,
    input_stack: gtk4::Stack,
    search_entry: gtk4::SearchEntry,
    waveform_area: gtk4::DrawingArea,
    rec_hint: gtk4::Label,
    transcribing_bar: gtk4::ProgressBar,
    mic_btn: gtk4::Button,
    provider_dropdown: gtk4::DropDown,
    web_toggle: gtk4::ToggleButton,
    results_box: gtk4::Box,
    reader_box: gtk4::Box,
    messages_scroll: gtk4::ScrolledWindow,
    messages_box: gtk4::Box,
    followup_box: gtk4::Box,
    followup_entry: gtk4::Entry,
    webview: WebView,
    config: RefCell<Config>,
    pending_gemini_prompt: RefCell<Option<String>>,
    conversation_queries: RefCell<Vec<String>>,
    antigravity_conv_id: RefCell<Option<String>>,
    active_ai_label: RefCell<Option<gtk4::Label>>,
    active_tools_expander: RefCell<Option<gtk4::Expander>>,
    active_tools_box: RefCell<Option<gtk4::Box>>,
    active_spinner: RefCell<Option<gtk4::Box>>,
    active_loading_label: RefCell<Option<gtk4::Label>>,
    active_suggestions_box: RefCell<Option<gtk4::Box>>,
    active_turn_plain_text: RefCell<Option<Rc<RefCell<String>>>>,
    last_plain_text: RefCell<String>,
    query_generation: Cell<u64>,
    screenshot_path: RefCell<Option<PathBuf>>,
    pending_test_followup: RefCell<Option<String>>,
    voice_recorder: RefCell<Option<VoiceRecorder>>,
    waveform_levels: RefCell<Arc<Mutex<VecDeque<f32>>>>,
    waveform_phase: Cell<f64>,
    waveform_amp: Cell<f64>,
    is_transcribing: Cell<bool>,
    hold_guard: RefCell<Option<gio::ApplicationHoldGuard>>,
    syncing_ui: RefCell<bool>,
    has_antigravity: bool,
    awaiting_captcha: Cell<bool>,
}

impl SpotlightController {
    pub fn new(app: &adw::Application, initial_config: Config) -> Self {
        // Launch compactly as a floating Spotlight bar; expands when a query is submitted.
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("AI Overview")
            .default_width(initial_config.width)
            .default_height(1)
            .decorated(false)
            .resizable(true)
            .build();
        window.add_css_class("spotlight-window");

        // Top bar (draggable via WindowHandle)
        let search_entry = gtk4::SearchEntry::builder()
            .hexpand(true)
            .placeholder_text(crate::i18n::tr(
                "Ask AI Overview… (Enter to search, Esc to close)",
                "Demander à AI Overview… (Entrée pour chercher, Échap pour fermer)",
            ))
            .build();
        search_entry.add_css_class("spotlight-search");

        let waveform_levels = Arc::new(Mutex::new(VecDeque::from(vec![0.08_f32; WAVEFORM_BARS])));
        let waveform_area = gtk4::DrawingArea::builder()
            .hexpand(true)
            .content_height(34)
            .build();

        let rec_hint = gtk4::Label::new(Some(crate::i18n::tr("Ctrl+G / Enter", "Ctrl+G / Entrée")));
        rec_hint.add_css_class("dim-label");
        rec_hint.add_css_class("caption");

        let recording_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        recording_box.add_css_class("spotlight-waveform-box");
        recording_box.append(&waveform_area);
        recording_box.append(&rec_hint);

        let transcribing_bar = gtk4::ProgressBar::builder()
            .hexpand(true)
            .valign(gtk4::Align::Center)
            .pulse_step(0.08)
            .build();
        transcribing_bar.add_css_class("spotlight-loading-bar");
        let transcribing_label = gtk4::Label::new(Some(crate::i18n::tr("Transcribing…", "Transcription…")));
        transcribing_label.add_css_class("dim-label");
        transcribing_label.add_css_class("caption");
        let transcribing_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
        transcribing_box.add_css_class("spotlight-waveform-box");
        transcribing_box.append(&transcribing_bar);
        transcribing_box.append(&transcribing_label);

        let input_stack = gtk4::Stack::builder()
            .hexpand(true)
            .transition_type(gtk4::StackTransitionType::Crossfade)
            .transition_duration(120)
            .build();
        input_stack.add_named(&search_entry, Some("search"));
        input_stack.add_named(&recording_box, Some("recording"));
        input_stack.add_named(&transcribing_box, Some("transcribing"));
        input_stack.set_visible_child_name("search");

        let mic_btn = gtk4::Button::builder()
            .icon_name("audio-input-microphone-symbolic")
            .tooltip_text(crate::i18n::tr(
                "Voice dictation (Ctrl+G)",
                "Dictée vocale (Ctrl+G)",
            ))
            .build();

        let has_antigravity = antigravity::is_available();
        let provider_labels: Vec<&str> = Provider::available(has_antigravity)
            .iter()
            .map(|p| p.label())
            .collect();
        let provider_dropdown = gtk4::DropDown::from_strings(&provider_labels);
        let initial_selected = if !has_antigravity && initial_config.provider == Provider::Antigravity {
            0
        } else {
            initial_config.provider.index()
        };
        provider_dropdown.set_selected(initial_selected);
        provider_dropdown.set_tooltip_text(Some(crate::i18n::tr(
            "Select AI provider",
            "Choisir le moteur IA",
        )));

        let web_toggle = gtk4::ToggleButton::builder()
            .icon_name("web-browser-symbolic")
            .tooltip_text(crate::i18n::tr("Toggle Web View", "Afficher la vue Web"))
            .visible(false)
            .build();

        // Native GNOME PopoverMenu
        let actions_section = gio::Menu::new();
        actions_section.append(
            Some(crate::i18n::tr("Sign in to Google", "Connexion Google")),
            Some("win.login"),
        );
        actions_section.append(
            Some(crate::i18n::tr(
                "Ask from Clipboard",
                "Demander depuis le presse-papiers",
            )),
            Some("win.clipboard"),
        );

        let nav_item = gio::MenuItem::new(None, None);
        nav_item.set_attribute_value("custom", Some(&"nav".to_variant()));
        let nav_section = gio::Menu::new();
        nav_section.append_item(&nav_item);

        let app_section = gio::Menu::new();
        app_section.append(
            Some(crate::i18n::tr("Preferences", "Paramètres")),
            Some("win.preferences"),
        );
        app_section.append(
            Some(crate::i18n::tr("Quit", "Quitter")),
            Some("win.quit"),
        );

        let menu = gio::Menu::new();
        menu.append_section(None, &actions_section);
        menu.append_section(None, &nav_section);
        menu.append_section(None, &app_section);

        let back_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
        back_btn.set_tooltip_text(Some(crate::i18n::tr("Back", "Précédent")));
        back_btn.add_css_class("flat");
        back_btn.add_css_class("circular");

        let forward_btn = gtk4::Button::from_icon_name("go-next-symbolic");
        forward_btn.set_tooltip_text(Some(crate::i18n::tr("Forward", "Suivant")));
        forward_btn.add_css_class("flat");
        forward_btn.add_css_class("circular");

        let reload_btn = gtk4::Button::from_icon_name("view-refresh-symbolic");
        reload_btn.set_tooltip_text(Some(crate::i18n::tr("Reload", "Actualiser")));
        reload_btn.add_css_class("flat");
        reload_btn.add_css_class("circular");

        let nav_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        nav_row.set_halign(gtk4::Align::Center);
        nav_row.append(&back_btn);
        nav_row.append(&forward_btn);
        nav_row.append(&reload_btn);

        let popover = gtk4::PopoverMenu::from_model(Some(&menu));
        popover.add_child(&nav_row, "nav");

        let menu_btn = gtk4::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .popover(&popover)
            .primary(true)
            .tooltip_text(crate::i18n::tr("Main Menu", "Menu principal"))
            .build();

        let top_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        top_box.add_css_class("spotlight-topbar");
        top_box.append(&input_stack);
        top_box.append(&mic_btn);
        top_box.append(&provider_dropdown);
        top_box.append(&web_toggle);
        top_box.append(&menu_btn);

        let handle = gtk4::WindowHandle::new();
        handle.set_child(Some(&top_box));

        // Slim progress bar under the top bar
        let progress_bar = gtk4::ProgressBar::new();
        progress_bar.add_css_class("spotlight-progress");
        progress_bar.set_visible(false);

        // Native conversational feed
        let messages_box = gtk4::Box::new(gtk4::Orientation::Vertical, 14);
        messages_box.add_css_class("spotlight-messages");

        let messages_scroll = gtk4::ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .child(&messages_box)
            .build();

        // Native follow-up bar (only shown in AI Mode)
        let followup_entry = gtk4::Entry::builder()
            .hexpand(true)
            .placeholder_text(crate::i18n::tr(
                "Ask a follow-up…",
                "Répondre au modèle…",
            ))
            .build();
        followup_entry.add_css_class("spotlight-followup-entry");

        let followup_send_btn = gtk4::Button::builder()
            .icon_name("go-up-symbolic")
            .tooltip_text(crate::i18n::tr("Send follow-up", "Envoyer la réponse"))
            .build();
        followup_send_btn.add_css_class("suggested-action");

        let followup_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        followup_box.add_css_class("spotlight-followup-bar");
        followup_box.append(&followup_entry);
        followup_box.append(&followup_send_btn);
        followup_box.set_visible(false);

        let reader_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        reader_box.add_css_class("spotlight-reader");
        reader_box.set_vexpand(true);
        reader_box.set_hexpand(true);
        reader_box.append(&messages_scroll);
        reader_box.append(&followup_box);

        let webview = create_webview(&initial_config);

        let overlay = gtk4::Overlay::new();
        overlay.set_vexpand(true);
        overlay.set_hexpand(true);
        overlay.set_child(Some(&webview));
        overlay.add_overlay(&reader_box);

        let results_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        results_box.set_vexpand(true);
        results_box.set_hexpand(true);
        results_box.append(&progress_bar);
        results_box.append(&overlay);
        results_box.set_visible(false);

        let root_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root_box.append(&handle);
        root_box.append(&results_box);

        window.set_content(Some(&root_box));

        let initial_hold = if initial_config.background_mode {
            Some(app.hold())
        } else {
            None
        };

        let controller = Self {
            inner: Rc::new(SpotlightInner {
                app: app.clone(),
                window,
                input_stack,
                search_entry,
                waveform_area,
                rec_hint,
                transcribing_bar,
                mic_btn,
                provider_dropdown,
                web_toggle,
                results_box,
                reader_box,
                messages_scroll,
                messages_box,
                followup_box,
                followup_entry,
                webview,
                config: RefCell::new(initial_config),
                pending_gemini_prompt: RefCell::new(None),
                conversation_queries: RefCell::new(Vec::new()),
                antigravity_conv_id: RefCell::new(None),
                active_ai_label: RefCell::new(None),
                active_tools_expander: RefCell::new(None),
                active_tools_box: RefCell::new(None),
                active_spinner: RefCell::new(None),
                active_loading_label: RefCell::new(None),
                active_suggestions_box: RefCell::new(None),
                active_turn_plain_text: RefCell::new(None),
                last_plain_text: RefCell::new(String::new()),
                query_generation: Cell::new(0),
                screenshot_path: RefCell::new(None),
                pending_test_followup: RefCell::new(None),
                voice_recorder: RefCell::new(None),
                waveform_levels: RefCell::new(waveform_levels),
                waveform_phase: Cell::new(0.0),
                waveform_amp: Cell::new(0.16),
                is_transcribing: Cell::new(false),
                hold_guard: RefCell::new(initial_hold),
                syncing_ui: RefCell::new(false),
                has_antigravity,
                awaiting_captcha: Cell::new(false),
            }),
        };

        controller.wire_signals(
            &progress_bar,
            &popover,
            &followup_send_btn,
            &back_btn,
            &forward_btn,
            &reload_btn,
        );
        controller.update_voice_shortcut_hints();
        controller
    }

    fn wire_signals(
        &self,
        progress_bar: &gtk4::ProgressBar,
        popover: &gtk4::PopoverMenu,
        followup_send_btn: &gtk4::Button,
        back_btn: &gtk4::Button,
        forward_btn: &gtk4::Button,
        reload_btn: &gtk4::Button,
    ) {
        // Fluid multi-layered audio wave DrawingArea
        let ctrl = self.clone();
        self.inner.waveform_area.set_draw_func(move |_, cr, width, height| {
            let w = width as f64;
            let h = height as f64;
            if w <= 4.0 || h <= 4.0 {
                return;
            }
            let cy = h / 2.0;
            let max_amp = (h / 2.0 - 4.0).max(4.0);
            let phase = ctrl.inner.waveform_phase.get();
            let amp = ctrl.inner.waveform_amp.get();

            cr.set_line_cap(gtk4::cairo::LineCap::Round);
            cr.set_line_join(gtk4::cairo::LineJoin::Round);

            let wave_y = |t: f64, freq: f64, p: f64, scale: f64| -> f64 {
                let env = (std::f64::consts::PI * t).sin().powf(1.6);
                let carrier = (t * std::f64::consts::TAU * freq + p).sin()
                    + 0.45 * (t * std::f64::consts::TAU * (freq * 1.73) - p * 0.7).sin();
                (carrier / 1.45) * env * amp * max_amp * scale
            };

            let steps = ((w / 3.0).ceil() as usize).max(32);

            // Soft symmetric ribbon fill under primary wave
            cr.new_path();
            for i in 0..=steps {
                let t = i as f64 / steps as f64;
                let x = t * w;
                let y = cy + wave_y(t, 2.6, phase, 1.0);
                if i == 0 {
                    cr.move_to(x, y);
                } else {
                    cr.line_to(x, y);
                }
            }
            for i in (0..=steps).rev() {
                let t = i as f64 / steps as f64;
                let x = t * w;
                let y = cy - wave_y(t, 2.6, phase, 1.0);
                cr.line_to(x, y);
            }
            cr.close_path();
            cr.set_source_rgba(0.208, 0.518, 0.894, 0.14);
            let _ = cr.fill();

            // Three layered smooth waves (back violet glow, mid light blue, front Adwaita blue)
            let layers = [
                (2.2, 0.85, 0.78, 2.6, (0.57, 0.36, 0.92, 0.30)),
                (3.1, 1.15, 0.92, 2.2, (0.38, 0.63, 0.92, 0.58)),
                (2.6, 1.00, 1.00, 2.2, (0.208, 0.518, 0.894, 0.95)),
            ];

            for (freq, p_mult, scale, lw, (r, g, b, a)) in layers {
                cr.new_path();
                for i in 0..=steps {
                    let t = i as f64 / steps as f64;
                    let x = t * w;
                    let y = cy + wave_y(t, freq, phase * p_mult, scale);
                    if i == 0 {
                        cr.move_to(x, y);
                    } else {
                        cr.line_to(x, y);
                    }
                }
                cr.set_source_rgba(r, g, b, a);
                cr.set_line_width(lw);
                let _ = cr.stroke();
            }
        });

        // Mic button toggle
        let ctrl = self.clone();
        self.inner.mic_btn.connect_clicked(move |_| {
            ctrl.toggle_voice_recording();
        });

        // Search entry activate (Enter)
        let ctrl = self.clone();
        self.inner.search_entry.connect_activate(move |entry| {
            let query = entry.text().to_string();
            if !query.trim().is_empty() {
                ctrl.submit_query(&query, None);
            }
        });

        // Follow-up entry activate (Enter) & send button
        let ctrl = self.clone();
        self.inner.followup_entry.connect_activate(move |entry| {
            let text = entry.text().to_string();
            if !text.trim().is_empty() {
                ctrl.submit_followup(&text);
            }
        });

        let ctrl = self.clone();
        followup_send_btn.connect_clicked(move |_| {
            let text = ctrl.inner.followup_entry.text().to_string();
            if !text.trim().is_empty() {
                ctrl.submit_followup(&text);
            }
        });

        // Provider dropdown change (temporary for current session, does not overwrite default)
        let ctrl = self.clone();
        self.inner
            .provider_dropdown
            .connect_selected_notify(move |dd| {
                if *ctrl.inner.syncing_ui.borrow() {
                    return;
                }
                let provider = Provider::from_index(dd.selected());
                let current_query = ctrl.inner.search_entry.text().to_string();
                if !current_query.trim().is_empty() && ctrl.inner.results_box.is_visible() {
                    ctrl.submit_query(&current_query, Some(provider));
                }
            });

        // Web view toggle button
        let ctrl = self.clone();
        self.inner.web_toggle.connect_toggled(move |btn| {
            if *ctrl.inner.syncing_ui.borrow() {
                return;
            }
            ctrl.inner.reader_box.set_visible(!btn.is_active());
        });

        // Native window actions for PopoverMenu
        let login_action = gio::SimpleAction::new("login", None);
        let ctrl = self.clone();
        login_action.connect_activate(move |_, _| {
            ctrl.open_login();
        });
        self.inner.window.add_action(&login_action);

        let clipboard_action = gio::SimpleAction::new("clipboard", None);
        let ctrl = self.clone();
        clipboard_action.connect_activate(move |_, _| {
            ctrl.submit_from_clipboard();
        });
        self.inner.window.add_action(&clipboard_action);

        let prefs_action = gio::SimpleAction::new("preferences", None);
        let ctrl = self.clone();
        prefs_action.connect_activate(move |_, _| {
            ctrl.open_preferences();
        });
        self.inner.window.add_action(&prefs_action);

        let quit_action = gio::SimpleAction::new("quit", None);
        let ctrl = self.clone();
        quit_action.connect_activate(move |_, _| {
            ctrl.quit();
        });
        self.inner.window.add_action(&quit_action);

        self.inner
            .app
            .set_accels_for_action("win.preferences", &["<Primary>comma"]);
        self.inner
            .app
            .set_accels_for_action("win.quit", &["<Primary>q"]);

        let wv = self.inner.webview.clone();
        let pop = popover.clone();
        back_btn.connect_clicked(move |_| {
            pop.popdown();
            if wv.can_go_back() {
                wv.go_back();
            }
        });

        let wv = self.inner.webview.clone();
        let pop = popover.clone();
        forward_btn.connect_clicked(move |_| {
            pop.popdown();
            if wv.can_go_forward() {
                wv.go_forward();
            }
        });

        let wv = self.inner.webview.clone();
        let pop = popover.clone();
        reload_btn.connect_clicked(move |_| {
            pop.popdown();
            wv.reload();
        });

        // Progress bar binding & load failure logging
        let pb = progress_bar.clone();
        self.inner
            .webview
            .connect_estimated_load_progress_notify(move |wv| {
                let p = wv.estimated_load_progress();
                pb.set_fraction(p);
                pb.set_visible(p > 0.0 && p < 1.0);
            });
        self.inner
            .webview
            .connect_load_failed(|_, _, failing_uri, error| {
                log_err!("webview", "Load failed for {failing_uri}: {error}");
                false
            });

        // Inject Gemini prompt when page finishes loading + CAPTCHA WebView fallback/recovery + test hooks
        let ctrl = self.clone();
        self.inner.webview.connect_load_changed(move |wv, event| {
            let uri = wv.uri().map(|u| u.to_string()).unwrap_or_default();
            if (event == LoadEvent::Committed || event == LoadEvent::Finished)
                && uri.contains("google.com/sorry")
            {
                if !ctrl.inner.awaiting_captcha.get() {
                    log_warn!("webview", "CAPTCHA detected ({uri}), switching to Web View");
                    ctrl.inner.awaiting_captcha.set(true);
                    ctrl.set_web_view_mode(true);
                }
                return;
            }

            if event != LoadEvent::Finished {
                return;
            }

            if ctrl.inner.awaiting_captcha.get() {
                log_info!("webview", "CAPTCHA solved ({uri}), reloading and resuming reader");
                ctrl.inner.awaiting_captcha.set(false);
                let provider = Provider::from_index(ctrl.inner.provider_dropdown.selected());
                if provider == Provider::GoogleAiMode {
                    ctrl.set_web_view_mode(false);
                    let generation = ctrl.inner.query_generation.get().wrapping_add(1);
                    ctrl.inner.query_generation.set(generation);
                    wv.reload();
                    ctrl.start_ai_mode_polling(generation, None);
                }
                return;
            }
            if let Some(prompt) = ctrl.inner.pending_gemini_prompt.borrow_mut().take() {
                let temp_chat = ctrl.inner.config.borrow().gemini_temporary_chat;
                let js = Provider::gemini_inject_js(&prompt, temp_chat);
                wv.evaluate_javascript(&js, None, None, None::<&gio::Cancellable>, |_| {});
            }

            let followup_opt = ctrl.inner.pending_test_followup.borrow_mut().take();
            let screenshot_opt = ctrl.inner.screenshot_path.borrow_mut().take();

            if let Some(followup) = followup_opt {
                let ctrl_f = ctrl.clone();
                glib::timeout_add_local_once(Duration::from_secs(6), move || {
                    ctrl_f.submit_followup(&followup);
                    if let Some(path) = screenshot_opt {
                        let ctrl_s = ctrl_f.clone();
                        glib::timeout_add_local_once(Duration::from_secs(6), move || {
                            ctrl_s.capture_screenshot_and_quit(&path);
                        });
                    }
                });
            } else if let Some(path) = screenshot_opt {
                let ctrl_s = ctrl.clone();
                glib::timeout_add_local_once(Duration::from_secs(7), move || {
                    ctrl_s.capture_screenshot_and_quit(&path);
                });
            }
        });

        // Keyboard shortcuts: Esc, Return (while recording), Ctrl+G, Ctrl+L, Ctrl+Q, Ctrl+,
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let ctrl = self.clone();
        key_controller.connect_key_pressed(move |_, key, _, mods| {
            if key == gdk::Key::Escape {
                if ctrl.inner.voice_recorder.borrow().is_some() {
                    ctrl.cancel_voice_recording();
                } else {
                    ctrl.dismiss();
                }
                return glib::Propagation::Stop;
            }
            if (key == gdk::Key::Return || key == gdk::Key::KP_Enter)
                && ctrl.inner.voice_recorder.borrow().is_some()
            {
                ctrl.finish_voice_recording();
                return glib::Propagation::Stop;
            }
            if mods.contains(gdk::ModifierType::CONTROL_MASK) {
                match key {
                    gdk::Key::g | gdk::Key::G => {
                        ctrl.toggle_voice_recording();
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::l | gdk::Key::L => {
                        ctrl.focus_search();
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::q | gdk::Key::Q => {
                        ctrl.quit();
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::comma => {
                        ctrl.open_preferences();
                        return glib::Propagation::Stop;
                    }
                    _ => {}
                }
            }
            glib::Propagation::Proceed
        });
        self.inner.window.add_controller(key_controller);

        // Close request: hide in background mode, otherwise quit cleanly
        let ctrl = self.clone();
        self.inner.window.connect_close_request(move |win| {
            if ctrl.inner.config.borrow().background_mode {
                win.set_visible(false);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }

    fn set_web_view_mode(&self, web_mode: bool) {
        *self.inner.syncing_ui.borrow_mut() = true;
        self.inner.web_toggle.set_active(web_mode);
        *self.inner.syncing_ui.borrow_mut() = false;
        self.inner.reader_box.set_visible(!web_mode);
    }

    fn expand_results_area(&self) {
        if !self.inner.results_box.is_visible() {
            self.inner.results_box.set_visible(true);
            let cfg = self.inner.config.borrow();
            self.inner.window.set_default_size(cfg.width, cfg.height);
        }
    }

    fn append_turn_widgets(&self, user_text: &str) {
        let user_label = gtk4::Label::builder()
            .label(user_text)
            .wrap(true)
            .selectable(true)
            .xalign(0.0)
            .halign(gtk4::Align::End)
            .build();
        user_label.add_css_class("spotlight-user-bubble");
        self.inner.messages_box.append(&user_label);

        let loading_label = gtk4::Label::builder()
            .label(crate::i18n::tr(
                "Generating response…",
                "Génération de la réponse…",
            ))
            .xalign(0.0)
            .build();
        loading_label.add_css_class("dim-label");
        loading_label.add_css_class("caption");
        let loading_bar = gtk4::ProgressBar::builder()
            .hexpand(true)
            .pulse_step(0.08)
            .build();
        loading_bar.add_css_class("spotlight-loading-bar");

        let spinner_row = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        spinner_row.append(&loading_label);
        spinner_row.append(&loading_bar);

        let bar_ref = loading_bar.clone();
        let row_ref = spinner_row.clone();
        glib::timeout_add_local(Duration::from_millis(80), move || {
            if !row_ref.is_visible() || row_ref.parent().is_none() {
                return glib::ControlFlow::Break;
            }
            bar_ref.pulse();
            glib::ControlFlow::Continue
        });

        let ai_label = gtk4::Label::builder()
            .use_markup(true)
            .selectable(true)
            .wrap(true)
            .xalign(0.0)
            .yalign(0.0)
            .visible(false)
            .build();
        ai_label.add_css_class("spotlight-ai-message");
        let win_for_links = self.inner.window.clone();
        ai_label.connect_activate_link(move |_, uri| {
            gtk4::UriLauncher::new(uri).launch(
                Some(&win_for_links),
                None::<&gio::Cancellable>,
                |_| {},
            );
            glib::Propagation::Stop
        });

        let turn_plain = Rc::new(RefCell::new(String::new()));
        let action_group = gio::SimpleActionGroup::new();
        let copy_action = gio::SimpleAction::new("copy-response", None);
        let win_for_copy = self.inner.window.clone();
        let plain_for_copy = turn_plain.clone();
        copy_action.connect_activate(move |_, _| {
            let text = plain_for_copy.borrow().clone();
            if !text.is_empty() {
                win_for_copy.clipboard().set_text(&text);
            }
        });
        action_group.add_action(&copy_action);
        ai_label.insert_action_group("ai", Some(&action_group));

        let extra_menu = gio::Menu::new();
        extra_menu.append(
            Some(crate::i18n::tr("Copy response", "Copier la réponse")),
            Some("ai.copy-response"),
        );
        ai_label.set_extra_menu(Some(&extra_menu));

        let tools_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        tools_box.add_css_class("spotlight-tools-list");
        let tools_expander = gtk4::Expander::builder()
            .label(crate::i18n::tr("⚙ Actions", "⚙ Actions"))
            .child(&tools_box)
            .expanded(false)
            .visible(false)
            .build();
        tools_expander.add_css_class("spotlight-tools-expander");

        let suggestions_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        suggestions_box.add_css_class("spotlight-suggestions");
        suggestions_box.set_visible(false);

        let ai_container = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        ai_container.append(&tools_expander);
        ai_container.append(&ai_label);
        ai_container.append(&suggestions_box);
        ai_container.append(&spinner_row);

        self.inner.messages_box.append(&ai_container);
        *self.inner.active_tools_expander.borrow_mut() = Some(tools_expander);
        *self.inner.active_tools_box.borrow_mut() = Some(tools_box);
        *self.inner.active_spinner.borrow_mut() = Some(spinner_row);
        *self.inner.active_loading_label.borrow_mut() = Some(loading_label);
        *self.inner.active_ai_label.borrow_mut() = Some(ai_label);
        *self.inner.active_suggestions_box.borrow_mut() = Some(suggestions_box);
        *self.inner.active_turn_plain_text.borrow_mut() = Some(turn_plain);

        self.scroll_to_bottom();
    }

    fn scroll_to_bottom(&self) {
        let scroll = self.inner.messages_scroll.clone();
        glib::idle_add_local_once(move || {
            let adj = scroll.vadjustment();
            adj.set_value(adj.upper() - adj.page_size());
        });
    }

    fn capture_screenshot_and_quit(&self, path: &PathBuf) {
        let diag_js = r#"JSON.stringify({
            url: location.href,
            debug: window.__aiOverviewDebug || [],
            textSample: (document.body?.innerText || '').slice(0, 400)
        })"#;
        let plain_len = self.inner.last_plain_text.borrow().len();
        eprintln!("[screenshot-debug] reader_plain_len={plain_len}");
        self.inner.webview.evaluate_javascript(
            diag_js,
            None,
            None,
            None::<&gio::Cancellable>,
            |res| match res {
                Ok(val) => eprintln!("[screenshot-debug] {}", val.to_str()),
                Err(err) => eprintln!("[screenshot-debug] js error: {err}"),
            },
        );

        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let win = &self.inner.window;
        let width = win.width() as f64;
        let height = win.height() as f64;
        if width > 0.0
            && height > 0.0
            && let Some(renderer) = win.renderer()
        {
            let paintable = gtk4::WidgetPaintable::new(Some(win));
            let snapshot = gtk4::Snapshot::new();
            paintable.snapshot(&snapshot, width, height);
            if let Some(node) = snapshot.to_node() {
                let bounds = graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
                let texture = renderer.render_texture(&node, Some(&bounds));
                match texture.save_to_png(path) {
                    Ok(()) => {
                        eprintln!("[screenshot] saved full window {}", path.display());
                        self.quit();
                        return;
                    }
                    Err(e) => eprintln!("[screenshot] window save error: {e}"),
                }
            }
        }

        let path_clone = path.clone();
        let ctrl_clone = self.clone();
        self.inner.webview.snapshot(
            webkit6::SnapshotRegion::Visible,
            webkit6::SnapshotOptions::NONE,
            None::<&gio::Cancellable>,
            move |res| {
                match res {
                    Ok(texture) => match texture.save_to_png(&path_clone) {
                        Ok(()) => eprintln!("[screenshot] saved webview {}", path_clone.display()),
                        Err(e) => eprintln!("[screenshot] save error: {e}"),
                    },
                    Err(e) => eprintln!("[screenshot] snapshot error: {e}"),
                }
                ctrl_clone.quit();
            },
        );
    }

    fn start_ai_mode_polling(&self, generation: u64, ignore_text: Option<String>) {
        let ctrl = self.clone();
        let last_len = Rc::new(Cell::new(0usize));
        let stable_ticks = Rc::new(Cell::new(0u32));
        let total_ticks = Rc::new(Cell::new(0u32));

        glib::timeout_add_local(Duration::from_millis(150), move || {
            if ctrl.inner.query_generation.get() != generation || ctrl.inner.awaiting_captcha.get() {
                return glib::ControlFlow::Break;
            }

            let ticks = total_ticks.get() + 1;
            total_ticks.set(ticks);

            let queries = ctrl.inner.conversation_queries.borrow().clone();
            let js = ai_mode_extract_js(&queries);
            let ctrl_cb = ctrl.clone();
            let last_len_cb = last_len.clone();
            let stable_cb = stable_ticks.clone();
            let ignore_cb = ignore_text.clone();

            ctrl.inner.webview.evaluate_javascript(
                &js,
                None,
                None,
                None::<&gio::Cancellable>,
                move |res| {
                    if ctrl_cb.inner.query_generation.get() != generation {
                        return;
                    }
                    if let Ok(val) = res
                        && let Some(answer) = ExtractedAnswer::from_json(val.to_str().as_str())
                    {
                        let plain = answer.to_plain_text();
                        if let Some(prev) = &ignore_cb
                            && &plain == prev
                        {
                            return;
                        }
                        if answer.raw_len != last_len_cb.get() {
                            last_len_cb.set(answer.raw_len);
                            stable_cb.set(0);
                            ctrl_cb.render_extracted_answer(&answer, plain);
                        } else {
                            stable_cb.set(stable_cb.get() + 1);
                        }
                    }
                },
            );

            if last_len.get() > 0 && stable_ticks.get() >= 10 {
                ctrl.finish_active_loading();
                return glib::ControlFlow::Break;
            }
            if ticks >= 80 {
                log_warn!(
                    "ai_mode",
                    "Polling timed out (last_len={}), falling back to webview={}",
                    last_len.get(),
                    last_len.get() == 0
                );
                ctrl.finish_active_loading();
                if last_len.get() == 0 {
                    ctrl.set_web_view_mode(true);
                }
                return glib::ControlFlow::Break;
            }

            glib::ControlFlow::Continue
        });
    }

    fn finish_active_loading(&self) {
        if let Some(spinner_row) = self.inner.active_spinner.borrow().as_ref() {
            spinner_row.set_visible(false);
        }
    }

    fn render_extracted_answer(&self, answer: &ExtractedAnswer, plain: String) {
        if let Some(ai_label) = self.inner.active_ai_label.borrow().as_ref() {
            ai_label.set_visible(true);
            ai_label.set_markup(&answer.to_pango_markup());
        }
        if let Some(turn_plain) = self.inner.active_turn_plain_text.borrow().as_ref() {
            *turn_plain.borrow_mut() = plain.clone();
        }
        if let Some(suggestions_box) = self.inner.active_suggestions_box.borrow().as_ref() {
            while let Some(child) = suggestions_box.first_child() {
                suggestions_box.remove(&child);
            }
            for suggestion in &answer.suggestions {
                let lbl = gtk4::Label::builder()
                    .label(format!("↪ {suggestion}"))
                    .wrap(true)
                    .xalign(0.0)
                    .build();
                let btn = gtk4::Button::builder().child(&lbl).build();
                btn.add_css_class("flat");
                btn.add_css_class("spotlight-suggestion-btn");
                let ctrl = self.clone();
                let text = suggestion.clone();
                btn.connect_clicked(move |_| {
                    ctrl.submit_followup(&text);
                });
                suggestions_box.append(&btn);
            }
            suggestions_box.set_visible(!answer.suggestions.is_empty());
        }
        if !answer.suggestions.is_empty() {
            self.finish_active_loading();
        }
        *self.inner.last_plain_text.borrow_mut() = plain;
        self.scroll_to_bottom();
    }

    pub fn schedule_screenshot(&self, path: PathBuf) {
        *self.inner.screenshot_path.borrow_mut() = Some(path);
    }

    pub fn schedule_followup_for_test(&self, followup: String) {
        *self.inner.pending_test_followup.borrow_mut() = Some(followup);
    }

    pub fn window(&self) -> adw::ApplicationWindow {
        self.inner.window.clone()
    }

    pub fn config_snapshot(&self) -> Config {
        self.inner.config.borrow().clone()
    }

    pub fn set_default_provider(&self, provider: Provider) {
        {
            let mut cfg = self.inner.config.borrow_mut();
            cfg.provider = provider;
            let _ = cfg.save();
        }
        *self.inner.syncing_ui.borrow_mut() = true;
        self.inner.provider_dropdown.set_selected(provider.index());
        *self.inner.syncing_ui.borrow_mut() = false;
    }

    pub fn set_gemini_temporary_chat(&self, enabled: bool) {
        let mut cfg = self.inner.config.borrow_mut();
        cfg.gemini_temporary_chat = enabled;
        let _ = cfg.save();
    }

    pub fn set_antigravity_auto_approve(&self, enabled: bool) {
        let mut cfg = self.inner.config.borrow_mut();
        cfg.antigravity_auto_approve = enabled;
        let _ = cfg.save();
    }

    pub fn set_antigravity_model(&self, model: Option<String>) {
        let mut cfg = self.inner.config.borrow_mut();
        cfg.antigravity_model = model;
        let _ = cfg.save();
    }

    pub fn set_speech_to_text(&self, stt: SpeechToText) {
        let mut cfg = self.inner.config.borrow_mut();
        cfg.speech_to_text = stt;
        let _ = cfg.save();
    }

    pub fn set_gnome_shortcut(&self, binding: String) {
        let mut cfg = self.inner.config.borrow_mut();
        cfg.gnome_shortcut = binding;
        let _ = cfg.save();
    }

    pub fn set_gnome_shortcut_provider(&self, provider: Option<Provider>) {
        let mut cfg = self.inner.config.borrow_mut();
        cfg.gnome_shortcut_provider = provider;
        let _ = cfg.save();
    }

    pub fn set_gnome_voice_shortcut(&self, binding: String) {
        {
            let mut cfg = self.inner.config.borrow_mut();
            cfg.gnome_voice_shortcut = binding;
            let _ = cfg.save();
        }
        self.update_voice_shortcut_hints();
    }

    fn update_voice_shortcut_hints(&self) {
        let binding = self.inner.config.borrow().gnome_voice_shortcut.clone();
        let accel_text = format_accel_label(&binding).unwrap_or_else(|| "Ctrl+G".to_string());
        let enter_word = crate::i18n::tr("Enter", "Entrée");
        self.inner
            .rec_hint
            .set_text(&format!("{accel_text} / {enter_word}"));
        let voice_label = crate::i18n::tr("Voice dictation", "Dictée vocale");
        self.inner
            .mic_btn
            .set_tooltip_text(Some(&format!("{voice_label} ({accel_text})")));
    }

    pub fn set_gnome_voice_shortcut_provider(&self, provider: Option<Provider>) {
        let mut cfg = self.inner.config.borrow_mut();
        cfg.gnome_voice_shortcut_provider = provider;
        let _ = cfg.save();
    }

    pub fn open_preferences(&self) {
        self.expand_results_area();
        self.inner.window.present();
        present_preferences(self);

        if let Some(path) = self.inner.screenshot_path.borrow_mut().take() {
            let ctrl = self.clone();
            glib::timeout_add_local_once(Duration::from_millis(1200), move || {
                ctrl.capture_screenshot_and_quit(&path);
            });
        }
    }

    pub fn present_and_trigger_voice(&self, provider_override: Option<Provider>) {
        self.apply_provider_for_present(provider_override);
        self.inner.window.present();
        if let Some(path) = self.inner.screenshot_path.borrow_mut().take() {
            self.toggle_voice_recording();
            let ctrl = self.clone();
            glib::timeout_add_local_once(Duration::from_millis(800), move || {
                ctrl.capture_screenshot_and_quit(&path);
            });
            return;
        }
        self.toggle_voice_recording();
    }

    pub fn toggle_voice_recording(&self) {
        if self.inner.is_transcribing.get() {
            return;
        }
        if self.inner.voice_recorder.borrow().is_some() {
            self.finish_voice_recording();
        } else {
            self.start_voice_recording();
        }
    }

    fn start_voice_recording(&self) {
        match VoiceRecorder::start() {
            Ok(recorder) => {
                *self.inner.waveform_levels.borrow_mut() = recorder.levels();
                *self.inner.voice_recorder.borrow_mut() = Some(recorder);
                self.inner.input_stack.set_visible_child_name("recording");
                self.inner
                    .mic_btn
                    .set_icon_name("media-playback-stop-symbolic");
                self.inner.mic_btn.add_css_class("destructive-action");

                let ctrl = self.clone();
                glib::timeout_add_local(Duration::from_millis(16), move || {
                    if ctrl.inner.voice_recorder.borrow().is_none() {
                        return glib::ControlFlow::Break;
                    }
                    let latest = {
                        let arc = ctrl.inner.waveform_levels.borrow();
                        arc.lock()
                            .ok()
                            .and_then(|q| q.back().copied())
                            .unwrap_or(0.14)
                    };
                    let target = (latest as f64).max(0.16);
                    let prev_amp = ctrl.inner.waveform_amp.get();
                    let next_amp = prev_amp * 0.78 + target * 0.22;
                    ctrl.inner.waveform_amp.set(next_amp);
                    ctrl.inner
                        .waveform_phase
                        .set(ctrl.inner.waveform_phase.get() + 0.11 + next_amp * 0.10);
                    ctrl.inner.waveform_area.queue_draw();
                    glib::ControlFlow::Continue
                });
            }
            Err(err) => {
                log_err!("voice", "start_voice_recording failed: {err}");
                self.inner.search_entry.set_text("");
                self.inner.search_entry.set_placeholder_text(Some(&err));
                self.focus_search();
            }
        }
    }

    fn finish_voice_recording(&self) {
        let Some(recorder) = self.inner.voice_recorder.borrow_mut().take() else {
            return;
        };
        self.inner
            .mic_btn
            .set_icon_name("audio-input-microphone-symbolic");
        self.inner.mic_btn.remove_css_class("destructive-action");
        self.inner.mic_btn.set_sensitive(false);
        self.inner.input_stack.set_visible_child_name("transcribing");
        self.inner.is_transcribing.set(true);

        let ctrl_pulse = self.clone();
        glib::timeout_add_local(Duration::from_millis(80), move || {
            if !ctrl_pulse.inner.is_transcribing.get() {
                return glib::ControlFlow::Break;
            }
            ctrl_pulse.inner.transcribing_bar.pulse();
            glib::ControlFlow::Continue
        });

        let (tx, rx) = mpsc::channel::<Result<String, String>>();
        std::thread::spawn(move || {
            let res = recorder
                .stop_and_write_wav()
                .and_then(|wav| voice::transcribe_with_handy(&wav));
            let _ = tx.send(res);
        });

        let ctrl = self.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(result) => {
                    ctrl.inner.is_transcribing.set(false);
                    ctrl.inner.mic_btn.set_sensitive(true);
                    ctrl.inner.input_stack.set_visible_child_name("search");
                    match result {
                        Ok(text) if !text.is_empty() => {
                            log_info!("voice", "Submitting transcribed query ({} chars)", text.chars().count());
                            ctrl.submit_query(&text, None);
                        }
                        Ok(_) => {
                            log_warn!("voice", "Transcription returned empty text");
                            ctrl.inner.search_entry.set_text("");
                            ctrl.inner.search_entry.set_placeholder_text(Some(
                                crate::i18n::tr(
                                    "No speech detected — try again",
                                    "Aucun texte détecté — réessayez",
                                ),
                            ));
                            ctrl.focus_search();
                        }
                        Err(err) => {
                            log_err!("voice", "Transcription error: {err}");
                            ctrl.inner.search_entry.set_text("");
                            ctrl.inner
                                .search_entry
                                .set_placeholder_text(Some(&err));
                            ctrl.focus_search();
                        }
                    }
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    log_err!("voice", "Transcription worker channel disconnected");
                    ctrl.inner.is_transcribing.set(false);
                    ctrl.inner.mic_btn.set_sensitive(true);
                    ctrl.inner.input_stack.set_visible_child_name("search");
                    ctrl.inner.search_entry.set_text("");
                    ctrl.inner.search_entry.set_placeholder_text(Some(
                        crate::i18n::tr(
                            "Transcription interrupted unexpectedly",
                            "Transcription interrompue de façon inattendue",
                        ),
                    ));
                    ctrl.focus_search();
                    glib::ControlFlow::Break
                }
            }
        });
    }

    fn cancel_voice_recording(&self) {
        if let Some(recorder) = self.inner.voice_recorder.borrow_mut().take() {
            recorder.cancel();
        }
        self.inner
            .mic_btn
            .set_icon_name("audio-input-microphone-symbolic");
        self.inner.mic_btn.remove_css_class("destructive-action");
        self.inner.input_stack.set_visible_child_name("search");
        self.focus_search();
    }

    pub fn set_background_mode(&self, enabled: bool, persist: bool) {
        {
            let mut cfg = self.inner.config.borrow_mut();
            cfg.background_mode = enabled;
            if persist {
                let _ = cfg.save();
            }
        }

        let mut guard = self.inner.hold_guard.borrow_mut();
        if enabled && guard.is_none() {
            *guard = Some(self.inner.app.hold());
        } else if !enabled {
            *guard = None;
        }
    }

    fn reset_provider_to_default(&self) {
        let mut default_provider = self.inner.config.borrow().provider;
        if !self.inner.has_antigravity && default_provider == Provider::Antigravity {
            default_provider = Provider::GoogleAiMode;
        }
        *self.inner.syncing_ui.borrow_mut() = true;
        self.inner.provider_dropdown.set_selected(default_provider.index());
        *self.inner.syncing_ui.borrow_mut() = false;
    }

    fn apply_provider_for_present(&self, provider_override: Option<Provider>) {
        if let Some(mut p) = provider_override {
            if !self.inner.has_antigravity && p == Provider::Antigravity {
                p = Provider::GoogleAiMode;
            }
            *self.inner.syncing_ui.borrow_mut() = true;
            self.inner.provider_dropdown.set_selected(p.index());
            *self.inner.syncing_ui.borrow_mut() = false;
        } else if !self.inner.window.is_visible() {
            self.reset_provider_to_default();
        }
    }

    pub fn focus_search(&self) {
        self.inner.search_entry.grab_focus();
        self.inner.search_entry.select_region(0, -1);
    }

    pub fn present_and_focus(&self, provider_override: Option<Provider>) {
        self.apply_provider_for_present(provider_override);
        self.inner.window.present();
        self.focus_search();
    }

    pub fn submit_query(&self, query: &str, provider_override: Option<Provider>) {
        let raw_trimmed = query.trim();
        if raw_trimmed.is_empty() {
            self.present_and_focus(provider_override);
            return;
        }

        // Optional quick prefix routing: "!..." or "/agy ..." -> Antigravity
        let (trimmed, effective_override) = if provider_override.is_none() {
            if let Some(rest) = raw_trimmed.strip_prefix('!') {
                (rest.trim(), Some(Provider::Antigravity))
            } else if let Some(rest) = raw_trimmed.strip_prefix("/agy ") {
                (rest.trim(), Some(Provider::Antigravity))
            } else {
                (raw_trimmed, None)
            }
        } else {
            (raw_trimmed, provider_override)
        };

        if trimmed.is_empty() {
            self.present_and_focus(effective_override);
            return;
        }

        let provider = match effective_override {
            Some(p) => {
                if !self.inner.has_antigravity && p == Provider::Antigravity {
                    self.inner.search_entry.set_placeholder_text(Some(
                        crate::i18n::tr(
                            "Antigravity CLI (agy) not installed",
                            "Antigravity CLI (agy) non installé",
                        ),
                    ));
                    self.inner.search_entry.grab_focus();
                    return;
                }
                *self.inner.syncing_ui.borrow_mut() = true;
                self.inner.provider_dropdown.set_selected(p.index());
                *self.inner.syncing_ui.borrow_mut() = false;
                p
            }
            None => Provider::from_index(self.inner.provider_dropdown.selected()),
        };

        let generation = self.inner.query_generation.get().wrapping_add(1);
        self.inner.query_generation.set(generation);
        self.inner.awaiting_captcha.set(false);
        log_info!(
            "query",
            "Submitting query (provider={provider:?}, len={}, gen={generation})",
            trimmed.chars().count()
        );

        self.inner.search_entry.set_text(trimmed);
        self.expand_results_area();

        if provider == Provider::GoogleAiMode || provider == Provider::Antigravity {
            self.inner
                .web_toggle
                .set_visible(provider == Provider::GoogleAiMode);
            self.inner.followup_box.set_visible(true);
            self.set_web_view_mode(false);

            while let Some(child) = self.inner.messages_box.first_child() {
                self.inner.messages_box.remove(&child);
            }
            *self.inner.last_plain_text.borrow_mut() = String::new();
            *self.inner.antigravity_conv_id.borrow_mut() = None;
            {
                let mut queries = self.inner.conversation_queries.borrow_mut();
                queries.clear();
                queries.push(trimmed.to_string());
            }
            self.append_turn_widgets(trimmed);
        } else {
            self.inner.web_toggle.set_visible(false);
            self.inner.followup_box.set_visible(false);
            self.set_web_view_mode(true);
        }

        if provider == Provider::Gemini {
            *self.inner.pending_gemini_prompt.borrow_mut() = Some(trimmed.to_string());
        } else {
            *self.inner.pending_gemini_prompt.borrow_mut() = None;
        }

        self.inner.window.present();

        if provider == Provider::Antigravity {
            self.start_antigravity_turn(trimmed, None, generation);
            return;
        }

        let url = provider.query_url(trimmed);
        self.inner.webview.load_uri(&url);

        if provider == Provider::GoogleAiMode {
            self.start_ai_mode_polling(generation, None);
        }
    }

    pub fn submit_followup(&self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let provider = Provider::from_index(self.inner.provider_dropdown.selected());
        if provider != Provider::GoogleAiMode && provider != Provider::Antigravity {
            return;
        }

        let generation = self.inner.query_generation.get().wrapping_add(1);
        self.inner.query_generation.set(generation);
        self.inner.awaiting_captcha.set(false);
        log_info!(
            "query",
            "Submitting follow-up (provider={provider:?}, len={}, gen={generation})",
            trimmed.chars().count()
        );

        self.inner.followup_entry.set_text("");
        self.finish_active_loading();
        if let Some(prev_sug) = self.inner.active_suggestions_box.borrow().as_ref() {
            prev_sug.set_visible(false);
        }
        self.inner
            .conversation_queries
            .borrow_mut()
            .push(trimmed.to_string());
        self.append_turn_widgets(trimmed);

        if provider == Provider::Antigravity {
            let conv_id = self.inner.antigravity_conv_id.borrow().clone();
            self.start_antigravity_turn(trimmed, conv_id, generation);
            return;
        }

        let prev_plain = self.inner.last_plain_text.borrow().clone();
        let js = ai_mode_followup_js(trimmed);
        self.inner
            .webview
            .evaluate_javascript(&js, None, None, None::<&gio::Cancellable>, |_| {});

        self.start_ai_mode_polling(
            generation,
            if prev_plain.is_empty() {
                None
            } else {
                Some(prev_plain)
            },
        );
    }

    fn start_antigravity_turn(&self, prompt: &str, conv_id: Option<String>, generation: u64) {
        let cfg = self.inner.config.borrow().clone();
        let rx = antigravity::spawn_turn(
            prompt,
            conv_id,
            cfg.antigravity_auto_approve,
            cfg.antigravity_model,
        );

        let ctrl = self.clone();
        let accumulated = Rc::new(RefCell::new(String::new()));
        let tools_expander = self.inner.active_tools_expander.borrow().clone();
        let tools_box = self.inner.active_tools_box.borrow().clone();
        let step_widgets: Rc<RefCell<std::collections::HashMap<u64, (gtk4::Expander, gtk4::Label)>>> =
            Rc::new(RefCell::new(std::collections::HashMap::new()));
        let active_steps: Rc<RefCell<std::collections::HashSet<u64>>> =
            Rc::new(RefCell::new(std::collections::HashSet::new()));

        glib::timeout_add_local(Duration::from_millis(50), move || {
            if ctrl.inner.query_generation.get() != generation {
                return glib::ControlFlow::Break;
            }

            loop {
                match rx.try_recv() {
                    Ok(AgyEvent::Init(id)) => {
                        log_info!("antigravity", "Turn initialized conv_id={id}");
                        *ctrl.inner.antigravity_conv_id.borrow_mut() = Some(id);
                    }
                    Ok(AgyEvent::ToolStep(step)) => {
                        log_info!(
                            "antigravity",
                            "Tool step #{} [{}] {}: {}",
                            step.step_index,
                            step.state,
                            step.tool_name,
                            step.summary
                        );
                        if let Some(lbl) = ctrl.inner.active_loading_label.borrow().as_ref() {
                            lbl.set_text(&format!("⚙ {}…", step.summary));
                        }
                        if let (Some(outer_exp), Some(list_box)) =
                            (tools_expander.as_ref(), tools_box.as_ref())
                        {
                            outer_exp.set_visible(true);
                            let icon = if step.state == "ACTIVE" {
                                active_steps.borrow_mut().insert(step.step_index);
                                "⏳"
                            } else {
                                active_steps.borrow_mut().remove(&step.step_index);
                                if step.state.contains("ERROR") || step.state.contains("FAIL") {
                                    "✗"
                                } else {
                                    "✓"
                                }
                            };
                            let dur = step
                                .duration_seconds
                                .map(|d| format!(" ({d:.2}s)"))
                                .unwrap_or_default();
                            let step_title = format!("{icon} {}{dur}", step.summary);
                            let detail_text = if step.detail.trim().is_empty() {
                                crate::i18n::tr(
                                    "No additional details",
                                    "Aucun détail supplémentaire",
                                )
                                .to_string()
                            } else {
                                step.detail.clone()
                            };

                            let mut map = step_widgets.borrow_mut();
                            if let Some((step_exp, detail_lbl)) = map.get(&step.step_index) {
                                step_exp.set_label(Some(&step_title));
                                detail_lbl.set_text(&detail_text);
                            } else {
                                let detail_lbl = gtk4::Label::builder()
                                    .label(&detail_text)
                                    .selectable(true)
                                    .wrap(true)
                                    .wrap_mode(gtk4::pango::WrapMode::WordChar)
                                    .xalign(0.0)
                                    .build();
                                detail_lbl.add_css_class("spotlight-tool-detail");

                                let detail_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
                                detail_box.add_css_class("spotlight-tool-detail-box");
                                detail_box.append(&detail_lbl);

                                let step_exp = gtk4::Expander::builder()
                                    .label(&step_title)
                                    .child(&detail_box)
                                    .expanded(false)
                                    .build();
                                step_exp.add_css_class("spotlight-tool-step-expander");

                                list_box.append(&step_exp);
                                map.insert(step.step_index, (step_exp, detail_lbl));
                            }

                            let total = map.len();
                            let noun = if total == 1 { "action" } else { "actions" };
                            let outer_label = if active_steps.borrow().is_empty() {
                                let done_word = if total == 1 {
                                    crate::i18n::tr("completed", "exécutée")
                                } else {
                                    crate::i18n::tr("completed", "exécutées")
                                };
                                format!("⚙ {total} {noun} {done_word}")
                            } else {
                                format!(
                                    "⚙ {total} {noun} {}",
                                    crate::i18n::tr("running…", "en cours…")
                                )
                            };
                            outer_exp.set_label(Some(&outer_label));
                        }
                    }
                    Ok(AgyEvent::TextDelta(delta)) => {
                        let mut acc = accumulated.borrow_mut();
                        acc.push_str(&delta);
                        let pango = antigravity::markdown_to_pango(acc.trim());
                        if let Some(ai_label) = ctrl.inner.active_ai_label.borrow().as_ref() {
                            ai_label.set_visible(true);
                            ai_label.set_markup(&pango);
                        }
                        ctrl.scroll_to_bottom();
                    }
                    Ok(AgyEvent::Done {
                        response,
                        conversation_id,
                    }) => {
                        if let Some(id) = conversation_id {
                            *ctrl.inner.antigravity_conv_id.borrow_mut() = Some(id);
                        }
                        let final_md = {
                            let acc = accumulated.borrow();
                            if acc.trim().is_empty() {
                                response
                            } else {
                                acc.clone()
                            }
                        };
                        let trimmed_final = final_md.trim().to_string();
                        if !trimmed_final.is_empty() {
                            let pango = antigravity::markdown_to_pango(&trimmed_final);
                            if let Some(ai_label) = ctrl.inner.active_ai_label.borrow().as_ref() {
                                ai_label.set_visible(true);
                                ai_label.set_markup(&pango);
                            }
                            if let Some(turn_plain) =
                                ctrl.inner.active_turn_plain_text.borrow().as_ref()
                            {
                                *turn_plain.borrow_mut() = trimmed_final.clone();
                            }
                            *ctrl.inner.last_plain_text.borrow_mut() = trimmed_final;
                        }
                        let total = step_widgets.borrow().len();
                        if total > 0
                            && let Some(outer_exp) = tools_expander.as_ref()
                        {
                            let noun = if total == 1 { "action" } else { "actions" };
                            let done_word = if total == 1 {
                                crate::i18n::tr("completed", "exécutée")
                            } else {
                                crate::i18n::tr("completed", "exécutées")
                            };
                            outer_exp.set_label(Some(&format!("⚙ {total} {noun} {done_word}")));
                        }
                        log_info!(
                            "antigravity",
                            "Turn done (response_len={}, tools={})",
                            ctrl.inner.last_plain_text.borrow().chars().count(),
                            step_widgets.borrow().len()
                        );
                        ctrl.finish_active_loading();
                        ctrl.scroll_to_bottom();
                        if let Some(path) = ctrl.inner.screenshot_path.borrow_mut().take() {
                            let ctrl_s = ctrl.clone();
                            glib::timeout_add_local_once(Duration::from_millis(250), move || {
                                ctrl_s.capture_screenshot_and_quit(&path);
                            });
                        }
                        return glib::ControlFlow::Break;
                    }
                    Ok(AgyEvent::Error(err)) => {
                        log_err!("antigravity", "Turn error: {err}");
                        let total = step_widgets.borrow().len();
                        if total > 0
                            && let Some(outer_exp) = tools_expander.as_ref()
                        {
                            let noun = if total == 1 { "action" } else { "actions" };
                            let done_word = if total == 1 {
                                crate::i18n::tr("completed", "exécutée")
                            } else {
                                crate::i18n::tr("completed", "exécutées")
                            };
                            outer_exp.set_label(Some(&format!("⚙ {total} {noun} {done_word}")));
                        }
                        if let Some(ai_label) = ctrl.inner.active_ai_label.borrow().as_ref() {
                            ai_label.set_visible(true);
                            ai_label.set_text(&format!("Erreur Antigravity : {err}"));
                        }
                        ctrl.finish_active_loading();
                        if let Some(path) = ctrl.inner.screenshot_path.borrow_mut().take() {
                            ctrl.capture_screenshot_and_quit(&path);
                        }
                        return glib::ControlFlow::Break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        log_warn!("antigravity", "Event channel disconnected");
                        ctrl.finish_active_loading();
                        return glib::ControlFlow::Break;
                    }
                }
            }

            glib::ControlFlow::Continue
        });
    }

    pub fn submit_from_clipboard(&self) {
        self.inner.window.present();
        let clipboard = self.inner.window.clipboard();
        let ctrl = self.clone();
        clipboard.read_text_async(None::<&gio::Cancellable>, move |res| {
            if let Ok(Some(text)) = res {
                let text_str = text.to_string();
                if !text_str.trim().is_empty() {
                    ctrl.submit_query(&text_str, None);
                } else {
                    ctrl.focus_search();
                }
            } else {
                ctrl.focus_search();
            }
        });
    }

    pub fn open_login(&self) {
        let generation = self.inner.query_generation.get().wrapping_add(1);
        self.inner.query_generation.set(generation);
        self.inner.awaiting_captcha.set(false);
        log_info!("webview", "Opening Google login (gen={generation})");
        *self.inner.pending_gemini_prompt.borrow_mut() = None;
        self.expand_results_area();
        self.inner.web_toggle.set_visible(false);
        self.inner.followup_box.set_visible(false);
        self.set_web_view_mode(true);
        self.inner.webview.load_uri(GOOGLE_LOGIN_URL);
        self.inner.window.present();
    }

    pub fn dismiss(&self) {
        if self.inner.config.borrow().background_mode {
            self.inner.window.set_visible(false);
        } else {
            self.quit();
        }
    }

    pub fn quit(&self) {
        if let Some(recorder) = self.inner.voice_recorder.borrow_mut().take() {
            recorder.cancel();
        }
        *self.inner.hold_guard.borrow_mut() = None;
        self.inner.app.quit();
    }
}
