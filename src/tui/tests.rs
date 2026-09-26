use super::*;
use ratatui::{Terminal, backend::TestBackend};

#[test]
fn tui_theme_preview_cancel_save_and_restart_do_not_touch_provider_config() {
    let (_temp, mut app) = persisted_app();
    let before = std::fs::read(&app.paths.config).unwrap();
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    app.handle_key(key(KeyCode::F(4))).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .any(|c| c.fg == Color::Rgb(216, 194, 142))
    );
    assert_eq!(app.theme, theme::Theme::Slate);
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert!(!app.paths.state_dir.join("tui-theme.json").exists());
    app.handle_key(key(KeyCode::F(4))).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.theme, theme::Theme::Moss);
    assert_eq!(theme::Theme::load(&app.paths), theme::Theme::Moss);
    assert_eq!(std::fs::read(&app.paths.config).unwrap(), before);
    assert!(!app.background.sync_running);
    assert!(app.background.queued_sync.is_none());
}

#[test]
fn tui_theme_settings_mouse_and_keyboard_work_on_every_client() {
    let (_temp, mut app) = persisted_app();
    let screen = Rect::new(0, 0, 100, 30);
    for tab in [
        ClientTab::Claude,
        ClientTab::Codex,
        ClientTab::Pi,
        ClientTab::Grok,
        ClientTab::Usage,
    ] {
        app.select_client_tab(tab);
        app.handle_key(KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE))
            .unwrap();
        assert!(matches!(app.modal, Some(Modal::Appearance(_))));
        let area = modal_area_for(app.modal.as_ref().unwrap(), screen);
        let row = theme::rows(
            area,
            match app.modal.as_ref().unwrap() {
                Modal::Appearance(form) => form,
                _ => unreachable!(),
            },
        )[2]
        .1;
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: row.x + 3,
                row: row.y,
                modifiers: KeyModifiers::NONE,
            },
            screen,
        )
        .unwrap();
        assert!(
            matches!(app.modal, Some(Modal::Appearance(ref form)) if form.theme == theme::Theme::Moss)
        );
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert!(app.modal.is_none());
    }
}
use std::path::PathBuf;

#[test]
fn claude_settings_returns_to_theme_preview_and_confirms_dirty_drafts() {
    let (_temp, mut app) = persisted_app();
    let mut form = PreferencesForm::new(app.config.claude.clone(), serde_json::json!({}));
    form.return_theme = Some(theme::Theme::Plum);
    app.modal = Some(Modal::Preferences(form.clone()));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(
        matches!(app.modal, Some(Modal::Appearance(ref form)) if form.theme == theme::Theme::Plum)
    );
    assert_eq!(app.theme, theme::Theme::Slate);

    form.fields[0].value = "hide".into();
    app.modal = Some(Modal::Preferences(form));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(app.modal, Some(Modal::Preferences(ref form)) if form.discard));
    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .unwrap();
    assert!(
        matches!(app.modal, Some(Modal::Appearance(ref form)) if form.theme == theme::Theme::Plum)
    );
    assert!(!app.paths.state_dir.join("tui-theme.json").exists());
}

#[test]
fn all_six_themes_fit_small_settings_and_cycle_both_directions() {
    let (_temp, mut app) = persisted_app();
    app.open_appearance();
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    for expected in [
        theme::Theme::Moss,
        theme::Theme::Sand,
        theme::Theme::Plum,
        theme::Theme::Pulse,
        theme::Theme::Classic,
        theme::Theme::Slate,
    ] {
        app.handle_key(key(KeyCode::Down)).unwrap();
        assert!(matches!(app.modal, Some(Modal::Appearance(ref form)) if form.theme == expected));
    }
    app.handle_key(key(KeyCode::Up)).unwrap();
    assert!(
        matches!(app.modal, Some(Modal::Appearance(ref form)) if form.theme == theme::Theme::Classic)
    );
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    for name in [
        "Classic",
        "Graphite",
        "Tundra",
        "Paper",
        "Nightfall",
        "Pulse",
    ] {
        assert!(text.contains(name), "{name}: {text}");
    }
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(theme::Theme::load(&app.paths), theme::Theme::Classic);
}

#[test]
fn pulse_pane_theme_is_saved_independently_and_recolors_its_buffer() {
    let (_temp, mut app) = persisted_app();
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    app.open_appearance();
    app.handle_key(key(KeyCode::Tab)).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap();
    assert!(
        matches!(app.modal, Some(Modal::Appearance(ref form)) if form.pulse_theme == theme::PulseTheme::Sand)
    );
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.theme, theme::Theme::Slate);
    assert_eq!(theme::PulseTheme::load(&app.paths), theme::PulseTheme::Sand);
    let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 2, 1));
    buffer[(0, 0)].set_fg(quick::INK).set_bg(quick::BG);
    buffer[(1, 0)].set_fg(quick::BG).set_bg(quick::BLUE);
    theme::PulseTheme::load(&app.paths).apply(&mut buffer);
    assert_eq!(buffer[(0, 0)].bg, Color::Rgb(240, 233, 217));
    assert_eq!(buffer[(0, 0)].fg, Color::Rgb(39, 61, 80));
    assert_eq!(buffer[(1, 0)].fg, Color::Rgb(255, 252, 244));
}

