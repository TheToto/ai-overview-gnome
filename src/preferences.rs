use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gdk, glib};
use libadwaita as adw;

use crate::config::{ANTIGRAVITY_MODELS, SpeechToText};
use crate::i18n::tr;
use crate::provider::Provider;
use crate::shortcut::{
    ShortcutKind, build_shortcut_command, open_gnome_keyboard_settings, register_gnome_shortcut,
    set_gnome_shortcut_binding,
};
use crate::window::SpotlightController;

fn build_shortcut_provider_model(has_agy: bool) -> (gtk4::StringList, Vec<Option<Provider>>) {
    let mut labels = vec![tr("Default", "Par défaut")];
    let mut values = vec![None];
    for &p in Provider::available(has_agy) {
        labels.push(p.label());
        values.push(Some(p));
    }
    (gtk4::StringList::new(&labels), values)
}

fn provider_option_index(values: &[Option<Provider>], target: Option<Provider>) -> u32 {
    values
        .iter()
        .position(|&p| p == target)
        .unwrap_or(0) as u32
}

pub fn present_preferences(ctrl: &SpotlightController) {
    let cfg = ctrl.config_snapshot();
    let has_agy = crate::antigravity::is_available();

    let dialog = adw::PreferencesDialog::builder()
        .title(tr("Preferences", "Paramètres"))
        .build();

    let page = adw::PreferencesPage::new();

    // 1. General
    let general_group = adw::PreferencesGroup::builder()
        .title(tr("General", "Général"))
        .build();

    let provider_labels: Vec<&str> = Provider::available(has_agy)
        .iter()
        .map(|p| p.label())
        .collect();
    let provider_model = gtk4::StringList::new(&provider_labels);
    let initial_provider_idx = if !has_agy && cfg.provider == Provider::Antigravity {
        0
    } else {
        cfg.provider.index()
    };
    let provider_row = adw::ComboRow::builder()
        .title(tr("Default AI Provider", "Moteur IA par défaut"))
        .model(&provider_model)
        .selected(initial_provider_idx)
        .build();

    let bg_row = adw::SwitchRow::builder()
        .title(tr("Keep running in background", "Garder en arrière-plan"))
        .subtitle(tr(
            "Esc hides the window instead of quitting",
            "Échap masque la fenêtre au lieu de quitter",
        ))
        .active(cfg.background_mode)
        .build();

    let temp_chat_row = adw::SwitchRow::builder()
        .title(tr("Gemini temporary chat", "Discussion temporaire Gemini"))
        .subtitle(tr(
            "Automatically opens Gemini in an ephemeral chat",
            "Ouvre Gemini en chat éphémère automatiquement",
        ))
        .active(cfg.gemini_temporary_chat)
        .build();

    let agy_auto_row = adw::SwitchRow::builder()
        .title(tr(
            "Auto-approve (Antigravity)",
            "Exécution automatique (Antigravity)",
        ))
        .subtitle(if has_agy {
            tr(
                "Allows small desktop actions without confirmation",
                "Autorise les petites actions bureau sans confirmation",
            )
        } else {
            tr(
                "Antigravity CLI (agy) not detected",
                "Antigravity CLI (agy) non détecté",
            )
        })
        .active(cfg.antigravity_auto_approve)
        .build();
    agy_auto_row.set_sensitive(has_agy);

    let agy_model_labels: Vec<&str> = ANTIGRAVITY_MODELS.iter().map(|(_, label)| *label).collect();
    let agy_model_list = gtk4::StringList::new(&agy_model_labels);
    let selected_agy_idx = cfg
        .antigravity_model
        .as_deref()
        .and_then(|id| ANTIGRAVITY_MODELS.iter().position(|(m, _)| *m == id))
        .unwrap_or(0) as u32;
    let agy_model_row = adw::ComboRow::builder()
        .title(tr("Antigravity model", "Modèle Antigravity"))
        .model(&agy_model_list)
        .selected(selected_agy_idx)
        .build();
    if !has_agy {
        agy_model_row.set_subtitle(tr(
            "Install agy to enable this mode",
            "Installez agy pour activer ce mode",
        ));
    }
    agy_model_row.set_sensitive(has_agy);

    general_group.add(&provider_row);
    general_group.add(&bg_row);
    general_group.add(&temp_chat_row);
    general_group.add(&agy_auto_row);
    general_group.add(&agy_model_row);

    // 2. Speech-to-Text
    let stt_group = adw::PreferencesGroup::builder()
        .title(tr("Voice dictation", "Dictée vocale"))
        .description(tr(
            "Triggered automatically when launched with --voice",
            "Déclenchée automatiquement lors d'un lancement avec --voice",
        ))
        .build();

    let stt_labels: Vec<&str> = SpeechToText::all().iter().map(|s| s.label()).collect();
    let stt_model = gtk4::StringList::new(&stt_labels);
    let stt_row = adw::ComboRow::builder()
        .title(tr("Dictation backend", "Logiciel de dictée"))
        .model(&stt_model)
        .selected(cfg.speech_to_text.index())
        .build();

    stt_group.add(&stt_row);

    // 3. GNOME Keyboard Shortcuts
    let shortcut_group = adw::PreferencesGroup::builder()
        .title(tr("Keyboard Shortcuts (GNOME)", "Raccourcis clavier (GNOME)"))
        .description(tr(
            "Saved in Settings → Keyboard → Custom Shortcuts",
            "Enregistrés dans Paramètres → Clavier → Raccourcis personnalisés",
        ))
        .build();

    let (normal_provider_model, normal_provider_values) = build_shortcut_provider_model(has_agy);
    let (voice_provider_model, voice_provider_values) = build_shortcut_provider_model(has_agy);

    let normal_shortcut_label = gtk4::ShortcutLabel::new(&cfg.gnome_shortcut);
    normal_shortcut_label.set_valign(gtk4::Align::Center);
    normal_shortcut_label.set_disabled_text(tr("None", "Aucun"));

    let normal_shortcut_row = adw::ActionRow::builder()
        .title(tr("Shortcut (Normal mode)", "Raccourci (Mode normal)"))
        .subtitle(tr(
            "Opens Spotlight with keyboard input",
            "Ouvre Spotlight avec saisie clavier",
        ))
        .activatable(true)
        .build();
    normal_shortcut_row.add_suffix(&normal_shortcut_label);
    normal_shortcut_row.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));

    let normal_provider_row = adw::ComboRow::builder()
        .title(tr(
            "Provider on launch (normal)",
            "Mode à l'ouverture (normal)",
        ))
        .model(&normal_provider_model)
        .selected(provider_option_index(
            &normal_provider_values,
            cfg.gnome_shortcut_provider,
        ))
        .build();

    let voice_shortcut_label = gtk4::ShortcutLabel::new(&cfg.gnome_voice_shortcut);
    voice_shortcut_label.set_valign(gtk4::Align::Center);
    voice_shortcut_label.set_disabled_text(tr("None", "Aucun"));

    let voice_shortcut_row = adw::ActionRow::builder()
        .title(tr("Shortcut (Voice mode)", "Raccourci (Mode vocal)"))
        .subtitle(tr(
            "Opens Spotlight and starts voice dictation immediately",
            "Ouvre Spotlight et lance directement la dictée vocale",
        ))
        .activatable(true)
        .build();
    voice_shortcut_row.add_suffix(&voice_shortcut_label);
    voice_shortcut_row.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));

    let voice_provider_row = adw::ComboRow::builder()
        .title(tr(
            "Provider on launch (voice)",
            "Mode à l'ouverture (vocal)",
        ))
        .model(&voice_provider_model)
        .selected(provider_option_index(
            &voice_provider_values,
            cfg.gnome_voice_shortcut_provider,
        ))
        .build();

    let open_gnome_row = adw::ActionRow::builder()
        .title(tr("GNOME Keyboard Settings", "Paramètres Clavier GNOME"))
        .subtitle(tr(
            "Review or edit in system settings",
            "Vérifier ou modifier dans les réglages système",
        ))
        .activatable(true)
        .build();
    open_gnome_row.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));

    shortcut_group.add(&normal_shortcut_row);
    shortcut_group.add(&normal_provider_row);
    shortcut_group.add(&voice_shortcut_row);
    shortcut_group.add(&voice_provider_row);
    shortcut_group.add(&open_gnome_row);

    page.add(&general_group);
    page.add(&stt_group);
    page.add(&shortcut_group);
    dialog.add(&page);

    // Wire signals
    let ctrl_p = ctrl.clone();
    provider_row.connect_selected_notify(move |row| {
        ctrl_p.set_default_provider(Provider::from_index(row.selected()));
    });

    let ctrl_b = ctrl.clone();
    bg_row.connect_active_notify(move |row| {
        ctrl_b.set_background_mode(row.is_active(), true);
    });

    let ctrl_t = ctrl.clone();
    temp_chat_row.connect_active_notify(move |row| {
        ctrl_t.set_gemini_temporary_chat(row.is_active());
    });

    let ctrl_agy = ctrl.clone();
    agy_auto_row.connect_active_notify(move |row| {
        ctrl_agy.set_antigravity_auto_approve(row.is_active());
    });

    let ctrl_agy_m = ctrl.clone();
    agy_model_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = ANTIGRAVITY_MODELS.get(row.selected() as usize) {
            ctrl_agy_m.set_antigravity_model(Some((*id).to_string()));
        }
    });

    let ctrl_s = ctrl.clone();
    stt_row.connect_selected_notify(move |row| {
        let stt = SpeechToText::from_index(row.selected());
        ctrl_s.set_speech_to_text(stt);
    });

    let ctrl_n = ctrl.clone();
    let dlg_n = dialog.clone();
    let label_n = normal_shortcut_label.clone();
    normal_shortcut_row.connect_activated(move |_| {
        present_shortcut_capture(&ctrl_n, &dlg_n, &label_n, ShortcutKind::Normal);
    });

    let ctrl_np = ctrl.clone();
    let dlg_np = dialog.clone();
    normal_provider_row.connect_selected_notify(move |row| {
        let chosen = normal_provider_values
            .get(row.selected() as usize)
            .copied()
            .flatten();
        ctrl_np.set_gnome_shortcut_provider(chosen);
        let binding = ctrl_np.config_snapshot().gnome_shortcut;
        if !binding.trim().is_empty() {
            let cmd = build_shortcut_command(ShortcutKind::Normal, chosen);
            if register_gnome_shortcut(ShortcutKind::Normal, &binding, &cmd).is_ok() {
                dlg_np.add_toast(adw::Toast::new(tr(
                    "Normal shortcut updated",
                    "Raccourci normal mis à jour",
                )));
            }
        }
    });

    let ctrl_v = ctrl.clone();
    let dlg_v = dialog.clone();
    let label_v = voice_shortcut_label.clone();
    voice_shortcut_row.connect_activated(move |_| {
        present_shortcut_capture(&ctrl_v, &dlg_v, &label_v, ShortcutKind::Voice);
    });

    let ctrl_vp = ctrl.clone();
    let dlg_vp = dialog.clone();
    voice_provider_row.connect_selected_notify(move |row| {
        let chosen = voice_provider_values
            .get(row.selected() as usize)
            .copied()
            .flatten();
        ctrl_vp.set_gnome_voice_shortcut_provider(chosen);
        let binding = ctrl_vp.config_snapshot().gnome_voice_shortcut;
        if !binding.trim().is_empty() {
            let cmd = build_shortcut_command(ShortcutKind::Voice, chosen);
            if register_gnome_shortcut(ShortcutKind::Voice, &binding, &cmd).is_ok() {
                dlg_vp.add_toast(adw::Toast::new(tr(
                    "Voice shortcut updated",
                    "Raccourci vocal mis à jour",
                )));
            }
        }
    });

    let dlg_k = dialog.clone();
    open_gnome_row.connect_activated(move |_| {
        if let Err(err) = open_gnome_keyboard_settings() {
            dlg_k.add_toast(adw::Toast::new(&err));
        }
    });

    dialog.present(Some(&ctrl.window()));
}

