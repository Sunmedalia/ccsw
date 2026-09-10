use super::*;
use ratatui::{Terminal, backend::TestBackend};
use std::path::PathBuf;

#[test]
fn renders_empty_state_in_narrow_terminal() {
    let paths = AppPaths {
        config: PathBuf::from("/tmp/config"),
        state_dir: PathBuf::from("/tmp/state"),
        cache: PathBuf::from("/tmp/cache"),
    };
    let mut app = App {
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
        codex_ui: codex::CodexUi::default(),
        pi_enabled: false,
        background: Background::default(),
        screen: Rect::new(0, 0, 80, 24),
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
    assert!(rendered.contains("q / Ctrl+C to quit"));
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
            id: "default-m".into(),
            label: None,
            description: None,
        },
        ModelEntry {
            max_output_tokens: None,
            context_window: None,
            id: "m-1".into(),
            label: None,
            description: None,
        },
        ModelEntry {
            max_output_tokens: None,
            context_window: None,
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
    let controls = showcase_controls(showcase_card);

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

    // 2. The details card only keeps provider-specific refresh/edit actions.
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
    let lines = home_profile_lines("one", profile, 2, 42);
    assert!(lines.len() > 5);
    assert!(lines.iter().all(|line| line.width() <= 42));
}

#[test]
fn model_form_api_model_picker_populates_fields() {
    let api_models = vec![
        ModelEntry {
            max_output_tokens: None,
            context_window: None,
            id: "qwen-max-latest".into(),
            label: Some("Qwen Max Latest".into()),
            description: Some("Alibaba Cloud flagship model".into()),
        },
        ModelEntry {
            max_output_tokens: None,
            context_window: None,
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
        codex_ui: codex::CodexUi::default(),
        pi_enabled: false,
        background: Background::default(),
        screen: Rect::new(0, 0, 80, 24),
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
    config::update(&app.paths.config, |latest| {
        *latest = app.config.clone();
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
    for _ in 0..7 {
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
            id: "a[1m]".into(),
            label: None,
            description: Some("Old description".into()),
        },
        ModelEntry {
            max_output_tokens: None,
            context_window: None,
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
        form.fields[5].value = invalid.into();
        assert!(form.validate_tokens().is_err());
    }
    form.fields[5].value = "8192".into();
    form.fields[6].value = "4096".into();
    assert!(form.validate_tokens().is_err());
    form.fields[6].value = "32768".into();
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
    form.fields[5].value = "8192".into();
    form.fields[6].value = "32768".into();
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
        assert!(text.contains("Codex Accounts"));
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
        assert!(text.contains("Pi API"));
    }
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
    for (width, height) in [(40, 12), (80, 24), (120, 36)] {
        for tab in [ClientTab::Claude, ClientTab::Codex, ClientTab::Pi] {
            app.select_client_tab(tab);
            for accounts in [false, true] {
                app.codex_ui.accounts = accounts;
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|frame| app.draw(frame)).unwrap();
                let buffer = terminal.backend().buffer();
                let first_row = (0..width)
                    .map(|x| buffer[(x, 0)].symbol())
                    .collect::<String>();
                for label in ["Claude Code", "Codex", "Pi"] {
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