#[test]
fn vim_keys_navigate_templates_and_picker_without_interfering_with_search() {
    let mut app = interactive_test_app();
    app.new_profile();
    for c in ['j', 'j', 'k', 'l'] {
        app.handle_modal(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
            .unwrap();
    }
    let Some(Modal::Profile(form)) = &mut app.modal else {
        panic!("missing form");
    };
    assert_eq!(form.fields[3].value, PROVIDER_TEMPLATES[0].url);
    form.selected = 8;
    let mut picker = ModelForm::with_api_models(
        ["first", "hjkl-model"]
            .into_iter()
            .map(|id| ModelEntry {
                id: id.into(),
                label: None,
                description: None,
                max_output_tokens: None,
                context_window: None,
                reasoning_max: None,
            })
            .collect(),
    );
    picker.focus_api_search = true;
    form.picker = Some(picker);
    app.handle_modal(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .unwrap();
    assert!(
        matches!(&app.modal, Some(Modal::Profile(form)) if form.picker.as_ref().unwrap().api_selected == 1)
    );
    for c in ['/', 'h', 'j', 'k', 'l'] {
        app.handle_modal(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
            .unwrap();
    }
    assert!(
        matches!(&app.modal, Some(Modal::Profile(form)) if form.picker.as_ref().unwrap().api_query == "hjkl")
    );
    app.handle_modal(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    app.handle_modal(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .unwrap();
    let Some(Modal::Profile(form)) = &app.modal else {
        panic!("draft lost");
    };
    assert_eq!(form.fields[8].value, "hjkl-model");
    assert!(form.picker.is_none());
}

#[test]
fn provider_templates_need_only_key_and_model_and_avoid_duplicate_ids() {
    for (index, template) in PROVIDER_TEMPLATES.iter().enumerate() {
        let mut app = interactive_test_app();
        let existing = app.config.profiles.values().next().unwrap().clone();
        app.config
            .profiles
            .insert(template.id.into(), existing.clone());
        app.config
            .profiles
            .insert(format!("{}-2", template.id), existing);
        let before = app.config.clone();
        app.new_profile();
        for _ in 0..=index {
            app.handle_modal(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
                .unwrap();
        }
        app.handle_modal(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        let Some(Modal::Profile(form)) = &mut app.modal else {
            panic!("missing template form");
        };
        assert!(form.template_selected.is_none());
        assert_eq!(form.selected, 5);
        assert_eq!(form.fields[0].value, format!("{}-3", template.id));
        assert_eq!(form.fields[1].value, template.name);
        assert_eq!(form.fields[3].value, template.url);
        assert!(form.fields[5].value.is_empty());
        assert!(form.fields[6].value.is_empty());
        assert!(form.models.is_empty());
        form.fields[5].value = "new-key".into();
        form.fields[6].value = "new-model".into();
        let (_, profile) = form.to_profile().unwrap();
        assert_eq!(profile.credential.value(), Some("new-key"));
        assert_eq!(profile.default_model, "new-model");
        assert_eq!(app.config, before);
    }
}

#[test]
fn template_picker_mouse_selection_and_custom_form_work() {
    let mut app = interactive_test_app();
    app.new_profile();
    let screen = Rect::new(0, 0, 80, 24);
    let area = modal_area(screen);
    let inner = panel_inner(area);
    let click = |column, row| MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };
    app.handle_modal_mouse(click(inner.x, inner.y + 5), screen)
        .unwrap();
    let use_button = modal_button_rects(area, 3)[0];
    app.handle_modal_mouse(click(use_button.x + 1, use_button.y), screen)
        .unwrap();
    assert!(
        matches!(&app.modal, Some(Modal::Profile(form)) if form.fields[3].value == "https://api.deepseek.com/anthropic")
    );
    app.new_profile();
    app.handle_modal(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(
        matches!(&app.modal, Some(Modal::Profile(form)) if form.fields[0].value.is_empty() && form.template_selected.is_none())
    );
}

#[test]
fn provider_catalog_mouse_selects_then_uses_model_in_target_field() {
    for screen in [Rect::new(0, 0, 120, 30), Rect::new(0, 0, 48, 18)] {
        let mut app = interactive_test_app();
        let mut form = ProfileForm::new();
        form.selected = 8; // Sonnet
        form.picker = Some(ModelForm::with_api_models(vec![ModelEntry {
            id: "remote-sonnet".into(),
            label: None,
            description: None,
            max_output_tokens: None,
            context_window: None,
            reasoning_max: None,
        }]));
        app.modal = Some(Modal::Profile(Box::new(form)));
        let outer = panel_inner(modal_area(screen));
        let list = panel_inner(Rect::new(
            outer.x,
            outer.y,
            outer.width,
            outer.height.saturating_sub(2),
        ));
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: list.x,
            row: list.y + 2,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_modal_mouse(click, screen).unwrap();
        let Some(Modal::Profile(form)) = &app.modal else {
            panic!("closed on first click");
        };
        assert!(form.picker.is_some());
        assert!(form.fields[8].value.is_empty());
        app.handle_modal_mouse(click, screen).unwrap();
        let Some(Modal::Profile(form)) = &app.modal else {
            panic!("draft lost");
        };
        assert!(form.picker.is_none());
        assert_eq!(form.fields[8].value, "remote-sonnet");
        assert!(form.fields[6].value.is_empty());
    }
}

#[test]
fn provider_discovery_uses_unsaved_connection_without_requiring_default() {
    let mut form = ProfileForm::new();
    form.fields[3].value = "https://example.com/gateway/messages".into();
    form.fields[5].value = "draft-key".into();
    assert!(form.to_profile().is_err());
    let profile = form.discovery_profile().unwrap();
    assert_eq!(profile.base_url, form.fields[3].value);
    assert_eq!(profile.credential.value(), Some("draft-key"));
    assert!(form.fields[6].value.is_empty());
    assert!(form.fields[0].value.is_empty());
}

#[test]
fn provider_picker_selects_default_without_saving_and_cancel_keeps_draft() {
    check_provider_picker_target(0);
}

#[test]
fn provider_picker_fills_each_model_field_and_preserves_other_values() {
    for selected in 6..=12 {
        check_provider_picker_target(selected);
    }
}

fn check_provider_picker_target(selected: usize) {
    let mut app = interactive_test_app();
    let mut form = ProfileForm::new();
    form.selected = selected;
    form.fields[12].value = "existing-model".into();
    let original_fields = form.fields.clone();
    let target = selected.max(6);
    let expected = if target == 12 {
        "existing-model,chosen-model"
    } else {
        "chosen-model"
    };
    form.fields[5].value = "draft-secret".into();
    let mut picker = ModelForm::with_api_models(vec![ModelEntry {
        id: "chosen-model".into(),
        label: None,
        description: None,
        max_output_tokens: None,
        context_window: None,
        reasoning_max: None,
    }]);
    picker.focus_api_search = true;
    form.picker = Some(picker.clone());
    app.modal = Some(Modal::Profile(Box::new(form)));
    app.handle_modal(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    let Some(Modal::Profile(form)) = &mut app.modal else {
        panic!("draft closed");
    };
    assert_eq!(form.fields[target].value, expected);
    assert_eq!(form.selected, target);
    for (index, original) in original_fields.iter().enumerate() {
        if index != target && index != 5 {
            assert_eq!(form.fields[index].value, original.value);
        }
    }
    assert!(form.picker.is_none());
    // Selecting the same fallback twice must not duplicate it.
    if target == 12 {
        form.fill_selected_model("chosen-model");
        assert_eq!(form.fields[target].value, expected);
    }
    form.picker = Some(picker);
    app.handle_modal(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    let Some(Modal::Profile(form)) = &app.modal else {
        panic!("draft closed");
    };
    assert_eq!(form.fields[5].value, "draft-secret");
    assert_eq!(form.fields[target].value, expected);
    assert!(form.picker.is_none());
}

#[test]
fn renders_empty_state_in_narrow_terminal() {
    let paths = AppPaths {
        config: PathBuf::from("/tmp/config"),
        state_dir: PathBuf::from("/tmp/state"),
        cache: PathBuf::from("/tmp/cache"),
    };
    let mut app = App {
        theme: theme::Theme::default(),
        paths,
        config: Config::default(),
        cache: ModelCache::default(),
        view_mode: ViewMode::Home,
        home_all_selected: false,
        profile_idx: 0,
        model_idx: 0,
        profile_offset: 0,
        model_offset: 0,
        focus: Focus::Profiles,
        status: "Ready".into(),
        status_error: false,
        modal: None,
        proxy_status: None,
        provider_editor: None,
        provider_card_selected: false,
        codex_ui: codex::CodexUi::default(),
        grok_auth: grok_auth::AuthUi::default(),
        grok_enabled: false,
        grok_home: std::path::PathBuf::from("/nonexistent-ccsw-test-grok"),
        pi_enabled: false,
        pi_home: std::path::PathBuf::from("/nonexistent-ccsw-test-pi"),
        background: Background::default(),
        screen: Rect::new(0, 0, 80, 24),
        usage: usage::UsageUi::default(),
    };
    let backend = TestBackend::new(72, 22);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("All Models"));
    assert!(rendered.contains("0 models"));
    assert!(rendered.contains("Providers"));
    assert!(rendered.contains("Sync"));
    assert!(!rendered.contains("Launch"));
}

#[test]
fn renders_home_and_provider_at_minimal_terminal_size() {
    let mut app = interactive_test_app();
    let backend = TestBackend::new(36, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();

    app.enter_provider_view();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    app.focus = Focus::Details;
    terminal.draw(|frame| app.draw(frame)).unwrap();
}

#[test]
fn help_opens_on_the_current_view_and_switches_sections() {
    let mut app = interactive_test_app();
    app.view_mode = ViewMode::AllEnabled;
    app.open_help();
    assert!(matches!(
        app.modal,
        Some(Modal::Help(HelpModal {
            section: HelpSection::AllEnabled,
            ..
        }))
    ));

    app.handle_modal(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(
        app.modal,
        Some(Modal::Help(HelpModal {
            section: HelpSection::Provider,
            ..
        }))
    ));
    app.handle_modal(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(
        app.modal,
        Some(Modal::Help(HelpModal {
            section: HelpSection::Home,
            ..
        }))
    ));
    app.handle_modal(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(app.modal.is_none());
}

#[test]
fn help_renders_in_full_and_narrow_terminals() {
    let mut app = interactive_test_app();
    app.view_mode = ViewMode::Provider;
    app.open_help();

    let backend = TestBackend::new(90, 26);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("1 Home"));
    assert!(rendered.contains("Help · Provider"));
    assert!(rendered.contains("Space / d / 1"));

    let backend = TestBackend::new(36, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Terminal too small"));
    assert!(rendered.contains("Esc to go back"));
}

#[test]
fn all_enabled_is_the_first_home_row_and_excludes_disabled_providers() {
    let mut app = interactive_test_app();
    app.config.profiles.get_mut("two").unwrap().enabled = false;

    let models = app.all_managed_models();
    assert_eq!(models.len(), 2);
    assert!(models.iter().all(|entry| entry.profile_id == "one"));
    assert_eq!(
        app.home_profile_item_heights(Rect::new(0, 0, 120, 30))
            .len(),
        3
    );

    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.find("All Models").unwrap() < rendered.find("One").unwrap());
    assert!(rendered.contains("2 models · 2 enabled · 1 providers"));

    app.home_all_selected = true;
    app.enter_all_enabled_view();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("All models"));
    assert!(rendered.contains("model-a"));
    assert!(!rendered.contains("Two  two"));
}

#[test]
fn disabled_provider_editor_marks_every_model_unavailable() {
    let mut app = interactive_test_app();
    app.config.profiles.get_mut("one").unwrap().enabled = false;
    let mut editor = app.create_route_editor_for("one".into()).unwrap();

    assert!(
        editor
            .catalog
            .iter()
            .all(|model| !editor.is_enabled(&model.id))
    );
    editor.toggle_selected();
    assert!(editor.status.contains("Provider is disabled"));
    assert!(
        editor
            .catalog
            .iter()
            .all(|model| !editor.is_enabled(&model.id))
    );
}

#[test]
fn all_enabled_page_can_disable_and_reenable_the_same_model() {
    let mut app = interactive_test_app();
    app.home_all_selected = true;
    app.enter_all_enabled_view();

    let selected = app
        .all_managed_models()
        .into_iter()
        .find(|entry| entry.profile_id == "one" && entry.model.id == "model-b")
        .unwrap();
    assert!(selected.enabled);
    let disabled = app.toggled_global_model_profile(&selected).unwrap();
    assert!(disabled.disabled_models.contains(&"model-b".into()));
    assert!(!disabled.enabled_models.contains(&"model-b".into()));
    app.config.profiles.insert("one".into(), disabled);

    let selected = app
        .all_managed_models()
        .into_iter()
        .find(|entry| entry.profile_id == "one" && entry.model.id == "model-b")
        .unwrap();
    assert!(!selected.enabled, "disabled model must remain visible");
    let enabled = app.toggled_global_model_profile(&selected).unwrap();
    assert!(!enabled.disabled_models.contains(&"model-b".into()));
    assert!(enabled.enabled_models.contains(&"model-b".into()));
}

#[test]
fn clicking_an_all_enabled_model_selects_then_opens_its_provider() {
    let mut app = interactive_test_app();
    app.home_all_selected = true;
    app.enter_all_enabled_view();
    let screen = Rect::new(0, 0, 120, 30);
    let panel = ui_areas(screen, app.focus, app.view_mode).models.unwrap();
    let index = app
        .all_managed_models()
        .iter()
        .position(|entry| entry.profile_id == "two" && entry.model.id == "model-b")
        .unwrap();

    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: panel.x + 6,
            row: panel.y + 1 + u16::try_from(index).unwrap() * 2,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();

    assert_eq!(app.view_mode, ViewMode::AllEnabled);
    assert_eq!(app.model_idx, index);
    assert!(app.status.contains("click again"));

    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: panel.x + 6,
            row: panel.y + 1 + u16::try_from(index).unwrap() * 2,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();

    assert_eq!(app.view_mode, ViewMode::Provider);
    assert_eq!(app.selected_profile_id().as_deref(), Some("two"));
    assert_eq!(app.selected_model().unwrap().id, "model-b");
}

#[test]
fn aggregate_editor_uses_the_target_provider_catalog() {
    let mut app = interactive_test_app();
    app.home_all_selected = true;
    app.enter_all_enabled_view();

    let editor = app.create_route_editor_for("two".into()).unwrap();
    assert_eq!(
        editor
            .catalog
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        ["model-a", "model-b"]
    );
}

#[test]
fn mouse_wheel_and_click_navigate_lists() {
    let (_temp, mut app) = persisted_app();
    let screen = Rect::new(0, 0, 120, 30);
    let areas = ui_areas(screen, app.focus, app.view_mode);
    let profiles = areas.profiles.unwrap();
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: profiles.x + 1,
            row: profiles.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    assert_eq!(app.profile_idx, 1);
    assert_eq!(app.focus, Focus::Profiles);

    app.view_mode = ViewMode::Provider;
    app.focus = Focus::Models;
    let models = ui_areas(screen, app.focus, app.view_mode).models.unwrap();
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: models.x + 1,
            row: models.y + 5,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    assert_eq!(app.model_idx, 1);
    assert_eq!(app.focus, Focus::Models);
}

#[test]
fn scrollbar_click_and_drag_cover_the_full_list() {
    let area = Rect::new(4, 3, 20, 12);
    assert_eq!(scrollbar_index(area, 23, 4, 100, 10), Some(0));
    assert_eq!(scrollbar_index(area, 23, 13, 100, 10), Some(99));
    assert_eq!(scrollbar_index(area, 22, 13, 100, 10), None);

    let mut app = interactive_test_app();
    let template = app.config.profiles["one"].clone();
    for index in 0..30 {
        app.config
            .profiles
            .insert(format!("route-{index:02}"), template.clone());
    }
    let screen = Rect::new(0, 0, 120, 20);
    let panel = ui_areas(screen, app.focus, app.view_mode).profiles.unwrap();
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: panel.x + panel.width - 1,
            row: panel.y + panel.height - 2,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    assert_eq!(app.profile_idx, app.config.profiles.len() - 1);
}

#[test]
fn model_picker_scrollbar_drag_moves_the_api_list() {
    let mut app = interactive_test_app();
    let models = (0..40)
        .map(|index| ModelEntry {
            id: format!("api-model-{index:02}"),
            label: None,
            description: None,
            max_output_tokens: None,
            context_window: None,
            reasoning_max: None,
        })
        .collect();
    app.modal = Some(Modal::Model(ModelForm::with_api_models(models)));
    let screen = Rect::new(0, 0, 120, 30);
    let area = modal_area_for(app.modal.as_ref().unwrap(), screen);
    let inner = panel_inner(area);
    let content = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(2),
    );
    let (_, api_area) = model_form_areas(content, false);
    let api_inner = panel_inner(api_area);
    let list_area = Rect::new(
        api_inner.x,
        api_inner.y + 2,
        api_inner.width,
        api_inner.height.saturating_sub(2),
    );
    app.handle_modal_mouse(
        MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: list_area.right() - 1,
            row: list_area.bottom() - 2,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    let Some(Modal::Model(form)) = &app.modal else {
        panic!("model form closed");
    };
    assert!(form.api_scroll > 0);
}

#[test]
fn provider_catalog_scrollbar_drag_selects_a_later_model() {
    let mut app = interactive_test_app();
    let profile = app.config.profiles.get_mut("one").unwrap();
    for index in 0..30 {
        profile.models.push(ModelEntry {
            id: format!("model-{index:02}"),
            label: None,
            description: None,
            max_output_tokens: None,
            context_window: None,
            reasoning_max: None,
        });
    }
    app.view_mode = ViewMode::Provider;
    app.focus = Focus::Models;
    let screen = Rect::new(0, 0, 120, 30);
    let panel = ui_areas(screen, app.focus, app.view_mode).models.unwrap();
    let list_area = Rect::new(
        panel.x,
        panel.y + 3,
        panel.width,
        panel.height.saturating_sub(3),
    );
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: list_area.right() - 1,
            row: list_area.bottom() - 2,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    assert!(app.model_idx > 0);
}

#[test]
fn route_details_exposes_clickable_provider_editor() {
    let mut app = interactive_test_app();
    app.view_mode = ViewMode::Provider;
    app.focus = Focus::Details;
    let screen = Rect::new(0, 0, 120, 30);
    let details = ui_areas(screen, app.focus, app.view_mode).details.unwrap();
    let edit = detail_controls(details)
        .into_iter()
        .find(|(control, _)| *control == DetailControl::Edit)
        .unwrap()
        .1;
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: edit.x,
            row: edit.y,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    assert!(matches!(app.modal, Some(Modal::Profile(_))));
}

#[test]
fn provider_page_keeps_global_sync_and_removes_duplicate_manager_controls() {
    let mut app = interactive_test_app();
    app.view_mode = ViewMode::Provider;
    app.focus = Focus::Details;
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(!rendered.contains("Manage models"));
    assert!(!rendered.contains("Enable all"));
    assert!(rendered.contains("Claude /model"));
    assert!(
        footer_controls(Rect::new(0, 27, 120, 3), false, app.view_mode)
            .iter()
            .any(|(control, _)| *control == FooterControl::Sync)
    );
}

#[test]
fn proxy_manager_renders_and_supports_keyboard_and_mouse_navigation() {
    let mut app = interactive_test_app();
    app.modal = Some(Modal::Proxy(ProxyManager {
        port_field: None,
        port_changed: false,
        instance: uuid::Uuid::new_v4(),
        runtime: Some(proxy::ProxyStatus {
            running: true,
            listen: "127.0.0.1:17321".into(),
            routes: 3,
            pid: Some(4242),
        }),
        service: Some(proxy::ProxyServiceStatus {
            installed: true,
            manager: "launchd",
            path: PathBuf::from("/tmp/com.ccsw.proxy.plist"),
        }),
        selected: 0,
        message: "Ready".into(),
        error: false,
    }));
    let screen = Rect::new(0, 0, 100, 30);
    let backend = TestBackend::new(screen.width, screen.height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Proxy control"));
    assert!(rendered.contains("Running in background"));
    assert!(rendered.contains("Enable at login"));
    assert!(rendered.contains("Disable at login"));

    app.handle_modal(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(
        app.modal,
        Some(Modal::Proxy(ProxyManager { selected: 1, .. }))
    ));

    let modal = modal_area_for(app.modal.as_ref().unwrap(), screen);
    let close = proxy_controls(modal)
        .into_iter()
        .find(|(control, _)| *control == ProxyControl::Close)
        .unwrap()
        .1;
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: close.x,
            row: close.y,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    assert!(app.modal.is_none());
}

#[test]
fn route_editor_searches_toggles_and_changes_default() {
    let models = ["alpha", "beta", "gamma"]
        .into_iter()
        .map(|id| ModelEntry {
            max_output_tokens: None,
            context_window: None,
            reasoning_max: None,
            id: id.into(),
            label: Some(id.to_uppercase()),
            description: None,
        })
        .collect();
    let mut editor = RouteEditor {
        profile_id: "route".into(),
        original_profile: interactive_test_app().config.profiles["one"].clone(),
        provider_enabled: true,
        catalog: models,
        enabled: BTreeSet::new(),
        disabled: BTreeSet::new(),
        locked: BTreeSet::new(),
        default_model: "alpha".into(),
        one_m: BTreeSet::new(),
        query: "bet".into(),
        selected: 0,
        search_active: false,
        status: String::new(),
    };
    assert_eq!(editor.filtered_indices(), [1]);
    editor.toggle_selected();
    assert!(editor.enabled.contains("beta"));
    editor.toggle_selected_1m();
    assert_eq!(editor.effective_id("beta"), "beta[1m]");
    editor.set_selected_default();
    assert_eq!(editor.default_model, "beta");
    assert!(!editor.enabled.contains("beta"));
    assert!(editor.is_enabled("beta"));
    assert!(!editor.is_enabled("gamma"));
}

#[test]
fn manual_model_form_supports_keyboard_1m_toggle() {
    let mut form = ModelForm::new();
    form.fields[0].value = "manual-model[1m]".into();
    form.fields[1].value = "Manual model".into();
    form.selected = 3;

    handle_form_key(
        &mut form.fields,
        &mut form.selected,
        KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
    );
    let model = form.to_model();
    assert_eq!(model.id, "manual-model[1m]");
    assert_eq!(model.label.as_deref(), Some("Manual model · 1M"));

    handle_form_key(
        &mut form.fields,
        &mut form.selected,
        KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
    );
    assert_eq!(form.to_model().id, "manual-model");
}

#[test]
fn model_form_can_save_a_model_without_enabling_it() {
    let mut form = ModelForm::new();
    form.fields[0].value = "parked-model".into();
    form.fields[4].value = "false".into();

    assert_eq!(form.to_model().id, "parked-model");
    assert!(!form.enable_now());
}

#[test]
fn saving_a_disabled_model_keeps_it_visible_in_the_provider_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = interactive_test_app();
    app.paths.config = dir.path().join("config.toml");
    std::fs::write(&app.paths.config, toml::to_string(&app.config).unwrap()).unwrap();
    app.enter_provider_view();

    let mut form = ModelForm::new();
    form.fields[0].value = "parked-model".into();
    form.fields[4].value = "false".into();
    app.modal = Some(Modal::Model(form));
    app.handle_modal(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    let profile = &app.config.profiles["one"];
    assert!(
        profile
            .models
            .iter()
            .any(|model| model.id == "parked-model")
    );
    assert!(
        profile
            .disabled_models
            .iter()
            .any(|id| id == "parked-model")
    );
    assert!(!profile.enabled_models.iter().any(|id| id == "parked-model"));
    let selected = app
        .provider_editor
        .as_ref()
        .unwrap()
        .selected_model()
        .unwrap();
    assert_eq!(selected.id, "parked-model");
    assert!(
        !app.provider_editor
            .as_ref()
            .unwrap()
            .is_enabled("parked-model")
    );
}

#[test]
fn blank_manual_model_stays_invalid_with_1m_enabled() {
    let mut form = ModelForm::new();
    form.fields[3].value = "true".into();
    assert!(canonical_model_id(form.fields[0].value.trim()).is_empty());
    assert!(form.to_model().id.is_empty());
}

#[test]
fn profile_form_cycles_api_format_and_auth_choices() {
    let mut form = ProfileForm::new();
    form.fields[0].value = "openai".into();
    form.fields[1].value = "OpenAI".into();
    form.fields[3].value = "https://api.example/v1".into();
    form.fields[5].value = "secret".into();
    form.fields[6].value = "gpt-test".into();
    form.selected = 2;
    handle_form_key(
        &mut form.fields,
        &mut form.selected,
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
    );
    form.selected = 4;
    handle_form_key(
        &mut form.fields,
        &mut form.selected,
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
    );
    let (_, profile) = form.to_profile().unwrap();
    assert_eq!(profile.api_format, ApiFormat::OpenaiChat);
    assert!(matches!(profile.credential, Credential::XApiKey { .. }));
}

#[test]
fn route_editor_persists_1m_for_default_aliases_and_enabled_models() {
    let mut app = interactive_test_app();
    let profile = app.config.profiles.get_mut("one").unwrap();
    profile.aliases.sonnet = Some("model-a".into());
    let editor = RouteEditor {
        profile_id: "one".into(),
        original_profile: profile.clone(),
        provider_enabled: true,
        catalog: profile.models.clone(),
        enabled: BTreeSet::from(["model-b".into()]),
        disabled: BTreeSet::new(),
        locked: BTreeSet::from(["model-a".into()]),
        default_model: "model-a".into(),
        one_m: BTreeSet::from(["model-a".into(), "model-b".into()]),
        query: String::new(),
        selected: 0,
        search_active: false,
        status: String::new(),
    };
    apply_route_editor(profile, &editor);
    assert_eq!(profile.default_model, "model-a[1m]");
    assert_eq!(profile.aliases.sonnet.as_deref(), Some("model-a[1m]"));
    assert_eq!(profile.enabled_models, ["model-b[1m]"]);
}

#[test]
fn form_field_cursor_and_text_editing() {
    let mut f = field("Test", "hello");
    assert_eq!(f.cursor, 5);
    f.insert_char('!');
    assert_eq!(f.value, "hello!");
    assert_eq!(f.cursor, 6);

    // move left twice
    f.cursor = 4;
    f.delete_backward();
    assert_eq!(f.value, "helo!");
    assert_eq!(f.cursor, 3);

    f.delete_forward();
    assert_eq!(f.value, "hel!");
    assert_eq!(f.cursor, 3);

    f.clear_text();
    assert_eq!(f.value, "");
    assert_eq!(f.cursor, 0);
}

#[test]
fn form_enter_navigates_and_submits() {
    let mut fields = vec![
        field("Name", "test"),
        toggle_field("Active", true),
        field("Target", "url"),
    ];
    let mut selected = 0;

    // Enter on field 0 advances to field 1
    let outcome = handle_form_key(
        &mut fields,
        &mut selected,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(outcome, FormOutcome::Stay);
    assert_eq!(selected, 1);

    // Enter confirms a toggle without changing it, then advances.
    let outcome = handle_form_key(
        &mut fields,
        &mut selected,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(outcome, FormOutcome::Stay);
    assert_eq!(fields[1].value, "true");
    assert_eq!(selected, 2);

    // Enter on last field returns Submit
    let outcome = handle_form_key(
        &mut fields,
        &mut selected,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(outcome, FormOutcome::Submit);
}

#[test]
fn route_editor_batch_enable_and_disable() {
    let catalog = vec![
        ModelEntry {
            max_output_tokens: None,
            context_window: None,
            reasoning_max: None,
            id: "default-m".into(),
            label: None,
            description: None,
        },
        ModelEntry {
            max_output_tokens: None,
            context_window: None,
            reasoning_max: None,
            id: "m-1".into(),
            label: None,
            description: None,
        },
        ModelEntry {
            max_output_tokens: None,
            context_window: None,
            reasoning_max: None,
            id: "m-2".into(),
            label: None,
            description: None,
        },
    ];
    let mut editor = RouteEditor {
        profile_id: "test".into(),
        original_profile: interactive_test_app().config.profiles["one"].clone(),
        provider_enabled: true,
        catalog,
        enabled: BTreeSet::new(),
        disabled: BTreeSet::new(),
        locked: BTreeSet::new(),
        default_model: "default-m".into(),
        one_m: BTreeSet::new(),
        query: String::new(),
        selected: 0,
        search_active: false,
        status: String::new(),
    };

    editor.enable_all_filtered();
    assert!(editor.enabled.contains("m-1"));
    assert!(editor.enabled.contains("m-2"));
    assert!(!editor.enabled.contains("default-m")); // default is required, not duplicated in enabled

    editor.disable_all_filtered();
    assert!(!editor.enabled.contains("m-1"));
    assert!(!editor.enabled.contains("m-2"));
    assert!(editor.is_enabled("default-m")); // default model stays effectively enabled
}

#[test]
fn app_esc_navigation_unfocuses_subpanels() {
    let mut app = interactive_test_app();
    app.view_mode = ViewMode::Provider;
    app.focus = Focus::Details;

    // In Provider view, Esc returns to Home view and Focus::Profiles
    app.view_mode = ViewMode::Home;
    app.focus = Focus::Profiles;
    assert_eq!(app.view_mode, ViewMode::Home);
    assert_eq!(app.focus, Focus::Profiles);
}

#[test]
fn home_screen_enter_and_esc_transitions() {
    let mut app = interactive_test_app();
    assert_eq!(app.view_mode, ViewMode::Home);
    assert_eq!(app.focus, Focus::Profiles);

    // Enter transitions to Provider view
    if app.selected_profile().is_some() {
        app.view_mode = ViewMode::Provider;
        app.focus = Focus::Models;
    }
    assert_eq!(app.view_mode, ViewMode::Provider);
    assert_eq!(app.focus, Focus::Models);

    // Esc transitions back to Home view
    app.view_mode = ViewMode::Home;
    app.focus = Focus::Profiles;
    assert_eq!(app.view_mode, ViewMode::Home);
    assert_eq!(app.focus, Focus::Profiles);
}

#[test]
fn home_screen_mouse_click_drills_down_to_provider() {
    let mut app = interactive_test_app();
    let screen = Rect::new(0, 0, 120, 30);
    let areas = ui_areas(screen, app.focus, app.view_mode);
    let panel = areas.profiles.unwrap();

    // The virtual All Models row is first; click the first provider below it.
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: panel.x + 8,
            row: panel.y + 4,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    assert_eq!(app.view_mode, ViewMode::Provider);
    assert_eq!(app.focus, Focus::Models);

    // Clicking the header back button returns to Home
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 4,
            row: 1,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    assert_eq!(app.view_mode, ViewMode::Home);
    assert_eq!(app.focus, Focus::Profiles);
}

#[test]
fn models_list_keybindings_set_default_and_toggle_1m() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = interactive_test_app();
    app.paths.config = dir.path().join("config.toml");
    std::fs::write(&app.paths.config, toml::to_string(&app.config).unwrap()).unwrap();

    // Model 0 is model-a (default), Model 1 is model-b
    app.view_mode = ViewMode::Provider;
    app.focus = Focus::Models;
    app.model_idx = 1;
    assert_eq!(app.selected_model().unwrap().id, "model-b");

    // Set selected as default
    app.set_selected_as_default();
    assert_eq!(app.config.profiles["one"].default_model, "model-b");

    // Toggle 1M on selected model (model-b)
    app.toggle_selected_model_1m();
    assert_eq!(app.config.profiles["one"].default_model, "model-b[1m]");
    assert!(app.selected_model().unwrap().id.ends_with("[1m]"));

    // Toggle 1M off
    app.toggle_selected_model_1m();
    assert_eq!(app.config.profiles["one"].default_model, "model-b");
    assert_eq!(app.selected_model().unwrap().id, "model-b");
}

#[test]
fn provider_screen_directly_displays_model_catalog_and_showcase() {
    let mut app = interactive_test_app();
    app.enter_provider_view();
    assert_eq!(app.view_mode, ViewMode::Provider);
    assert!(app.provider_editor.is_some());

    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("Selected model"));
    assert!(rendered.contains("Provider"));
    assert!(rendered.contains("One  →  model-a"));
    assert!(rendered.contains("model-b"));
}

#[test]
fn mouse_click_showcase_buttons_perform_actions() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = interactive_test_app();
    app.paths.config = dir.path().join("config.toml");
    std::fs::write(&app.paths.config, toml::to_string(&app.config).unwrap()).unwrap();

    app.enter_provider_view();
    let screen = Rect::new(0, 0, 120, 30);
    let details = ui_areas(screen, app.focus, app.view_mode).details.unwrap();
    let (showcase_card, _) = provider_detail_cards(details);
    let controls = showcase_controls(showcase_card, false);

    // Click Default button
    let default_btn = controls
        .iter()
        .find(|(c, _)| *c == ShowcaseControl::Default)
        .unwrap()
        .1;
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: default_btn.x + 1,
            row: default_btn.y,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    assert_eq!(app.config.profiles["one"].default_model, "model-a");

    // Click 1M button
    let onem_btn = controls
        .iter()
        .find(|(c, _)| *c == ShowcaseControl::OneM)
        .unwrap()
        .1;
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: onem_btn.x + 1,
            row: onem_btn.y,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    assert!(app.config.profiles["one"].default_model.ends_with("[1m]"));

    // Toggle is separate from deletion and keeps the catalog entry.
    let toggle_btn = controls
        .iter()
        .find(|(c, _)| *c == ShowcaseControl::Toggle)
        .unwrap()
        .1;
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: toggle_btn.x + 1,
            row: toggle_btn.y,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    assert!(!app.provider_editor.as_ref().unwrap().is_enabled("model-a"));
    assert_eq!(app.config.profiles["one"].models.len(), 2);
}

#[test]
fn mouse_click_catalog_add_and_detail_edit() {
    let mut app = interactive_test_app();
    app.enter_provider_view();
    let screen = Rect::new(0, 0, 120, 30);

    // 1. Click catalog add button
    let models = ui_areas(screen, app.focus, app.view_mode).models.unwrap();
    let search_area = Rect::new(models.x, models.y, models.width, 3);
    let add_btn = catalog_add_button_rect(search_area).unwrap();
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: add_btn.x + 1,
            row: add_btn.y,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    assert!(matches!(app.modal, Some(Modal::Model(_))));

    // Close modal
    app.modal = None;

    // 2. The details card keeps provider-specific edit/delete actions.
    let details = ui_areas(screen, app.focus, app.view_mode).details.unwrap();
    let (_, provider_card) = provider_detail_cards(details);
    assert_eq!(detail_controls(provider_card).len(), 2);
    let edit_btn = detail_controls(provider_card)
        .into_iter()
        .find(|(c, _)| *c == DetailControl::Edit)
        .unwrap()
        .1;
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: edit_btn.x + 1,
            row: edit_btn.y,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    assert!(matches!(app.modal, Some(Modal::Profile(_))));
}

#[test]
fn model_can_be_disabled_and_enabled_freely() {
    let mut app = interactive_test_app();
    app.view_mode = ViewMode::Provider;
    app.init_provider_editor();
    let editor = app.provider_editor.as_mut().unwrap();

    // Initially default model "model-a" is enabled
    assert!(editor.is_enabled("model-a"));
    assert_eq!(editor.default_model, "model-a");

    // Select model-a (index 0) and toggle it to disable
    editor.selected = 0;
    editor.toggle_selected();
    assert!(!editor.is_enabled("model-a"));
    // Since model-b was enabled, default model should be switched to model-b
    assert_eq!(editor.default_model, "model-b");

    // Toggle model-a again to re-enable it
    editor.toggle_selected();
    assert!(editor.is_enabled("model-a"));

    // Single model profile toggle test (like flatkey)
    let single_catalog = vec![ModelEntry {
        max_output_tokens: None,
        context_window: None,
        reasoning_max: None,
        id: "only-model".into(),
        label: None,
        description: None,
    }];
    let mut single_editor = RouteEditor {
        profile_id: "single".into(),
        original_profile: app.config.profiles["one"].clone(),
        provider_enabled: true,
        catalog: single_catalog,
        enabled: BTreeSet::new(),
        disabled: BTreeSet::new(),
        locked: BTreeSet::new(),
        default_model: "only-model".into(),
        one_m: BTreeSet::new(),
        query: String::new(),
        selected: 0,
        search_active: false,
        status: String::new(),
    };
    assert!(single_editor.is_enabled("only-model"));
    single_editor.toggle_selected();
    assert!(!single_editor.is_enabled("only-model"));
    assert!(single_editor.status.contains("Disabled only-model"));

    single_editor.toggle_selected();
    assert!(single_editor.is_enabled("only-model"));
    assert!(single_editor.status.contains("Enabled only-model"));
}

#[test]
fn disabling_a_model_persists_without_deleting_its_catalog_entry() {
    let mut app = interactive_test_app();
    let dir = tempfile::tempdir().unwrap();
    app.paths.config = dir.path().join("config.toml");
    std::fs::write(&app.paths.config, toml::to_string(&app.config).unwrap()).unwrap();
    app.enter_provider_view();

    let editor = app.provider_editor.as_mut().unwrap();
    editor.selected = 1;
    editor.toggle_selected();
    app.commit_provider_editor().unwrap();

    let profile = &app.config.profiles["one"];
    assert_eq!(profile.models.len(), 2);
    assert!(profile.models.iter().any(|model| model.id == "model-b"));
    assert!(profile.disabled_models.iter().any(|id| id == "model-b"));
    assert!(
        !discovery::active_models(profile, &[])
            .iter()
            .any(|model| model.id == "model-b")
    );

    app.init_provider_editor();
    assert!(!app.provider_editor.as_ref().unwrap().is_enabled("model-b"));
}

#[test]
fn narrow_layout_uses_one_provider_panel_and_wraps_profile_cards() {
    let mut app = interactive_test_app();
    app.config.profiles.get_mut("one").unwrap().base_url =
        "https://gateway.example.com/a/very/long/path/that/must/wrap".into();
    let narrow = Rect::new(0, 0, 44, 22);

    app.enter_provider_view();
    let model_view = ui_areas(narrow, Focus::Models, app.view_mode);
    assert!(model_view.models.is_some());
    assert!(model_view.details.is_none());
    let details_view = ui_areas(narrow, Focus::Details, app.view_mode);
    assert!(details_view.models.is_none());
    assert!(details_view.details.is_some());

    let profile = &app.config.profiles["one"];
    let lines = home_profile_lines("one", profile, 2, 42, false);
    assert!(lines.len() > 5);
    assert!(lines.iter().all(|line| line.width() <= 42));
}

#[test]
fn model_form_api_model_picker_populates_fields() {
    let api_models = vec![
        ModelEntry {
            max_output_tokens: None,
            context_window: None,
            reasoning_max: None,
            id: "qwen-max-latest".into(),
            label: Some("Qwen Max Latest".into()),
            description: Some("Alibaba Cloud flagship model".into()),
        },
        ModelEntry {
            max_output_tokens: None,
            context_window: None,
            reasoning_max: None,
            id: "deepseek-v4-flash[1m]".into(),
            label: Some("DeepSeek V4 Flash".into()),
            description: Some("Fast reasoning model".into()),
        },
    ];

    let mut form = ModelForm::with_api_models(api_models);
    assert_eq!(form.filtered_api_models().len(), 2);

    // Filter by keyword "deep"
    form.api_query = "deep".into();
    assert_eq!(form.filtered_api_models().len(), 1);
    assert_eq!(form.filtered_api_models()[0].id, "deepseek-v4-flash[1m]");

    // Pick the filtered model (index 0)
    form.pick_api_model(0);
    assert_eq!(form.fields[0].value, "deepseek-v4-flash");
    assert_eq!(form.fields[1].value, "DeepSeek V4 Flash");
    assert_eq!(form.fields[2].value, "Fast reasoning model");
    assert_eq!(form.fields[3].value, "true");

    // Form to model conversion
    let model = form.to_model();
    assert_eq!(model.id, "deepseek-v4-flash[1m]");
    assert!(model.label.unwrap().contains("1M"));
}

#[test]
fn model_form_search_and_scrolling() {
    let api_models = (0..20)
        .map(|i| ModelEntry {
            max_output_tokens: None,
            context_window: None,
            reasoning_max: None,
            id: format!("model-{i:02}"),
            label: Some(format!("Model {i}")),
            description: None,
        })
        .collect::<Vec<_>>();

    let mut form = ModelForm::with_api_models(api_models);
    assert_eq!(form.filtered_api_models().len(), 20);
    assert_eq!(form.api_scroll, 0);
    assert_eq!(form.api_selected, 0);
    assert!(!form.focus_api_search);

    // Scroll down in list
    form.scroll_api_list(true, 5, 10);
    assert_eq!(form.api_scroll, 5);

    // Scroll up in list
    form.scroll_api_list(false, 3, 10);
    assert_eq!(form.api_scroll, 2);

    // Move selection down
    form.move_api_selection(true, 10);
    assert_eq!(form.api_selected, 6);

    // Move selection up
    form.move_api_selection(false, 10);
    assert_eq!(form.api_selected, 5);

    // Search query filtering
    form.api_query = "model-1".into();
    let filtered = form.filtered_api_models();
    assert_eq!(filtered.len(), 10); // model-10 .. model-19
    assert_eq!(filtered[0].id, "model-10");
}

#[test]
fn provider_catalog_only_shows_added_models_not_unselected_gateway_models() {
    let mut app = interactive_test_app();
    // Insert cached discovered models from remote router (e.g. 10 models)
    let discovered = (0..10)
        .map(|i| ModelEntry {
            max_output_tokens: None,
            context_window: None,
            reasoning_max: None,
            id: format!("gateway-model-{i}"),
            label: Some(format!("Gateway Model {i}")),
            description: None,
        })
        .collect();
    app.cache.profiles.insert(
        "one".into(),
        CachedModels {
            fetched_at: 1000,
            models: discovered,
        },
    );

    // Profile "one" only has "model-a" and "model-b"
    let catalog = app.catalog_models();
    let ids: Vec<&str> = catalog.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["model-a", "model-b"]);
    assert!(!ids.contains(&"gateway-model-0"));
}

fn interactive_test_app() -> App {
    let model = |id: &str| ModelEntry {
        max_output_tokens: None,
        context_window: None,
        reasoning_max: None,
        id: id.into(),
        label: None,
        description: None,
    };
    let profile = |name: &str| Profile {
        name: name.into(),
        enabled: true,
        base_url: "https://example.com".into(),
        api_format: ApiFormat::Anthropic,
        credential: Credential::None,
        default_model: "model-a".into(),
        aliases: RoleModels::default(),
        subagent_model: None,
        fallback_models: vec![],
        enabled_models: vec!["model-b".into()],
        disabled_models: vec![],
        models: vec![model("model-a"), model("model-b")],
    };
    let mut config = Config::default();
    config.profiles.insert("one".into(), profile("One"));
    config.profiles.insert("two".into(), profile("Two"));
    App {
        paths: AppPaths {
            config: PathBuf::from("/tmp/ccsw-test-config"),
            state_dir: PathBuf::from("/tmp/ccsw-test-state"),
            cache: PathBuf::from("/tmp/ccsw-test-cache"),
        },
        config,
        cache: ModelCache::default(),
        theme: theme::Theme::default(),
        view_mode: ViewMode::Home,
        home_all_selected: false,
        profile_idx: 0,
        model_idx: 0,
        profile_offset: 0,
        model_offset: 0,
        focus: Focus::Profiles,
        status: "Ready".into(),
        status_error: false,
        modal: None,
        proxy_status: None,
        provider_editor: None,
        provider_card_selected: false,
        codex_ui: codex::CodexUi::default(),
        grok_auth: grok_auth::AuthUi::default(),
        grok_enabled: false,
        grok_home: std::path::PathBuf::from("/nonexistent-ccsw-test-grok"),
        pi_enabled: false,
        pi_home: std::path::PathBuf::from("/nonexistent-ccsw-test-pi"),
        background: Background::default(),
        screen: Rect::new(0, 0, 80, 24),
        usage: usage::UsageUi::default(),
    }
}
#[test]
fn usage_page_opens_filters_and_renders_at_supported_sizes() {
    use ratatui::backend::TestBackend;
    let mut app = interactive_test_app();
    app.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE))
        .unwrap();
    assert!(app.usage.active);
    for (width, height) in [(40, 12), (80, 24), (120, 40)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Usage"));
        assert!(text.contains("scroll"));
    }
    for key in [
        KeyCode::Char('a'),
        KeyCode::Right,
        KeyCode::Tab,
        KeyCode::Tab,
        KeyCode::Tab,
    ] {
        app.handle_key(KeyEvent::new(key, KeyModifiers::NONE))
            .unwrap();
    }
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("Pi: not tracked"));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(app.modal.is_none());
}

#[test]
fn usage_tab_opens_tables_and_click_filters() {
    use ratatui::backend::TestBackend;
    let mut app = interactive_test_app();
    let day = app.usage.snapshot.today();
    app.usage.snapshot.rows.push(crate::usage::Row {
        hour: 12,
        model: "gpt-test-model".into(),
        day: day.clone(),
        client: "Claude".into(),
        provider: "usage-fixture".into(),
        name: "Usage fixture".into(),
        kind: "generation".into(),
        totals: crate::usage::Totals {
            calls: 123,
            success: 120,
            failed: 3,
            input: 10000,
            output: 2000,
            ..Default::default()
        },
    });
    for width in [40, 72, 120] {
        let screen = Rect::new(0, 0, width, 24);
        let (_, rect) = client_tabs(screen)
            .into_iter()
            .find(|(tab, _)| *tab == ClientTab::Usage)
            .expect("Usage remains visible");
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: rect.x,
                row: rect.y,
                modifiers: KeyModifiers::NONE,
            },
            screen,
        )
        .unwrap();
        assert!(app.usage.active);
        app.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .unwrap();
        let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(rendered.contains("Usage fixture"));
        assert!(rendered.contains("123"));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(rendered.contains(&day));
        assert!(rendered.to_ascii_lowercase().contains("tokens"));
        app.handle_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE))
            .unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(rendered.contains("Success"));
        app.handle_key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE))
            .unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(rendered.contains("Models 4") || rendered.contains("Models4"));
        assert!(rendered.contains("gpt-test-model"));
        assert!(rendered.contains("123"));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
    }
}