fn present_shortcut_capture(
    ctrl: &SpotlightController,
    parent_dialog: &adw::PreferencesDialog,
    shortcut_label: &gtk4::ShortcutLabel,
    kind: ShortcutKind,
) {
    let title = match kind {
        ShortcutKind::Normal => tr("Shortcut (Normal mode)", "Raccourci (Mode normal)"),
        ShortcutKind::Voice => tr("Shortcut (Voice mode)", "Raccourci (Mode vocal)"),
    };

    let icon = gtk4::Image::builder()
        .icon_name("preferences-desktop-keyboard-shortcuts-symbolic")
        .pixel_size(48)
        .build();
    let heading = gtk4::Label::builder()
        .label(tr("Press shortcut", "Appuyez sur le raccourci"))
        .build();
    heading.add_css_class("title-2");
    let desc = gtk4::Label::builder()
        .label(tr(
            "Press the desired key combination\n(Esc to cancel)",
            "Appuyez sur la combinaison souhaitée\n(Échap pour annuler)",
        ))
        .justify(gtk4::Justification::Center)
        .wrap(true)
        .build();
    desc.add_css_class("dim-label");

    let content_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(12)
        .margin_top(24)
        .margin_bottom(28)
        .margin_start(24)
        .margin_end(24)
        .halign(gtk4::Align::Center)
        .valign(gtk4::Align::Center)
        .build();
    content_box.append(&icon);
    content_box.append(&heading);
    content_box.append(&desc);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&adw::HeaderBar::new());
    toolbar_view.set_content(Some(&content_box));

    let capture_dialog = adw::Dialog::builder()
        .title(title)
        .content_width(360)
        .child(&toolbar_view)
        .build();

    let cfg_snapshot = ctrl.config_snapshot();
    let previous_binding = match kind {
        ShortcutKind::Normal => cfg_snapshot.gnome_shortcut,
        ShortcutKind::Voice => cfg_snapshot.gnome_voice_shortcut,
    };
    if !previous_binding.trim().is_empty() {
        let _ = set_gnome_shortcut_binding(kind, "");
    }
    let saved = Rc::new(Cell::new(false));

    let key_ctrl = gtk4::EventControllerKey::new();
    key_ctrl.set_propagation_phase(gtk4::PropagationPhase::Capture);

    let ctrl_c = ctrl.clone();
    let parent_c = parent_dialog.clone();
    let label_c = shortcut_label.clone();
    let dlg_c = capture_dialog.clone();
    let saved_c = saved.clone();

    key_ctrl.connect_key_pressed(move |_, key, _, state| {
        let mods = state & gtk4::accelerator_get_default_mod_mask();

        if key == gdk::Key::Escape && mods.is_empty() {
            dlg_c.close();
            return glib::Propagation::Stop;
        }

        if is_modifier_key(key) {
            return glib::Propagation::Stop;
        }

        let accel = gtk4::accelerator_name(key, mods).to_string();
        if accel.is_empty() {
            return glib::Propagation::Stop;
        }

        let cfg = ctrl_c.config_snapshot();
        let provider = match kind {
            ShortcutKind::Normal => cfg.gnome_shortcut_provider,
            ShortcutKind::Voice => cfg.gnome_voice_shortcut_provider,
        };
        let cmd = build_shortcut_command(kind, provider);

        match register_gnome_shortcut(kind, &accel, &cmd) {
            Ok(()) => {
                saved_c.set(true);
                match kind {
                    ShortcutKind::Normal => ctrl_c.set_gnome_shortcut(accel.clone()),
                    ShortcutKind::Voice => ctrl_c.set_gnome_voice_shortcut(accel.clone()),
                }
                label_c.set_accelerator(&accel);
                parent_c.add_toast(adw::Toast::new(tr(
                    "Shortcut saved in GNOME",
                    "Raccourci enregistré dans GNOME",
                )));
            }
            Err(err) => {
                parent_c.add_toast(adw::Toast::new(&format!(
                    "{}: {err}",
                    tr("Error", "Erreur")
                )));
            }
        }

        dlg_c.close();
        glib::Propagation::Stop
    });

    let prev_restore = previous_binding.clone();
    let saved_restore = saved.clone();
    capture_dialog.connect_closed(move |_| {
        if !saved_restore.get() && !prev_restore.trim().is_empty() {
            let _ = set_gnome_shortcut_binding(kind, &prev_restore);
        }
    });

    capture_dialog.add_controller(key_ctrl);
    capture_dialog.present(Some(parent_dialog));
}

fn is_modifier_key(key: gdk::Key) -> bool {
    matches!(
        key,
        gdk::Key::Shift_L
            | gdk::Key::Shift_R
            | gdk::Key::Control_L
            | gdk::Key::Control_R
            | gdk::Key::Alt_L
            | gdk::Key::Alt_R
            | gdk::Key::Super_L
            | gdk::Key::Super_R
            | gdk::Key::Meta_L
            | gdk::Key::Meta_R
            | gdk::Key::Hyper_L
            | gdk::Key::Hyper_R
            | gdk::Key::ISO_Level3_Shift
            | gdk::Key::Caps_Lock
            | gdk::Key::Num_Lock
    )
}