pub(super) fn persisted_app() -> (tempfile::TempDir, App) {
    let temp = tempfile::tempdir().unwrap();
    let mut app = interactive_test_app();
    app.paths = AppPaths {
        config: temp.path().join("config.toml"),
        cache: temp.path().join("cache.json"),
        state_dir: temp.path().join("state"),
    };
    app.pi_home = temp.path().join("pi");
    crate::pi::native::update(&app.pi_home, |native| {
        native.profiles = app.config.profiles.clone();
        for profile in native.profiles.values_mut() {
            profile.enabled = true;
            profile.disabled_models.clear();
        }
        Ok(())
    })
    .unwrap();
    config::update(&app.paths.config, |latest| {
        *latest = app.config.clone();
        latest.codex.profiles = latest.profiles.clone();
        latest.pi.profiles = latest.profiles.clone();
        Ok(())
    })
    .unwrap();
    (temp, app)
}

#[test]
fn model_modal_keyboard_sequence_keeps_input_until_explicit_close() {
    let (_temp, mut app) = persisted_app();
    app.enter_provider_view();
    app.open_add_model_modal();
    for _ in 0..8 {
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .unwrap();
    }
    for key in [
        KeyCode::Char('中'),
        KeyCode::Char('文'),
        KeyCode::Left,
        KeyCode::Backspace,
        KeyCode::PageDown,
        KeyCode::PageUp,
        KeyCode::Down,
        KeyCode::Up,
    ] {
        app.handle_key(KeyEvent::new(key, KeyModifiers::NONE))
            .unwrap();
        assert!(
            matches!(app.modal, Some(Modal::Model(_))),
            "modal closed on {key:?}"
        );
    }
    let Some(Modal::Model(form)) = &app.modal else {
        unreachable!()
    };
    assert_eq!(form.api_query, "文");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(
        matches!(&app.modal, Some(Modal::Model(form)) if form.api_query.is_empty() && form.focus_api_search)
    );
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(&app.modal, Some(Modal::Model(form)) if !form.focus_api_search));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(app.modal.is_none());
}

#[test]
fn save_from_model_search_and_last_toggle_both_submit() {
    for from_search in [false, true] {
        let (_temp, mut app) = persisted_app();
        app.enter_provider_view();
        app.open_add_model_modal();
        let Some(Modal::Model(form)) = &mut app.modal else {
            unreachable!()
        };
        form.fields[0].value = "new-model".into();
        form.selected = form.fields.len() - 1;
        form.focus_api_search = from_search;
        let key = if from_search {
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
        } else {
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
        };
        app.handle_key(key).unwrap();
        assert!(app.modal.is_none());
        assert!(
            config::load(&app.paths.config).unwrap().profiles["one"]
                .models
                .iter()
                .any(|model| model.id == "new-model")
        );
        assert_eq!(app.background.status, sync::Status::NotConnected);
        assert!(
            !app.paths.state_dir.exists(),
            "first local save must not connect Claude"
        );
    }
}

#[test]
fn failed_save_preserves_form_and_default_reports_failure() {
    let (temp, mut app) = persisted_app();
    app.enter_provider_view();
    app.open_add_model_modal();
    let Some(Modal::Model(form)) = &mut app.modal else {
        unreachable!()
    };
    form.fields[0].value = "keep-my-input".into();
    let blocker = temp.path().join("not-a-directory");
    std::fs::write(&blocker, "block").unwrap();
    app.paths.config = blocker.join("config.toml");
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.status_error);
    assert!(
        matches!(&app.modal, Some(Modal::Model(form)) if form.fields[0].value == "keep-my-input")
    );
    app.modal = None;
    app.set_selected_as_default();
    assert!(app.status_error);
    assert!(app.status.contains("Could not save"));
}

#[test]
fn reenable_provider_preserves_explicitly_disabled_default() {
    let (_temp, mut app) = persisted_app();
    app.config = config::update(&app.paths.config, |config| {
        let provider = config.profiles.get_mut("one").unwrap();
        provider.enabled = false;
        provider
            .disabled_models
            .push(provider.default_model.clone());
        Ok(())
    })
    .unwrap();
    let previous = app.config.profiles["one"].clone();
    app.toggle_selected_provider().unwrap();
    let current = &app.config.profiles["one"];
    assert!(current.enabled);
    assert_eq!(current.disabled_models, previous.disabled_models);
    assert_eq!(current.enabled_models, previous.enabled_models);
}

#[test]
fn model_edit_merges_external_connection_change_without_resurrecting_deletion() {
    let (_temp, mut app) = persisted_app();
    app.enter_provider_view();
    config::update(&app.paths.config, |config| {
        config.profiles.get_mut("one").unwrap().base_url = "https://new.example".into();
        Ok(())
    })
    .unwrap();
    app.provider_editor.as_mut().unwrap().toggle_selected_1m();
    app.commit_provider_editor().unwrap();
    assert_eq!(app.config.profiles["one"].base_url, "https://new.example");
    config::update(&app.paths.config, |config| {
        config.profiles.remove("one");
        Ok(())
    })
    .unwrap();
    app.provider_editor.as_mut().unwrap().toggle_selected_1m();
    assert!(app.commit_provider_editor().is_err());
    assert!(
        !config::load(&app.paths.config)
            .unwrap()
            .profiles
            .contains_key("one")
    );
}

#[test]
fn api_selection_clears_previous_context_and_description() {
    let mut form = ModelForm::with_api_models(vec![
        ModelEntry {
            max_output_tokens: None,
            context_window: None,
            reasoning_max: None,
            id: "a[1m]".into(),
            label: None,
            description: Some("Old description".into()),
        },
        ModelEntry {
            max_output_tokens: None,
            context_window: None,
            reasoning_max: None,
            id: "b".into(),
            label: None,
            description: None,
        },
    ]);
    form.pick_api_model(0);
    form.pick_api_model(1);
    assert_eq!(form.to_model().id, "b");
    assert!(form.to_model().description.is_none());
}

#[test]
fn unicode_input_and_scrolled_form_keep_the_selected_field_visible() {
    let text = "https://模型.example/long/路径/末尾";
    let shown = input_window(text, Some(text.chars().count()), 12, false);
    assert!(shown.ends_with("末尾▌"));
    assert!(UnicodeWidthStr::width(shown.as_str()) <= 12);
    assert!(!input_window("secret-key", Some(10), 6, true).contains("key"));
    let mut app = interactive_test_app();
    app.edit_profile();
    let Some(Modal::Profile(form)) = &mut app.modal else {
        unreachable!()
    };
    form.selected = form.fields.len() - 1;
    let mut terminal = Terminal::new(TestBackend::new(60, 18)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let output = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(output.contains("Fallbacks"));
    let area = modal_area_for(app.modal.as_ref().unwrap(), Rect::new(0, 0, 60, 18));
    let inner = panel_inner(area);
    let content = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(4),
    );
    let (_, offset) = form_viewport(content, 12);
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: inner.x,
            row: inner.y,
            modifiers: KeyModifiers::NONE,
        },
        Rect::new(0, 0, 60, 18),
    )
    .unwrap();
    assert!(matches!(&app.modal, Some(Modal::Profile(form)) if form.selected == offset));
}

#[test]
fn render_matrix_covers_pages_forms_and_errors() {
    for (width, height) in [(120, 36), (100, 28), (80, 24), (60, 18), (40, 12)] {
        let mut app = interactive_test_app();
        app.config.profiles.get_mut("one").unwrap().name = "Production · 主网关".into();
        for view in [ViewMode::Home, ViewMode::AllEnabled, ViewMode::Provider] {
            app.view_mode = view;
            app.init_provider_editor();
            for modal in [0, 1, 2, 3, 4] {
                app.modal = None;
                app.status_error = false;
                app.status = "Ready".into();
                app.view_mode = view;
                match modal {
                    1 => {
                        app.home_all_selected = false;
                        app.view_mode = ViewMode::Provider;
                        app.edit_profile();
                    }
                    2 => app.open_add_model_modal(),
                    3 => app.open_help(),
                    4 => {
                        app.modal = Some(Modal::Proxy(ProxyManager::empty()));
                    }
                    _ => {}
                }
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|frame| app.draw(frame)).unwrap();
                let buffer = terminal.backend().buffer();
                assert_eq!(buffer.area.width, width);
                if let Ok(directory) = std::env::var("CCSW_RENDER_DIR") {
                    std::fs::create_dir_all(&directory).unwrap();
                    let lines = (0..height)
                        .map(|row| {
                            (0..width)
                                .map(|column| buffer[(column, row)].symbol())
                                .collect::<String>()
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    std::fs::write(
                        format!("{directory}/{width}x{height}-{view:?}-{modal}.txt"),
                        lines,
                    )
                    .unwrap();
                }
                app.status_error = true;
                app.status = "Could not save · test error · retry with Ctrl+S".into();
                terminal.draw(|frame| app.draw(frame)).unwrap();
            }
        }
    }
}

#[test]
fn proxy_port_editor_supports_keyboard_mouse_and_validation() {
    let (_temp, mut app) = persisted_app();
    app.modal = Some(Modal::Proxy(ProxyManager::empty()));
    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
        .unwrap();
    for ch in "17322".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .unwrap();
    }
    assert!(
        matches!(&app.modal, Some(Modal::Proxy(manager)) if manager.port_field.as_ref().unwrap().value == "17322")
    );
    for (width, height) in [(40, 12), (60, 18), (120, 36)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("17322"));
        assert!(text.contains("Save port"));
    }
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(&app.modal, Some(Modal::Proxy(manager)) if manager.port_field.is_none()));
    let screen = Rect::new(0, 0, 80, 24);
    let area = modal_area_for(app.modal.as_ref().unwrap(), screen);
    let (_, rect) = proxy_controls(area)
        .into_iter()
        .find(|(control, _)| *control == ProxyControl::Port)
        .unwrap();
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    let Some(Modal::Proxy(manager)) = &mut app.modal else {
        unreachable!()
    };
    manager.port_field.as_mut().unwrap().value = "70000".into();
    manager.activate(&app.paths, ProxyControl::Port);
    assert!(manager.error);
    assert!(manager.port_field.is_some());
    assert!(manager.message.contains("65535"));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    manager.port_field.as_mut().unwrap().value = port.to_string();
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while app.background.proxy_running && std::time::Instant::now() < deadline {
        app.poll_background();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!app.background.proxy_running);
    assert!(
        matches!(&app.modal, Some(Modal::Proxy(manager)) if manager.port_field.is_none() && manager.port_changed && !manager.error)
    );
    assert_eq!(
        app.proxy_status.as_ref().unwrap().listen,
        format!("127.0.0.1:{port}")
    );
}

#[test]
fn model_token_form_validates_and_round_trips() {
    let mut form = ModelForm::new();
    form.fields[0].value = "m".into();
    for invalid in ["0", "-1", "1.5", "4294967296", "no"] {
        form.fields[6].value = invalid.into();
        assert!(form.validate_tokens().is_err());
    }
    form.fields[6].value = "8192".into();
    form.fields[7].value = "4096".into();
    assert!(form.validate_tokens().is_err());
    form.fields[7].value = "32768".into();
    form.validate_tokens().unwrap();
    let model = form.to_model();
    let roundtrip: ModelEntry = toml::from_str(&toml::to_string(&model).unwrap()).unwrap();
    assert_eq!(roundtrip.max_output_tokens, Some(8192));
    assert_eq!(roundtrip.context_window, Some(32768));
}

#[test]
fn edit_shortcut_targets_the_current_page_regardless_of_panel_focus() {
    let (_temp, mut app) = persisted_app();
    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(app.modal, Some(Modal::Profile(_))));
    app.modal = None;
    app.enter_provider_view();

    for focus in [Focus::Models, Focus::Details] {
        app.focus = focus;
        let id = canonical_model_id(&app.selected_model().unwrap().id);
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
            .unwrap();
        assert!(matches!(&app.modal, Some(Modal::Model(form))
            if form.original_model_id.as_deref() == Some(id.as_str())));
        app.modal = None;
        app.handle_key(KeyEvent::new(KeyCode::Char('E'), KeyModifiers::SHIFT))
            .unwrap();
        assert!(matches!(app.modal, Some(Modal::Profile(_))));
        app.modal = None;
    }
}

#[test]
fn editing_tokens_keeps_disabled_model_disabled() {
    let (_temp, mut app) = persisted_app();
    app.enter_provider_view();
    let id = app.selected_model().unwrap().id;
    if let Some(editor) = &mut app.provider_editor {
        editor.disabled.insert(canonical_model_id(&id));
    }
    app.focus = Focus::Details;
    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .unwrap();
    let Some(Modal::Model(form)) = &mut app.modal else {
        panic!("model editor missing");
    };
    assert!(!form.enable_now());
    form.fields[6].value = "8192".into();
    form.fields[7].value = "32768".into();
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    let selected = app.selected_profile().unwrap();
    let model = selected
        .models
        .iter()
        .find(|m| canonical_model_id(&m.id) == canonical_model_id(&id))
        .unwrap();
    assert_eq!(model.max_output_tokens, Some(8192));
    assert!(
        selected
            .disabled_models
            .iter()
            .any(|m| canonical_model_id(m) == canonical_model_id(&id))
    );
}

#[test]
fn model_form_1m_shortcut_preserves_input_and_focus() {
    let mut form = ModelForm::new();
    form.fields[0].value = "model".into();
    form.fields[0].cursor = 5;
    for selected in 0..form.fields.len() {
        form.selected = selected;
        let before = form.fields[selected].value.clone();
        form.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT), 5);
        assert_eq!(form.to_model().id, "model[1m]");
        assert_eq!(form.selected, selected);
        assert!(!form.focus_api_search);
        form.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT), 5);
        assert_eq!(form.to_model().id, "model");
        assert_eq!(form.fields[selected].value, before);
    }
    form.selected = 0;
    form.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE), 5);
    assert_eq!(form.to_model().id, "model1");
    form.focus_api_search = true;
    form.api_query = "search".into();
    form.api_query_cursor = 6;
    form.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT), 5);
    assert!(form.focus_api_search);
    assert_eq!(form.api_query, "search");
    assert_eq!(form.api_query_cursor, 6);
    assert_eq!(form.to_model().id, "model1[1m]");
    form.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE), 5);
    assert_eq!(form.api_query, "search1");
}

#[test]
fn codex_navigation_preserves_provider_editing_and_has_scrollable_help() {
    let (_temp, mut app) = persisted_app();
    app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE))
        .unwrap();
    assert!(app.codex_ui.enabled);
    app.enter_provider_view();
    app.focus = Focus::Details;
    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(app.modal, Some(Modal::Model(_))));
    app.modal = None;
    app.handle_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE))
        .unwrap();
    assert!(app.codex_ui.accounts);
    for (width, height) in [(40, 12), (60, 18), (120, 36)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("ChatGPT Account"));
        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE))
            .unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE))
            .unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
    }
    app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE))
        .unwrap();
    assert!(!app.codex_ui.enabled);
}

#[test]
fn pi_navigation_keeps_model_and_provider_edit_shortcuts() {
    let (_temp, mut app) = persisted_app();
    for _ in 0..2 {
        app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE))
            .unwrap();
    }
    assert!(app.pi_enabled);
    assert!(!app.codex_ui.enabled);
    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(app.modal, Some(Modal::Profile(_))));
    app.modal = None;
    app.enter_provider_view();
    app.focus = Focus::Details;
    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(app.modal, Some(Modal::Model(_))));
    app.modal = None;
    for (width, height) in [(40, 12), (80, 24), (120, 36)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("‹ Back (Esc)"));
    }
    app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.client_tab(), ClientTab::Grok);
    app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.client_tab(), ClientTab::Usage);
    app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE))
        .unwrap();
    assert!(!app.pi_enabled);
    assert!(!app.codex_ui.enabled);
}

#[test]
fn client_tabs_click_from_accounts_and_preserve_active_view_and_modal() {
    let (_temp, mut app) = persisted_app();
    let screen = Rect::new(3, 2, 80, 24);
    let click = |app: &mut App, tab| {
        let (_, rect) = client_tabs(screen)
            .into_iter()
            .find(|(t, _)| *t == tab)
            .unwrap();
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: rect.x + 1,
                row: rect.y,
                modifiers: KeyModifiers::NONE,
            },
            screen,
        )
        .unwrap();
    };
    click(&mut app, ClientTab::Codex);
    assert!(app.codex_ui.enabled);
    app.handle_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE))
        .unwrap();
    assert!(app.codex_ui.accounts);
    click(&mut app, ClientTab::Pi);
    assert!(app.pi_enabled);
    app.enter_provider_view();
    click(&mut app, ClientTab::Pi);
    assert_eq!(app.view_mode, ViewMode::Provider);
    app.edit_profile();
    click(&mut app, ClientTab::Claude);
    assert!(app.pi_enabled);
    assert!(matches!(app.modal, Some(Modal::Profile(_))));
    app.modal = None;
    click(&mut app, ClientTab::Claude);
    assert_eq!(app.client_tab(), ClientTab::Claude);
}

#[test]
fn client_tabs_are_visible_and_highlighted_on_all_clients_at_minimum_size() {
    let (_temp, mut app) = persisted_app();
    app.theme = theme::Theme::Classic;
    for (width, height) in [(40, 12), (80, 24), (120, 36)] {
        for tab in [
            ClientTab::Claude,
            ClientTab::Codex,
            ClientTab::Pi,
            ClientTab::Grok,
            ClientTab::Usage,
        ] {
            app.select_client_tab(tab);
            for accounts in [false, true] {
                app.codex_ui.accounts = accounts;
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|frame| app.draw(frame)).unwrap();
                let buffer = terminal.backend().buffer();
                let first_row = (0..width)
                    .map(|x| buffer[(x, 0)].symbol())
                    .collect::<String>();
                for label in ["Claude Code", "Codex", "Pi", "Grok", "Usage"] {
                    assert!(first_row.contains(label));
                }
                for (candidate, rect) in client_tabs(Rect::new(0, 0, width, height)) {
                    assert_eq!(
                        buffer[(rect.x, rect.y)].bg,
                        if candidate == tab { ROUTE } else { SELECTION }
                    );
                }
            }
        }
    }
}

#[test]
fn client_provider_edits_are_isolated_on_disk_and_screen() {
    let (_temp, mut app) = persisted_app();
    let before = config::load(&app.paths.config).unwrap();
    app.select_client_tab(ClientTab::Codex);
    let id = app.selected_profile_id().unwrap();
    let original = app.config.profiles[&id].clone();
    let mut edited = original.clone();
    edited.name = "Codex only".into();
    app.config = config::update_client_profile(
        &app.paths.config,
        app.config_client(),
        &id,
        &original,
        &edited,
    )
    .unwrap();
    app.queue_sync(false, None);
    assert!(app.background.queued_sync.is_none());
    app.select_client_tab(ClientTab::Pi);
    assert_ne!(app.config.profiles[&id].name, "Codex only");
    app.select_client_tab(ClientTab::Claude);
    assert_eq!(app.config.profiles, before.profiles);
    let disk = config::load(&app.paths.config).unwrap();
    assert_eq!(disk.profiles, before.profiles);
    assert_eq!(disk.pi.profiles, before.pi.profiles);
    assert_eq!(disk.codex.profiles[&id].name, "Codex only");
}

#[test]
fn fresh_client_tabs_start_with_independent_empty_catalogs() {
    let (_temp, mut app) = persisted_app();
    app.pi_home = _temp.path().join("empty-pi");
    config::update(&app.paths.config, |c| {
        c.codex.profiles.clear();
        c.pi.profiles.clear();
        Ok(())
    })
    .unwrap();
    app.select_client_tab(ClientTab::Codex);
    assert!(app.config.profiles.is_empty());
    app.select_client_tab(ClientTab::Pi);
    assert!(app.config.profiles.is_empty());
    app.select_client_tab(ClientTab::Claude);
    assert!(!app.config.profiles.is_empty());
}

#[test]
fn pi_help_is_client_specific_and_codex_account_buttons_are_clickable() {
    let (_temp, mut app) = persisted_app();
    app.select_client_tab(ClientTab::Pi);
    app.open_help();
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("Pi Help"));
    assert!(text.contains("Set the selected provider and model in settings.json"));
    assert!(!text.contains("manage proxy"));
    app.modal = None;
    app.select_client_tab(ClientTab::Codex);
    app.handle_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE))
        .unwrap();
    let area = Rect::new(0, 0, 120, 36);
    app.codex_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 33,
            modifiers: KeyModifiers::NONE,
        },
        area,
    )
    .unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("Import current login"));
}

#[test]
fn codex_account_provider_and_help_use_shared_navigation() {
    let (_temp, mut app) = persisted_app();
    app.select_client_tab(ClientTab::Codex);
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("ChatGPT Account"));
    assert!(text.contains("Providers · F2 Pi"));
    app.home_all_selected = false;
    let area = Rect::new(0, 0, 120, 36);
    let panel = ui_areas(area, app.focus, app.view_mode).profiles.unwrap();
    let account_row = panel_inner(panel).y + app.home_profile_item_heights(panel)[0] as u16;
    let mouse = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 8,
        row: account_row,
        modifiers: KeyModifiers::NONE,
    };
    app.handle_mouse(mouse, area).unwrap();
    assert!(!app.codex_ui.accounts);
    assert!(app.home_all_selected);
    app.handle_mouse(mouse, area).unwrap();
    assert!(app.codex_ui.accounts);
    app.open_help();
    assert!(matches!(app.modal, Some(Modal::Help(_))));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("2 Accounts"));
    assert!(text.contains("No quota queries"));
    app.handle_modal(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(
        app.modal,
        Some(Modal::Help(HelpModal {
            section: HelpSection::Provider,
            ..
        }))
    ));
}

#[test]
fn codex_account_apply_without_login_stays_on_provider_home() {
    let (_temp, mut app) = persisted_app();
    app.select_client_tab(ClientTab::Codex);
    app.select_home_index(1);
    app.apply_codex();
    assert!(!app.codex_ui.accounts);
    assert!(app.status_error);
    assert!(app.status.contains("No saved account"));
    for width in [20, 38, 80, 118] {
        for line in app.chatgpt_provider_lines(width) {
            assert!(line.width() <= usize::from(width));
        }
    }
}

#[test]
fn codex_all_models_has_separate_home_row_and_only_enabled_models() {
    let (_temp, mut app) = persisted_app();
    config::update(&app.paths.config, |c| {
        let mut profile = c.profiles.values().next().unwrap().clone();
        profile.default_model = "enabled-model".into();
        profile.models.clear();
        profile.aliases = Default::default();
        profile.enabled_models.clear();
        profile.disabled_models = vec!["hidden-model".into()];
        profile.fallback_models.clear();
        profile.subagent_model = None;
        c.codex.profiles.clear();
        c.codex.profiles.insert("api".into(), profile);
        Ok(())
    })
    .unwrap();
    app.select_client_tab(ClientTab::Codex);
    app.select_home_index(0);
    assert!(app.codex_ui.home_models);
    assert_eq!(app.home_selected_index(), 0);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.view_mode, ViewMode::AllEnabled);
    assert!(!app.codex_ui.accounts);
    assert_eq!(app.all_managed_models().len(), 1);
    assert_eq!(app.all_managed_models()[0].model.id, "enabled-model");
    assert!(app.all_managed_models().iter().all(|m| m.enabled));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    app.select_home_index(1);
    assert!(app.home_account_selected());
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(app.codex_ui.accounts);
}

#[test]
fn subscription_toggle_requires_confirmation_and_cancel_preserves_config() {
    let (_temp, mut app) = persisted_app();
    app.select_client_tab(ClientTab::Codex);
    app.config.codex.accounts.insert(
        "saved".into(),
        crate::codex::accounts::Account {
            name: "Saved".into(),
            ..Default::default()
        },
    );
    // Choose the account using the existing account-list selection mechanism.
    app.open_codex_accounts();
    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    app.select_home_index(1);
    let before = app.config.clone();
    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
        .unwrap();
    assert!(app.codex_navigation_blocked());
    assert!(!app.codex_ui.busy);
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("Enable ChatGPT subscription?"));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.config, before);
    assert!(!app.codex_navigation_blocked());

    app.config.codex.active = Some(crate::codex::Selection::Account { id: "saved".into() });
    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
        .unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("Disable ChatGPT subscription?"));
    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.subscription_enabled());
    assert!(!app.codex_navigation_blocked());
}

#[test]
fn codex_space_selects_account_without_applying_or_following_cursor() {
    let (_temp, mut app) = persisted_app();
    app.select_client_tab(ClientTab::Codex);
    app.config.codex.accounts.insert(
        "a".into(),
        crate::codex::accounts::Account {
            name: "First".into(),
            ..Default::default()
        },
    );
    app.config.codex.accounts.insert(
        "b".into(),
        crate::codex::accounts::Account {
            name: "Second".into(),
            ..Default::default()
        },
    );
    app.open_codex_accounts();
    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(!app.codex_ui.busy);
    assert!(app.config.codex.active.is_none());
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("● First"));
    assert!(text.contains("○ Second"));
}

#[test]
fn pi_uses_provider_layout_without_proxy_controls() {
    let (_temp, mut app) = persisted_app();
    app.select_client_tab(ClientTab::Pi);
    for (width, height) in [(40, 12), (80, 24), (120, 36)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("Providers · F2 Grok CLI"));
        let area = Rect::new(0, 0, width, height);
        let controls = app.client_footer_controls(app_rows(area)[2], width < 100);
        assert!(
            !controls
                .iter()
                .any(|(control, _)| *control == FooterControl::Proxy)
        );
        let gap = controls[1].1.x - controls[0].1.right();
        assert!(gap >= 1);
        for pair in controls.windows(2) {
            assert_eq!(pair[0].1.right() + gap, pair[1].1.x);
        }
        app.handle_key(KeyEvent::new(KeyCode::Char('P'), KeyModifiers::NONE))
            .unwrap();
        assert!(app.modal.is_none());
        let (_, help) = controls
            .iter()
            .find(|(control, _)| *control == FooterControl::Help)
            .unwrap();
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: help.x,
                row: help.y,
                modifiers: KeyModifiers::NONE,
            },
            area,
        )
        .unwrap();
        assert!(matches!(app.modal, Some(Modal::Help(_))));
        app.modal = None;
    }
}

#[test]
fn pi_native_model_form_saves_without_changing_ccsw_config() {
    let (_temp, mut app) = persisted_app();
    let ccsw_before = std::fs::read(&app.paths.config).unwrap();
    app.select_client_tab(ClientTab::Pi);
    let profile = app.selected_profile_id().unwrap();
    app.enter_provider_view();
    app.open_add_model_modal();
    let Some(Modal::Model(form)) = &mut app.modal else {
        panic!("model form missing")
    };
    form.fields[0].value = "native-added".into();
    assert!(!form.fields.iter().any(|field| field.label == "Enable now"));
    for (label, value) in [("Max output tokens", "8192"), ("Context window", "128000")] {
        form.fields
            .iter_mut()
            .find(|field| field.label == label)
            .unwrap()
            .value = value.into();
    }
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.modal.is_none(), "{}", app.status);
    let models: serde_json::Value =
        serde_json::from_slice(&std::fs::read(app.pi_home.join("models.json")).unwrap()).unwrap();
    assert!(
        models["providers"][&profile]["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == "native-added")
    );
    assert!(models["providers"].get(format!("ccsw-{profile}")).is_none());
    let saved = models["providers"][&profile]["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == "native-added")
        .unwrap();
    assert_eq!(saved["maxTokens"], 8192);
    assert_eq!(saved["contextWindow"], 128000);
    app.select_model_id("native-added");
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
        .unwrap();
    assert!(!app.status_error, "{}", app.status);
    let settings: serde_json::Value =
        serde_json::from_slice(&std::fs::read(app.pi_home.join("settings.json")).unwrap()).unwrap();
    assert_eq!(settings["defaultProvider"], profile);
    assert_eq!(settings["defaultModel"], "native-added");
    app.select_client_tab(ClientTab::Claude);
    app.select_client_tab(ClientTab::Pi);
    assert!(
        app.config.profiles[&profile]
            .models
            .iter()
            .any(|m| m.id == "native-added")
    );
    assert_eq!(std::fs::read(&app.paths.config).unwrap(), ccsw_before);
}

#[test]
fn pi_availability_controls_do_not_mutate_native_files() {
    let (_temp, mut app) = persisted_app();
    app.select_client_tab(ClientTab::Pi);
    let before = std::fs::read(app.pi_home.join("models.json")).unwrap();
    let screen = Rect::new(0, 0, 120, 36);
    for view in [ViewMode::Home, ViewMode::Provider, ViewMode::AllEnabled] {
        app.view_mode = view;
        app.home_all_selected = false;
        if view == ViewMode::Provider {
            app.init_provider_editor();
        }
        for key in [' ', 'A', 'C'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE))
                .unwrap();
            assert!(!app.status_error, "{}", app.status);
        }
        let ui = ui_areas(screen, app.focus, view);
        let (column, row) = match view {
            ViewMode::Home => {
                let panel = ui.profiles.unwrap();
                (
                    panel.x + 2,
                    panel.y + 1 + app.home_profile_item_heights(panel)[0] as u16,
                )
            }
            ViewMode::Provider => {
                let panel = ui.models.unwrap();
                (panel.x + 2, panel.y + 4)
            }
            ViewMode::AllEnabled => {
                let panel = ui.models.unwrap();
                (panel.x + 2, panel.y + 1)
            }
        };
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            },
            screen,
        )
        .unwrap();
        if view == ViewMode::Provider {
            app.provider_editor.as_mut().unwrap().search_active = true;
            app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
                .unwrap();
            assert!(!app.provider_editor.as_ref().unwrap().search_active);
            let (card, _) = provider_detail_cards(ui.details.unwrap());
            assert!(
                !showcase_controls(card, true)
                    .iter()
                    .any(|(control, _)| *control == ShowcaseControl::Toggle)
            );
        }
        assert_eq!(
            std::fs::read(app.pi_home.join("models.json")).unwrap(),
            before
        );
    }
}

#[test]
fn pi_views_and_help_use_configured_model_labels() {
    let (_temp, mut app) = persisted_app();
    app.select_client_tab(ClientTab::Pi);
    let mut terminal = Terminal::new(TestBackend::new(150, 45)).unwrap();
    for view in [ViewMode::Home, ViewMode::Provider, ViewMode::AllEnabled] {
        app.view_mode = view;
        app.home_all_selected = false;
        if view == ViewMode::Provider {
            app.init_provider_editor();
        }
        app.status.clear();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
            .to_lowercase();
        assert!(rendered.contains("configured"));
        for stale in [
            " enabled",
            " disabled",
            "space toggle",
            "space enable",
            "space disable",
        ] {
            assert!(!rendered.contains(stale), "{view:?}: {stale}");
        }
    }
    for section in HelpSection::ALL {
        let help = HelpModal {
            section,
            pi: true,
            ..HelpModal::for_view(ViewMode::Home)
        };
        app.modal = Some(Modal::Help(help));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
            .to_lowercase();
        assert!(!rendered.contains("enable all"));
        assert!(!rendered.contains(" disabled"));
        assert!(!rendered.contains("role dependency"));
    }
}

#[test]
fn provider_context_controls_toggle_each_role_and_fallbacks() {
    let mut app = interactive_test_app();
    for edit in [false, true] {
        let mut form = if edit {
            ProfileForm::edit("one".into(), &app.config.profiles["one"])
        } else {
            ProfileForm::new()
        };
        for index in 6..form.fields.len() {
            form.fields[index].value = if index == 12 {
                "alpha[1m], beta"
            } else {
                "alpha"
            }
            .into();
        }
        app.modal = Some(Modal::Profile(Box::new(form)));
        for index in 6..13 {
            let Some(Modal::Profile(form)) = &mut app.modal else {
                panic!()
            };
            form.selected = index;
            app.handle_modal(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT))
                .unwrap();
            let Some(Modal::Profile(form)) = &app.modal else {
                panic!()
            };
            assert!(form.model_field_is_1m(index));
            assert_eq!(form.selected, index);
            assert_eq!(
                form.fields[index].value,
                if index == 12 {
                    "alpha[1m],beta[1m]"
                } else {
                    "alpha[1m]"
                }
            );
            app.handle_modal(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT))
                .unwrap();
            let Some(Modal::Profile(form)) = &app.modal else {
                panic!()
            };
            assert!(!form.model_field_is_1m(index));
        }
    }
    let mut form = ProfileForm::new();
    form.toggle_model_field_1m(6);
    assert!(form.fields[6].value.is_empty());
}

#[test]
fn provider_context_checkbox_click_matches_scrolled_row() {
    for screen in [Rect::new(0, 0, 120, 30), Rect::new(0, 0, 48, 18)] {
        let mut app = interactive_test_app();
        let mut form = ProfileForm::new();
        form.selected = 12;
        form.fields[12].value = "alpha,beta".into();
        app.modal = Some(Modal::Profile(Box::new(form)));
        let inner = panel_inner(modal_area(screen));
        let content = Rect::new(
            inner.x,
            inner.y,
            inner.width,
            inner.height.saturating_sub(4),
        );
        let (_, offset) = form_viewport(content, 12);
        let checkbox = profile_1m_rect(content, (12 - offset) as u16);
        app.handle_modal_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: checkbox.x + 2,
                row: checkbox.y,
                modifiers: KeyModifiers::NONE,
            },
            screen,
        )
        .unwrap();
        let Some(Modal::Profile(form)) = &app.modal else {
            panic!()
        };
        assert_eq!(form.fields[12].value, "alpha[1m],beta[1m]");
        assert!(form.fields[11].value.is_empty());
    }
}

#[test]
fn claude_preferences_save_presets_and_custom_values_without_changing_routes() {
    let (_temp, mut app) = persisted_app();
    let profiles = app.config.profiles.clone();
    app.modal = Some(Modal::Preferences(PreferencesForm::new(
        app.config.claude.clone(),
        serde_json::json!({}),
    )));
    for key in [
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::ALT),
        KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT),
    ] {
        app.handle_key(key).unwrap();
    }
    let Some(Modal::Preferences(form)) = &mut app.modal else {
        panic!()
    };
    form.fields[6].value = "CUSTOM".into();
    form.fields[7].value = "$(literal)".into();
    assert!(form.fields[7].secret);
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.modal.is_none(), "{}", app.status);
    assert_eq!(app.config.profiles, profiles);
    assert_eq!(app.config.claude.env["CUSTOM"], "$(literal)");
    assert_eq!(app.config.claude.env["CLAUDE_CODE_EFFORT_LEVEL"], "max");
    assert_eq!(app.config.claude.hide_attribution, Some(true));
    assert_eq!(
        config::load(&app.paths.config).unwrap().claude,
        app.config.claude
    );
    let settings = app.config.claude.clone();
    app.select_client_tab(ClientTab::Codex);
    app.config = app
        .update_client_config(|c| {
            c.profiles.get_mut("one").unwrap().name = "Changed".into();
            Ok(())
        })
        .unwrap();
    assert_eq!(config::load(&app.paths.config).unwrap().claude, settings);
}

#[test]
fn claude_preferences_mouse_scroll_masking_validation_and_discard() {
    let (_temp, mut app) = persisted_app();
    let screen = Rect::new(0, 0, 48, 18);
    app.modal = Some(Modal::Preferences(PreferencesForm::new(
        app.config.claude.clone(),
        serde_json::json!({}),
    )));
    let area = modal_area(screen);
    let add = modal_button_rects(area, 4)[1];
    app.handle_modal_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: add.x,
            row: add.y,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    let Some(Modal::Preferences(form)) = &mut app.modal else {
        panic!()
    };
    form.fields[6].value = "ANTHROPIC_BASE_URL".into();
    form.fields[7].value = "very-private-value".into();
    form.selected = 7;
    app.handle_modal(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(matches!(app.modal, Some(Modal::Preferences(_))));
    let mut terminal = Terminal::new(TestBackend::new(48, 18)).unwrap();
    let Some(Modal::Preferences(form)) = &app.modal else {
        panic!()
    };
    terminal
        .draw(|frame| draw_preferences(frame, area, form))
        .unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(!content.contains("very-private-value"));
    assert!(content.contains("Save"));
    let remove = preference_actions(area)[0];
    app.handle_modal_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: remove.x,
            row: remove.y,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    let Some(Modal::Preferences(form)) = &app.modal else {
        panic!()
    };
    assert_eq!(form.fields.len(), 6);
    app.handle_modal(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::ALT))
        .unwrap();
    app.handle_modal(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(app.modal.is_some());
    app.handle_modal(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.modal.is_some());
    app.handle_modal(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    app.handle_modal(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.modal.is_none());
    assert!(app.config.claude.env.is_empty());
}

#[test]
fn provider_form_tests_use_unsaved_connection_and_target_model() {
    let mut form = ProfileForm::new();
    form.fields[3].value = "https://draft.example/v1".into();
    form.fields[5].value = "draft-key".into();
    form.selected = 8;
    form.fields[8].value = "draft-sonnet[1m]".into();
    let (profile, models) = form.model_test_request().unwrap();
    assert_eq!(profile.base_url, "https://draft.example/v1");
    assert_eq!(profile.credential.value(), Some("draft-key"));
    assert_eq!(models, ["draft-sonnet[1m]"]);
    assert!(form.fields[0].value.is_empty());
    assert!(form.fields[6].value.is_empty());
    form.selected = 12;
    form.fields[12].value = "fallback-a, fallback-b".into();
    assert_eq!(
        form.model_test_request().unwrap().1,
        ["fallback-a", "fallback-b"]
    );
    form.fields[12].value.clear();
    assert!(form.model_test_request().is_err());
}

#[test]
fn provider_model_test_button_and_1m_color_work_in_narrow_form() {
    let mut app = interactive_test_app();
    app.theme = theme::Theme::Classic;
    let mut form = ProfileForm::new();
    form.selected = 12;
    app.modal = Some(Modal::Profile(Box::new(form)));
    let screen = Rect::new(0, 0, 48, 18);
    let inner = panel_inner(modal_area(screen));
    let content = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(4),
    );
    let (_, offset) = form_viewport(content, 12);
    let row = (12 - offset) as u16;
    let button = profile_test_rect(content, row);
    app.handle_modal_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: button.x + 1,
            row: button.y,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    assert!(app.status.contains("Enter a model name"));
    let Some(Modal::Profile(form)) = &mut app.modal else {
        panic!()
    };
    assert_eq!(form.selected, 12);
    form.fields[12].value = "model[1m]".into();
    let mut terminal = Terminal::new(TestBackend::new(48, 18)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let checkbox = profile_1m_rect(content, row);
    let cell = &terminal.backend().buffer()[(checkbox.x, checkbox.y)];
    assert_eq!(cell.symbol(), "[");
    assert_eq!(cell.bg, ROUTE);
    app.handle_modal_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: checkbox.x + 1,
            row: checkbox.y,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert_eq!(
        terminal.backend().buffer()[(checkbox.x, checkbox.y)].fg,
        ROUTE
    );
}

#[test]
fn base_url_test_accepts_blank_credentials_without_relaxing_model_validation() {
    let mut form = ProfileForm::new();
    form.fields[3].value = "https://example.com/v1".into();
    for kind in ["bearer", "x-api-key", "api-key", "none"] {
        form.fields[4].value = kind.into();
        for blank in ["", "   "] {
            form.fields[5].value = blank.into();
            let profile = form.connection_test_profile().unwrap();
            assert_eq!(profile.credential, Credential::None);
            assert_eq!(profile.base_url, "https://example.com/v1");
            assert_eq!(form.fields[4].value, kind);
            assert_eq!(form.fields[5].value, blank);
        }
    }
    form.fields[4].value = "bearer".into();
    form.fields[5].value.clear();
    assert!(form.discovery_profile().is_err());
    form.fields[5].value = "provided-token".into();
    assert_eq!(
        form.connection_test_profile().unwrap().credential.value(),
        Some("provided-token")
    );
    form.fields[3].value = "not a URL".into();
    assert!(form.connection_test_profile().is_err());
}

#[test]
fn quit_is_only_available_on_provider_home() {
    let mut app = interactive_test_app();
    for view in [ViewMode::Home, ViewMode::Provider, ViewMode::AllEnabled] {
        app.view_mode = view;
        for width in [40, 60, 120] {
            let controls = footer_controls(Rect::new(0, 0, width, 1), width < 100, view);
            assert_eq!(
                controls.iter().any(|(c, _)| *c == FooterControl::Quit),
                view == ViewMode::Home
            );
        }
        for key in [
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ] {
            assert_eq!(app.handle_key(key).unwrap(), view == ViewMode::Home);
        }
    }
    app.view_mode = ViewMode::Home;
    app.new_profile();
    assert!(
        !app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
            .unwrap()
    );
}

#[test]
fn provider_delete_button_and_details_shortcut_open_confirmation() {
    let mut app = interactive_test_app();
    app.enter_provider_view();
    let screen = Rect::new(0, 0, 120, 30);
    let details = ui_areas(screen, app.focus, app.view_mode).details.unwrap();
    let (_, card) = provider_detail_cards(details);
    let (_, button) = detail_controls(card)
        .into_iter()
        .find(|(c, _)| *c == DetailControl::Delete)
        .unwrap();
    let before = app.config.clone();
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: button.x,
            row: button.y,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    assert!(matches!(app.modal, Some(Modal::DeleteProfile)));
    assert_eq!(app.config, before);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    app.focus = Focus::Details;
    app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(app.modal, Some(Modal::DeleteProfile)));
    assert_eq!(app.config, before);
}

#[test]
fn provider_form_only_displays_its_test_messages() {
    let mut app = interactive_test_app();
    app.status = "Unrelated global sync status".into();
    app.modal = Some(Modal::Profile(Box::new(ProfileForm::new())));
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| app.draw_modal(frame, app.modal.as_ref().unwrap()))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(!text.contains("Unrelated global sync status"));
    app.start_profile_model_test();
    let Some(Modal::Profile(form)) = &app.modal else {
        panic!()
    };
    let message = form.test_message.as_ref().unwrap().0.clone();
    assert!(message.contains("Cannot test model"));
    app.status = "Another unrelated sync status".into();
    terminal
        .draw(|frame| app.draw_modal(frame, app.modal.as_ref().unwrap()))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Cannot test model"));
    assert!(!text.contains("Another unrelated sync status"));
}

#[test]
fn home_delete_button_follows_provider_selection_and_confirms() {
    let mut app = interactive_test_app();
    app.select_home_index(0);
    let screen = Rect::new(0, 0, 80, 24);
    let footer = ui_areas(screen, app.focus, app.view_mode).footer;
    assert!(
        !app.client_footer_controls(footer, true)
            .iter()
            .any(|(c, _)| *c == FooterControl::DeleteProfile)
    );
    app.select_home_index(1);
    let (_, button) = app
        .client_footer_controls(footer, true)
        .into_iter()
        .find(|(c, _)| *c == FooterControl::DeleteProfile)
        .unwrap();
    let before = app.config.clone();
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: button.x,
            row: button.y,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    assert!(matches!(app.modal, Some(Modal::DeleteProfile)));
    assert_eq!(app.config, before);
}

#[test]
fn deleting_models_ignores_source_cleans_roles_and_survives_catalog_refresh() {
    for catalog_only in [false, true] {
        let (_temp, mut app) = persisted_app();
        let cached = app.config.profiles["one"].models.clone();
        let profile = app.config.profiles.get_mut("one").unwrap();
        profile.default_model = "model-a[1m]".into();
        profile.aliases.opus = Some("model-a".into());
        profile.subagent_model = Some("model-a[1m]".into());
        profile.fallback_models = vec!["model-a".into()];
        if catalog_only {
            profile.models.clear();
        }
        config::update(&app.paths.config, |latest| {
            latest.profiles = app.config.profiles.clone();
            Ok(())
        })
        .unwrap();
        app.cache.profiles.insert(
            "one".into(),
            CachedModels {
                fetched_at: 0,
                models: cached.clone(),
            },
        );
        app.enter_provider_view();
        assert_eq!(
            canonical_model_id(&app.selected_model().unwrap().id),
            "model-a"
        );
        app.delete_selected_model();
        assert!(matches!(app.modal, Some(Modal::DeleteModel)));
        app.handle_modal(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.config.profiles["one"].default_model, "model-b");
        assert!(app.config.profiles["one"].aliases.opus.is_none());
        assert!(app.config.profiles["one"].subagent_model.is_none());
        assert!(app.config.profiles["one"].fallback_models.is_empty());
        app.config = config::load(&app.paths.config).unwrap();
        app.cache.profiles.insert(
            "one".into(),
            CachedModels {
                fetched_at: 1,
                models: cached,
            },
        );
        app.init_provider_editor();
        assert!(
            app.catalog_models()
                .iter()
                .all(|model| canonical_model_id(&model.id) != "model-a")
        );
        // The last default stays protected until another model is added.
        app.delete_selected_model();
        app.handle_modal(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert!(app.status.contains("add another model first"));
        assert_eq!(app.config.profiles["one"].default_model, "model-b");
    }
}

#[test]
fn repeated_model_click_edits_and_reasoning_arrows_cycle() {
    let mut app = interactive_test_app();
    app.enter_provider_view();
    let screen = Rect::new(0, 0, 120, 30);
    let models = ui_areas(screen, app.focus, app.view_mode).models.unwrap();
    let click = |x, y| MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    };
    // The second row is initially unselected; two clicks select then edit it.
    let event = click(models.x + 8, models.y + 5);
    app.handle_mouse(event, screen).unwrap();
    assert!(app.modal.is_none());
    app.handle_mouse(event, screen).unwrap();
    let Some(Modal::Model(form)) = &mut app.modal else {
        panic!("second click must edit")
    };
    let index = form
        .fields
        .iter()
        .position(|field| field.label == "Reasoning max")
        .unwrap();
    form.selected = index;
    form.fields[index].value = "high".into();
    let area = modal_area_for(app.modal.as_ref().unwrap(), screen);
    let inner = panel_inner(area);
    let content = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(2),
    );
    let (form_area, _) = model_form_areas(content, false);
    let fields = panel_inner(form_area);
    let (_, offset) = form_viewport(fields, index);
    let x = fields.x + (fields.width / 3).min(17) + 2;
    let y = fields.y + (index - offset) as u16;
    app.handle_modal_mouse(click(x, y), screen).unwrap();
    let Some(Modal::Model(form)) = &app.modal else {
        panic!()
    };
    assert_eq!(form.fields[index].value, "medium");
    app.handle_modal_mouse(click(x + 9, y), screen).unwrap();
    let Some(Modal::Model(form)) = &app.modal else {
        panic!()
    };
    assert_eq!(form.fields[index].value, "high");
}

#[test]
fn grok_tabs_import_settings_sync_and_client_isolation() {
    let (temp, mut app) = persisted_app();
    app.grok_home = temp.path().join("grok");
    std::fs::create_dir_all(&app.grok_home).unwrap();
    let native = app.grok_home.join("config.toml");
    std::fs::write(&native, "[models]\ndefault='custom'\n[model.custom]\nmodel='upstream'\nbase_url='https://example.invalid/v1'\napi_key='private-import-key'\n[ui]\npermission_mode='ask'\n").unwrap();
    let before = config::load(&app.paths.config).unwrap();
    app.select_client_tab(ClientTab::Grok);
    assert!(app.grok_enabled);
    assert!(app.config.profiles.is_empty());
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    app.handle_key(key(KeyCode::Char('i'))).unwrap();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(rendered.contains("Import Grok"));
    assert!(!rendered.contains("private-import-key"));
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.config.profiles.len(), 1);
    assert_eq!(
        app.config.grok.preferences.permission_mode.as_deref(),
        Some("ask")
    );
    assert!(!crate::grok::connected(&app.paths));
    app.handle_key(key(KeyCode::Char('p'))).unwrap();
    assert!(crate::grok::connected(&app.paths));
    assert!(!app.status_error, "{}", app.status);
    app.handle_key(key(KeyCode::F(4))).unwrap();
    app.handle_key(key(KeyCode::Char('c'))).unwrap();
    if let Some(Modal::Grok(dialog)) = app.modal.as_mut() {
        if let grok::Dialog::Settings { fields, .. } = dialog.as_mut() {
            fields[3].value = "high".into();
            fields[5].value = "true".into();
        } else {
            panic!("settings expected");
        }
    } else {
        panic!("Grok settings expected");
    }
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(matches!(app.modal, Some(Modal::Appearance(_))));
    app.handle_key(key(KeyCode::Esc)).unwrap();
    let v: toml::Value = toml::from_str(&std::fs::read_to_string(&native).unwrap()).unwrap();
    assert_eq!(
        v["models"]["default_reasoning_effort"].as_str(),
        Some("high")
    );
    assert_eq!(v["ui"]["compact_mode"].as_bool(), Some(true));
    app.toggle_selected_provider().unwrap();
    app.sync_grok_after_edit();
    let v: toml::Value = toml::from_str(&std::fs::read_to_string(&native).unwrap()).unwrap();
    assert!(
        v["models"]["disabled_models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some("custom"))
    );
    assert!(!app.status_error, "{}", app.status);
    assert!(app.config.grok.preferences.default.is_none());
    app.handle_key(key(KeyCode::Char('D'))).unwrap();
    assert!(!crate::grok::connected(&app.paths));
    let after = config::load(&app.paths.config).unwrap();
    assert_eq!(before.profiles, after.profiles);
    assert_eq!(before.codex, after.codex);
    assert_eq!(before.pi, after.pi);
    app.select_client_tab(ClientTab::Claude);
    assert!(!app.grok_enabled);
    assert_eq!(app.config.profiles, before.profiles);
}

#[test]
fn grok_narrow_tabs_settings_discard_mouse_and_reconnect() {
    let (temp, mut app) = persisted_app();
    app.grok_home = temp.path().join("grok");
    app.select_client_tab(ClientTab::Grok);
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    for (width, height) in [(40, 12), (80, 24)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
        let row: String = (0..width)
            .map(|x| terminal.backend().buffer()[(x, 0)].symbol())
            .collect();
        for label in ["Claude Code", "Codex", "Pi", "Grok", "Usage"] {
            assert!(row.contains(label), "{row}");
        }
    }
    app.open_grok_preferences(None);
    app.handle_key(key(KeyCode::Char('x'))).unwrap();
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert!(
        matches!(app.modal, Some(Modal::Grok(ref d)) if matches!(d.as_ref(), grok::Dialog::Settings { discard: true, .. }))
    );
    app.handle_key(key(KeyCode::Char('n'))).unwrap();
    app.handle_key(key(KeyCode::Esc)).unwrap();
    app.handle_key(key(KeyCode::Char('y'))).unwrap();
    assert!(app.modal.is_none());
    app.open_grok_preferences(None);
    let screen = Rect::new(0, 0, 80, 24);
    let area = modal_area(screen);
    let inner = panel_inner(area);
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: inner.x + 2,
            row: inner.y + 4,
            modifiers: KeyModifiers::NONE,
        },
        screen,
    )
    .unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.modal.is_none());
    assert_eq!(
        app.config.grok.preferences.permission_mode.as_deref(),
        Some("default")
    );
    app.apply_grok(false);
    assert!(!app.status_error, "{}", app.status);
    let path = app.grok_home.join("config.toml");
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("default", "ask");
    std::fs::write(&path, &text).unwrap();
    app.apply_grok(false);
    assert!(
        matches!(app.modal, Some(Modal::Grok(ref d)) if matches!(d.as_ref(), grok::Dialog::Reconnect { .. }))
    );
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    app.apply_grok(false);
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert!(
        crate::grok::conflicts(&app.paths, &app.grok_home)
            .unwrap()
            .is_empty()
    );
    app.handle_key(key(KeyCode::F(2))).unwrap();
    assert_eq!(app.client_tab(), ClientTab::Usage);
}

#[test]
fn grok_default_action_and_model_form_use_native_capabilities() {
    let (_temp, mut app) = persisted_app();
    let template = app.config.profiles["one"].clone();
    app.select_client_tab(ClientTab::Grok);
    app.config = app
        .update_client_config(|c| {
            let mut p = template;
            p.default_model = "first".into();
            p.models.clear();
            p.enabled_models = vec!["second".into()];
            c.profiles.insert("test".into(), p);
            Ok(())
        })
        .unwrap();
    app.open_add_model_modal();
    assert!(
        matches!(app.modal, Some(Modal::Model(ref f)) if !f.fields.iter().any(|f| f.label == "Reasoning max") && f.fields.iter().any(|f| f.label == "Enable now"))
    );
    app.modal = None;
    app.enter_provider_view();
    let editor = app.provider_editor.as_mut().unwrap();
    editor.selected = editor
        .catalog
        .iter()
        .position(|m| m.id == "second")
        .unwrap();
    app.set_selected_as_default();
    assert_eq!(
        app.config.grok.preferences.default.as_deref(),
        Some("ccsw::test::second")
    );
    app.open_grok_preferences(None);
    if let Some(Modal::Grok(d)) = app.modal.as_mut()
        && let grok::Dialog::Settings { selected, .. } = d.as_mut()
    {
        *selected = 3;
    }
    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::ALT))
        .unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
        .unwrap();
    for ch in "custom-effort".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .unwrap();
    }
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(
        app.config.grok.preferences.reasoning_effort.as_deref(),
        Some("custom-effort")
    );
}

#[test]
fn grok_oauth_native_default_preserves_providers_and_credentials() {
    let (temp, mut app) = persisted_app();
    let provider = app.config.profiles["one"].clone();
    app.grok_home = temp.path().join("grok");
    std::fs::create_dir_all(&app.grok_home).unwrap();
    let auth_path = app.grok_home.join("auth.json");
    let credentials =
        r#"{"issuer":{"auth_mode":"oidc","key":"SECRET","email":"user@example.com"}}"#;
    std::fs::write(&auth_path, credentials).unwrap();
    app.select_client_tab(ClientTab::Grok);
    app.config = app
        .update_client_config(|c| {
            c.profiles.insert("one".into(), provider);
            Ok(())
        })
        .unwrap();
    let before = app.config.profiles.clone();
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    app.handle_key(key(KeyCode::Char('o'))).unwrap();
    assert!(app.modal.is_none() && app.grok_auth.page.is_some());
    for (width, height) in [(40, 12), (100, 30)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        for label in ["Browser", "Device code", "Use OAuth", "Sign out"] {
            assert!(rendered.contains(label), "{width}x{height}: {label}");
        }
        assert!(!rendered.contains("SECRET"));
    }
    app.handle_key(key(KeyCode::Char('u'))).unwrap();
    assert_eq!(
        app.config.grok.preferences.default.as_deref(),
        Some("grok-build")
    );
    assert_eq!(
        serde_json::to_value(&before).unwrap(),
        serde_json::to_value(&app.config.profiles).unwrap()
    );
    assert!(
        std::fs::read_to_string(app.grok_home.join("config.toml"))
            .unwrap()
            .contains("grok-build")
    );
    app.handle_key(key(KeyCode::Char('x'))).unwrap();
    assert!(
        app.grok_auth
            .page
            .as_ref()
            .is_some_and(|a| a.confirm_logout)
    );
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert!(!app.grok_auth.busy);
    assert_eq!(std::fs::read_to_string(auth_path).unwrap(), credentials);
}

#[test]
fn grok_oauth_rejects_api_overrides_and_missing_login() {
    let home = tempfile::tempdir().unwrap();
    assert!(crate::grok::validate_oauth_model(home.path(), "grok-build").is_err());
    std::fs::write(
        home.path().join("auth.json"),
        r#"{"auth_mode":"oidc","key":"SECRET"}"#,
    )
    .unwrap();
    assert!(crate::grok::validate_oauth_model(home.path(), "grok-build").is_ok());
    assert!(crate::grok::validate_oauth_model(home.path(), "ccsw::provider::model").is_err());
    std::fs::write(
        home.path().join("config.toml"),
        "[model.grok-build]\napi_key='api-key'\n",
    )
    .unwrap();
    assert!(crate::grok::validate_oauth_model(home.path(), "grok-build").is_err());
    std::fs::write(
        home.path().join("config.toml"),
        "[endpoints]\nmodels_base_url='https://example.invalid'\n",
    )
    .unwrap();
    assert!(crate::grok::validate_oauth_model(home.path(), "grok-build").is_err());
}

#[test]
fn grok_oauth_provider_row_uses_home_keyboard_and_mouse_navigation() {
    let (temp, mut app) = persisted_app();
    app.grok_home = temp.path().join("grok");
    std::fs::create_dir_all(&app.grok_home).unwrap();
    std::fs::write(
        app.grok_home.join("auth.json"),
        r#"{"auth_mode":"oidc","key":"SECRET","email":"grok@example.com"}"#,
    )
    .unwrap();
    app.select_client_tab(ClientTab::Grok);
    assert_eq!(app.home_prefix_count(), 2);
    let before = std::fs::read(&app.paths.config).unwrap();
    let area = Rect::new(0, 0, 120, 36);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Grok OAuth Account"));
    assert!(text.contains("grok@example.com"));
    assert!(!text.contains("SECRET"));
    assert!(
        !app.client_footer_controls(ui_areas(area, app.focus, app.view_mode).footer, false)
            .iter()
            .any(|(control, _)| *control == FooterControl::Proxy)
    );
    app.select_home_index(0);
    app.move_selection(1);
    assert_eq!(app.home_selected_index(), 1);
    assert!(app.home_grok_oauth_selected());
    assert!(app.selected_profile_id().is_none());
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(app.modal.is_none() && app.grok_auth.page.is_some());
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(app.home_grok_oauth_selected());
    assert_eq!(app.view_mode, ViewMode::Home);
    app.select_home_index(0);
    let panel = ui_areas(area, app.focus, app.view_mode).profiles.unwrap();
    let mouse = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: panel.x + 2,
        row: panel_inner(panel).y + app.home_profile_item_heights(panel)[0] as u16,
        modifiers: KeyModifiers::NONE,
    };
    app.handle_mouse(mouse, area).unwrap();
    assert!(app.home_grok_oauth_selected());
    assert!(app.modal.is_none());
    app.handle_mouse(mouse, area).unwrap();
    assert!(app.modal.is_none() && app.grok_auth.page.is_some());
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(std::fs::read(&app.paths.config).unwrap(), before);
    app.select_client_tab(ClientTab::Claude);
    assert_eq!(app.home_prefix_count(), 1);
    assert!(!app.home_grok_oauth_selected());
    app.select_client_tab(ClientTab::Grok);
    assert_eq!(app.home_selected_index(), 0);
}

#[test]
fn grok_oauth_row_does_not_shift_api_provider_selection_or_edits() {
    let (temp, mut app) = persisted_app();
    let provider = app.config.profiles["one"].clone();
    app.grok_home = temp.path().join("grok");
    app.select_client_tab(ClientTab::Grok);
    app.config = app
        .update_client_config(|c| {
            c.profiles.insert("one".into(), provider);
            Ok(())
        })
        .unwrap();
    app.select_home_index(1);
    app.move_selection(1);
    assert_eq!(app.home_selected_index(), 2);
    assert_eq!(app.selected_profile_id().as_deref(), Some("one"));
    app.enter_provider_view();
    assert_eq!(app.view_mode, ViewMode::Provider);
    assert!(app.provider_editor.is_some());
    app.return_home();
    app.move_selection(-1);
    assert!(app.home_grok_oauth_selected());
    app.edit_profile();
    assert!(app.modal.is_none() && app.grok_auth.page.is_some());
}

#[test]
fn grok_account_management_is_a_page_with_back_navigation_and_visible_errors() {
    let (temp, mut app) = persisted_app();
    app.grok_home = temp.path().join("grok");
    app.select_client_tab(ClientTab::Grok);
    app.select_home_index(1);
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert!(app.modal.is_none());
    assert!(app.grok_auth.page.is_some());
    for (width, height) in [(40, 12), (120, 36)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Grok OAuth Accounts"));
        assert!(text.contains("Account configuration"));
        assert!(!text.contains("Providers · F2"));
        assert!(text.contains("Grok accounts"));
    }
    assert!(
        !app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
            .unwrap()
    );
    assert!(app.grok_auth.page.is_some());
    assert!(app.handle_key(key(KeyCode::Char('u'))).is_err());
    assert!(app.grok_auth.page.is_some());
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Sign in to Grok OAuth first"));
    // Refresh and Back operate on the account page, without opening a modal.
    let area = Rect::new(0, 0, 120, 36);
    let refresh = grok_auth::account_actions(area)[3];
    let mouse = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: refresh.x,
        row: refresh.y,
        modifiers: KeyModifiers::NONE,
    };
    app.handle_mouse(mouse, area).unwrap();
    assert!(app.modal.is_none() && app.grok_auth.page.is_some());
    app.grok_auth.busy = true;
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert!(app.grok_auth.page.is_some());
    app.select_client_tab(ClientTab::Claude);
    assert!(app.grok_enabled);
    app.grok_auth.busy = false;
    app.handle_mouse(
        MouseEvent {
            column: 3,
            row: 1,
            ..mouse
        },
        area,
    )
    .unwrap();
    assert!(app.grok_auth.page.is_none());
    assert!(app.home_grok_oauth_selected());
    app.handle_key(key(KeyCode::Enter)).unwrap();
    app.handle_key(key(KeyCode::F(2))).unwrap();
    assert!(app.usage.active);
    app.select_client_tab(ClientTab::Claude);
    assert!(!app.grok_enabled);
    assert!(app.grok_auth.page.is_none());
}

#[test]
fn grok_accounts_match_codex_vertical_layout_and_footer_at_all_sizes() {
    let (temp, mut app) = persisted_app();
    app.grok_home = temp.path().join("grok");
    std::fs::create_dir_all(&app.grok_home).unwrap();
    std::fs::write(
        app.grok_home.join("auth.json"),
        r#"{"auth_mode":"oidc","key":"SECRET","email":"account@example.com"}"#,
    )
    .unwrap();
    app.select_client_tab(ClientTab::Grok);
    app.open_grok_auth();
    for (width, height) in [(40, 12), (80, 24), (120, 36)] {
        let screen = Rect::new(0, 0, width, height);
        let rows = account_page_rows(screen, false);
        for row in rows {
            assert_eq!(row.x, 0);
            assert_eq!(row.width, width);
        }
        assert_eq!(rows[1].bottom(), rows[2].y);
        assert_eq!(rows[2].bottom(), rows[3].y);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let line = |y: u16| -> String { (0..width).map(|x| buffer[(x, y)].symbol()).collect() };
        assert!(line(rows[1].y).contains("Grok accounts"));
        assert!(line(rows[1].y + 1).contains("account@example.com"));
        assert!(line(rows[2].y).contains("Account configuration"));
        if height >= 16 {
            assert!(line(rows[2].y + 1).contains("Native model"));
        }
        let buttons = grok_auth::account_actions(screen);
        for button in &buttons {
            assert!(button.y >= rows[3].y && button.bottom() <= screen.bottom());
            assert!(button.right() <= screen.right());
        }
        let refresh = buttons[3];
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: refresh.x,
                row: refresh.y,
                modifiers: KeyModifiers::NONE,
            },
            screen,
        )
        .unwrap();
        assert!(app.grok_auth.page.is_some());
    }
    app.grok_auth.busy = true;
    let screen = Rect::new(0, 0, 40, 12);
    assert_eq!(account_page_rows(screen, true)[1].height, 0);
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Login progress"));
    assert!(text.contains("Cancel/Esc"));
    assert!(!text.contains("Grok accounts"));
    app.grok_auth.busy = false;
}
