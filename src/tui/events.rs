use super::*;

impl App {
    pub(super) fn event_loop(&mut self, terminal: &mut TuiTerminal) -> Result<()> {
        self.initialize_background();
        let mut closing = false;
        let mut redraw = true;
        loop {
            redraw |= self.poll_background();
            if redraw {
                terminal.draw(|frame| self.draw(frame))?;
                redraw = false;
            }
            if closing {
                if !self.background.sync_running
                    && !self.background.proxy_running
                    && self.background.queued_sync.is_none()
                {
                    return Ok(());
                }
                if self.status != "Finishing pending changes…" {
                    self.status = "Finishing pending changes…".into();
                    redraw = true;
                }
                std::thread::sleep(std::time::Duration::from_millis(40));
                continue;
            }
            if !event::poll(std::time::Duration::from_millis(50))? {
                continue;
            }
            let input = event::read()?;
            redraw = true;
            if self.screen.width < 40 || self.screen.height < 12 {
                if matches!(input, Event::Key(key) if key.code == KeyCode::Char('q') || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)))
                {
                    closing = true;
                }
                continue;
            }
            let result = match input {
                Event::Key(key) if key.kind == event::KeyEventKind::Press => self.handle_key(key),
                Event::Mouse(mouse) => {
                    let size = terminal.size()?;
                    self.handle_mouse(mouse, Rect::new(0, 0, size.width, size.height))
                        .map(|action| action == MouseAction::Quit)
                }
                _ => Ok(false),
            };
            match result {
                Ok(quit) => closing = quit,
                Err(error) => {
                    self.reload_for_edit();
                    self.init_provider_editor();
                    self.set_error(format!("Could not save changes: {error:#}"));
                }
            }
        }
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        let before = self.config.clone();
        let result = self.handle_key_inner(key);
        if before != self.config {
            self.queue_sync(false, None);
        }
        result
    }

    pub(super) fn handle_key_inner(&mut self, key: KeyEvent) -> Result<bool> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Ok(true);
        }
        if self.modal.is_some() {
            self.handle_modal(key)?;
            return Ok(false);
        }
        match self.view_mode {
            ViewMode::Home => match key.code {
                KeyCode::Char('q') => return Ok(true),
                KeyCode::Char('?') => self.open_help(),
                KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
                KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
                KeyCode::Enter if self.home_all_selected => {
                    self.enter_all_enabled_view();
                }
                KeyCode::Enter if self.selected_profile().is_some() => {
                    self.enter_provider_view();
                }
                KeyCode::Char('n') => self.new_profile(),
                KeyCode::Char('e') | KeyCode::Char('E') => self.edit_profile(),
                KeyCode::Char('x') if self.selected_profile().is_some() => {
                    self.modal = Some(Modal::DeleteProfile);
                }
                KeyCode::Char('r') | KeyCode::Char('t') => self.refresh_models(),
                KeyCode::Char(' ') if self.selected_profile().is_some() => {
                    self.toggle_selected_provider()?;
                }
                KeyCode::Char('A') => self.enable_all_models(),
                KeyCode::Char('p') => self.sync_all_to_claude(),
                KeyCode::Char('P') => self.open_proxy_manager(),
                _ => {}
            },
            ViewMode::AllEnabled => match key.code {
                KeyCode::Char('q') => return Ok(true),
                KeyCode::Char('?') => self.open_help(),
                KeyCode::Esc => self.return_home(),
                KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
                KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
                KeyCode::PageUp => {
                    self.model_idx = self.model_idx.saturating_sub(10);
                }
                KeyCode::PageDown => {
                    self.model_idx = (self.model_idx + 10)
                        .min(self.all_managed_models().len().saturating_sub(1));
                }
                KeyCode::Home => self.model_idx = 0,
                KeyCode::End => {
                    self.model_idx = self.all_managed_models().len().saturating_sub(1);
                }
                KeyCode::Char(' ') => {
                    self.toggle_selected_global_model()?;
                }
                KeyCode::Enter => self.open_selected_global_model(),
                KeyCode::Char('p') => self.sync_all_to_claude(),
                KeyCode::Char('P') => self.open_proxy_manager(),
                _ => {}
            },
            ViewMode::Provider => {
                let mut search_active = false;
                if let Some(editor) = &self.provider_editor {
                    search_active = editor.search_active;
                }
                if search_active {
                    let editor = self.ensure_provider_editor().unwrap();
                    match key.code {
                        KeyCode::Esc => {
                            if !editor.query.is_empty() {
                                editor.query.clear();
                                editor.selected = 0;
                            } else {
                                editor.search_active = false;
                            }
                        }
                        KeyCode::Char(c) => {
                            editor.query.push(c);
                            editor.selected = 0;
                        }
                        KeyCode::Backspace => {
                            editor.query.pop();
                            editor.selected = 0;
                        }
                        KeyCode::Tab | KeyCode::Down => {
                            editor.search_active = false;
                        }
                        KeyCode::Enter => {
                            editor.toggle_selected();
                            self.commit_provider_editor()?;
                        }
                        KeyCode::Up if editor.selected > 0 => {
                            editor.selected -= 1;
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Char('q') => return Ok(true),
                        KeyCode::Char('?') => self.open_help(),
                        KeyCode::Esc => {
                            self.return_home();
                        }
                        KeyCode::Tab | KeyCode::BackTab => self.toggle_focus(),
                        KeyCode::Left | KeyCode::Char('h') => {
                            self.focus = Focus::Models;
                        }
                        KeyCode::Right | KeyCode::Char('l') => {
                            self.focus = Focus::Details;
                        }
                        KeyCode::Char('/') => {
                            if let Some(editor) = self.ensure_provider_editor() {
                                editor.search_active = true;
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            let name = if let Some(editor) = self.ensure_provider_editor() {
                                let filtered = editor.filtered_indices();
                                if !filtered.is_empty() {
                                    if editor.selected > 0 {
                                        editor.selected -= 1;
                                    } else {
                                        editor.selected = filtered.len().saturating_sub(1);
                                    }
                                    filtered
                                        .get(editor.selected)
                                        .map(|&idx| editor.catalog[idx].label().to_string())
                                } else {
                                    None
                                }
                            } else {
                                None
                            };
                            if let Some(name) = name {
                                self.status_error = false;
                                self.status =
                                    format!("Selected {name} · Space toggle · d default · 1 1M");
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            let name = if let Some(editor) = self.ensure_provider_editor() {
                                let filtered = editor.filtered_indices();
                                if !filtered.is_empty() {
                                    if editor.selected + 1 < filtered.len() {
                                        editor.selected += 1;
                                    } else {
                                        editor.selected = 0;
                                    }
                                    filtered
                                        .get(editor.selected)
                                        .map(|&idx| editor.catalog[idx].label().to_string())
                                } else {
                                    None
                                }
                            } else {
                                None
                            };
                            if let Some(name) = name {
                                self.status_error = false;
                                self.status =
                                    format!("Selected {name} · Space toggle · d default · 1 1M");
                            }
                        }
                        KeyCode::PageUp => {
                            if let Some(editor) = self.ensure_provider_editor() {
                                editor.selected = editor.selected.saturating_sub(10);
                            }
                        }
                        KeyCode::PageDown => {
                            if let Some(editor) = self.ensure_provider_editor() {
                                let filtered = editor.filtered_indices();
                                editor.selected =
                                    (editor.selected + 10).min(filtered.len().saturating_sub(1));
                            }
                        }
                        KeyCode::Home => {
                            if let Some(editor) = self.ensure_provider_editor() {
                                editor.selected = 0;
                            }
                        }
                        KeyCode::End => {
                            if let Some(editor) = self.ensure_provider_editor() {
                                let filtered = editor.filtered_indices();
                                editor.selected = filtered.len().saturating_sub(1);
                            }
                        }
                        KeyCode::Char(' ') => {
                            if let Some(editor) = self.ensure_provider_editor() {
                                editor.toggle_selected();
                            }
                            self.commit_provider_editor()?;
                            if let Some(editor) = &self.provider_editor {
                                let filtered = editor.filtered_indices();
                                if let Some(&idx) = filtered.get(editor.selected) {
                                    let model = &editor.catalog[idx];
                                    let enabled = editor.is_enabled(&model.id);
                                    self.status = format!(
                                        "Model {} is now {}",
                                        model.label(),
                                        if enabled { "enabled" } else { "disabled" }
                                    );
                                }
                            }
                        }
                        KeyCode::Char('1') => {
                            self.toggle_selected_model_1m();
                        }
                        KeyCode::Char('d') => {
                            self.set_selected_as_default();
                        }
                        KeyCode::Char('A') => {
                            if let Some(editor) = self.ensure_provider_editor() {
                                editor.enable_all_filtered();
                            }
                            self.commit_provider_editor()?;
                            self.status = "Enabled all filtered models".into();
                        }
                        KeyCode::Char('C') => {
                            if let Some(editor) = self.ensure_provider_editor() {
                                editor.disable_all_filtered();
                            }
                            self.commit_provider_editor()?;
                            self.status = "Cleared non-essential enabled models".into();
                        }
                        KeyCode::Char('a') if self.selected_profile().is_some() => {
                            self.open_add_model_modal();
                        }
                        KeyCode::Char('x') => {
                            self.delete_selected_model();
                        }
                        KeyCode::Char('e') => self.edit_model(),
                        KeyCode::Char('E') => self.edit_profile(),
                        KeyCode::Char('r') | KeyCode::Char('t') => {
                            self.refresh_models();
                            self.init_provider_editor();
                        }
                        KeyCode::Char('p') => self.sync_all_to_claude(),
                        KeyCode::Char('P') => self.open_proxy_manager(),
                        _ => {}
                    }
                }
            }
        }
        Ok(false)
    }

    pub(super) fn handle_mouse(&mut self, mouse: MouseEvent, area: Rect) -> Result<MouseAction> {
        let before = self.config.clone();
        let result = self.handle_mouse_inner(mouse, area);
        if before != self.config {
            self.queue_sync(false, None);
        }
        result
    }

    pub(super) fn handle_mouse_inner(
        &mut self,
        mouse: MouseEvent,
        area: Rect,
    ) -> Result<MouseAction> {
        if self.modal.is_some() {
            self.handle_modal_mouse(mouse, area)?;
            return Ok(MouseAction::None);
        }

        let ui = ui_areas(area, self.focus, self.view_mode);
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let delta = if mouse.kind == MouseEventKind::ScrollUp {
                    -1
                } else {
                    1
                };
                if ui
                    .profiles
                    .is_some_and(|panel| contains(panel, mouse.column, mouse.row))
                {
                    self.focus = Focus::Profiles;
                    self.move_selection(delta);
                } else if ui
                    .models
                    .is_some_and(|panel| contains(panel, mouse.column, mouse.row))
                {
                    self.focus = Focus::Models;
                    if self.view_mode == ViewMode::Provider {
                        if let Some(editor) = self.ensure_provider_editor() {
                            let filtered = editor.filtered_indices();
                            if !filtered.is_empty() {
                                if delta < 0 {
                                    if editor.selected > 0 {
                                        editor.selected -= 1;
                                    }
                                } else if editor.selected + 1 < filtered.len() {
                                    editor.selected += 1;
                                }
                            }
                        }
                    } else {
                        self.move_selection(delta);
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) => {
                if self.view_mode != ViewMode::Home && mouse.row <= 2 && mouse.column <= 16 {
                    self.return_home();
                    return Ok(MouseAction::None);
                }

                if let Some(panel) = ui.profiles {
                    let visible = if self.view_mode == ViewMode::Home {
                        visible_variable_items(
                            &self.home_profile_item_heights(panel),
                            self.profile_offset,
                            usize::from(panel.height.saturating_sub(2)),
                        )
                    } else {
                        usize::from(panel.height.saturating_sub(2))
                    };
                    if let Some(index) = scrollbar_index(
                        panel,
                        mouse.column,
                        mouse.row,
                        if self.view_mode == ViewMode::Home {
                            self.config.profiles.len().saturating_add(1)
                        } else {
                            self.config.profiles.len()
                        },
                        visible,
                    ) {
                        self.focus = Focus::Profiles;
                        if self.view_mode == ViewMode::Home {
                            self.select_home_index(index);
                        } else {
                            self.profile_idx = index;
                            self.model_idx = self.default_model_index();
                            self.model_offset = 0;
                        }
                        return Ok(MouseAction::None);
                    }
                }
                if let Some(panel) = ui.models {
                    if self.view_mode == ViewMode::Provider {
                        let search_area = Rect::new(panel.x, panel.y, panel.width, 3);
                        let list_area = Rect::new(
                            panel.x,
                            panel.y + 3,
                            panel.width,
                            panel.height.saturating_sub(3),
                        );
                        if let Some(btn_rect) = catalog_add_button_rect(search_area)
                            && contains(btn_rect, mouse.column, mouse.row)
                        {
                            self.open_add_model_modal();
                            return Ok(MouseAction::None);
                        }
                        if contains(search_area, mouse.column, mouse.row) {
                            self.focus = Focus::Models;
                            if let Some(editor) = self.ensure_provider_editor() {
                                editor.search_active = true;
                            }
                            return Ok(MouseAction::None);
                        }
                        if contains(list_area, mouse.column, mouse.row) {
                            self.focus = Focus::Models;
                            let clicked = if let Some(editor) = self.ensure_provider_editor() {
                                let offset =
                                    route_editor_offset(editor, list_area.height.saturating_sub(2));
                                let inner_y = list_area.y + 1;
                                if mouse.row >= inner_y {
                                    let index =
                                        offset + usize::from(mouse.row.saturating_sub(inner_y));
                                    let filtered = editor.filtered_indices();
                                    if index < filtered.len() {
                                        editor.selected = index;
                                        let should_toggle = mouse.column < list_area.x + 4;
                                        if should_toggle {
                                            editor.toggle_selected();
                                        } else {
                                            editor.search_active = false;
                                        }
                                        Some((index, should_toggle))
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            };
                            if let Some((index, should_toggle)) = clicked {
                                self.model_idx = index;
                                if should_toggle {
                                    self.commit_provider_editor()?;
                                }
                                return Ok(MouseAction::None);
                            }
                        }
                    } else {
                        let models = self.models();
                        if let Some(index) = scrollbar_index(
                            panel,
                            mouse.column,
                            mouse.row,
                            models.len(),
                            usize::from(panel.height.saturating_sub(2) / 2),
                        ) {
                            self.focus = Focus::Models;
                            self.model_idx = index;
                            self.status_error = false;
                            self.status = format!(
                                "Selected {} · Space toggles availability",
                                models[index].label()
                            );
                            return Ok(MouseAction::None);
                        }
                    }
                }

                if mouse.kind == MouseEventKind::Drag(MouseButton::Left) {
                    return Ok(MouseAction::None);
                }
                for (control, rect) in footer_controls(ui.footer, area.width < 100, self.view_mode)
                {
                    if !contains(rect, mouse.column, mouse.row) {
                        continue;
                    }
                    return Ok(match control {
                        FooterControl::Back => {
                            self.return_home();
                            MouseAction::None
                        }
                        FooterControl::Models => {
                            self.focus = Focus::Models;
                            MouseAction::None
                        }
                        FooterControl::Details => {
                            self.focus = Focus::Details;
                            MouseAction::None
                        }
                        FooterControl::AddProfile => {
                            self.new_profile();
                            MouseAction::None
                        }
                        FooterControl::Sync => {
                            self.sync_all_to_claude();
                            MouseAction::None
                        }
                        FooterControl::Proxy => {
                            self.open_proxy_manager();
                            MouseAction::None
                        }
                        FooterControl::Help => {
                            self.open_help();
                            MouseAction::None
                        }
                        FooterControl::Quit => MouseAction::Quit,
                    });
                }

                if let Some(panel) = ui.profiles {
                    let index = if self.view_mode == ViewMode::Home {
                        clicked_variable_item(
                            panel,
                            mouse.row,
                            self.profile_offset,
                            &self.home_profile_item_heights(panel),
                        )
                    } else {
                        clicked_list_index(panel, mouse.column, mouse.row, self.profile_offset, 1)
                    };
                    let item_count = if self.view_mode == ViewMode::Home {
                        self.config.profiles.len().saturating_add(1)
                    } else {
                        self.config.profiles.len()
                    };
                    if let Some(index) = index.filter(|index| *index < item_count) {
                        self.focus = Focus::Profiles;
                        if self.view_mode == ViewMode::Home {
                            if index > 0 && mouse.column < panel.x.saturating_add(5) {
                                self.select_home_index(index);
                                self.toggle_selected_provider()?;
                                return Ok(MouseAction::None);
                            }
                            if self.home_selected_index() == index {
                                if index == 0 {
                                    self.enter_all_enabled_view();
                                } else {
                                    self.enter_provider_view();
                                }
                            } else {
                                self.select_home_index(index);
                            }
                        } else {
                            self.profile_idx = index;
                            self.model_idx = self.default_model_index();
                            self.model_offset = 0;
                        }
                    }
                } else if let Some(panel) = ui.models
                    && self.view_mode != ViewMode::Provider
                    && let Some(index) =
                        clicked_list_index(panel, mouse.column, mouse.row, self.model_offset, 2)
                {
                    if self.view_mode == ViewMode::AllEnabled {
                        let models = self.all_managed_models();
                        if index < models.len() {
                            let was_selected = self.model_idx == index;
                            self.focus = Focus::Models;
                            self.model_idx = index;
                            if mouse.column < panel.x.saturating_add(4) {
                                self.toggle_selected_global_model()?;
                                return Ok(MouseAction::None);
                            }
                            if was_selected {
                                self.open_selected_global_model();
                            } else {
                                let entry = &models[index];
                                self.status_error = false;
                                self.status = format!(
                                    "{} · {} · selected · Enter or click again to open provider",
                                    entry.profile_name,
                                    entry.model.label()
                                );
                            }
                            return Ok(MouseAction::None);
                        }
                    } else if index < self.models().len() {
                        self.focus = Focus::Models;
                        self.model_idx = index;
                        if let Some(model) = self.selected_model() {
                            self.status_error = false;
                            self.status =
                                format!("Selected {} · Space toggles availability", model.label());
                        }
                    }
                } else if let Some(details) = ui.details
                    && contains(details, mouse.column, mouse.row)
                {
                    self.focus = Focus::Details;
                    if self.view_mode == ViewMode::Provider {
                        let (showcase_card, provider_card) = provider_detail_cards(details);
                        if contains(showcase_card, mouse.column, mouse.row) {
                            if let Some((control, _)) = showcase_controls(showcase_card)
                                .into_iter()
                                .find(|(_, rect)| contains(*rect, mouse.column, mouse.row))
                            {
                                match control {
                                    ShowcaseControl::Toggle => {
                                        if let Some(editor) = self.ensure_provider_editor() {
                                            editor.toggle_selected();
                                        }
                                        self.commit_provider_editor()?;
                                        if let Some(editor) = &self.provider_editor {
                                            let filtered = editor.filtered_indices();
                                            if let Some(&idx) = filtered.get(editor.selected) {
                                                let model = &editor.catalog[idx];
                                                let enabled = editor.is_enabled(&model.id);
                                                self.status = format!(
                                                    "Model {} is now {}",
                                                    model.label(),
                                                    if enabled { "enabled" } else { "disabled" }
                                                );
                                            }
                                        }
                                        return Ok(MouseAction::None);
                                    }
                                    ShowcaseControl::Default => {
                                        self.set_selected_as_default();
                                        return Ok(MouseAction::None);
                                    }
                                    ShowcaseControl::OneM => {
                                        self.toggle_selected_model_1m();
                                        return Ok(MouseAction::None);
                                    }
                                    ShowcaseControl::Delete => {
                                        self.delete_selected_model();
                                        return Ok(MouseAction::None);
                                    }
                                }
                            }
                        } else if contains(provider_card, mouse.column, mouse.row)
                            && let Some((control, _)) = detail_controls(provider_card)
                                .into_iter()
                                .find(|(_, rect)| contains(*rect, mouse.column, mouse.row))
                        {
                            match control {
                                DetailControl::FetchModels => {
                                    self.refresh_models();
                                    self.init_provider_editor();
                                }
                                DetailControl::Edit => {
                                    self.edit_profile();
                                }
                            }
                            return Ok(MouseAction::None);
                        }
                    } else if let Some((control, _)) = detail_controls(details)
                        .into_iter()
                        .find(|(_, rect)| contains(*rect, mouse.column, mouse.row))
                    {
                        match control {
                            DetailControl::FetchModels => {
                                self.refresh_models();
                            }
                            DetailControl::Edit => {
                                self.edit_profile();
                            }
                        }
                        return Ok(MouseAction::None);
                    }
                }
            }
            _ => {}
        }
        Ok(MouseAction::None)
    }

    pub(super) fn handle_modal_mouse(&mut self, mouse: MouseEvent, screen: Rect) -> Result<()> {
        let Some(modal) = self.modal.as_ref() else {
            return Ok(());
        };
        let area = modal_area_for(modal, screen);
        if matches!(modal, Modal::Proxy(manager) if manager.port_field.is_some()) {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                let buttons = modal_button_rects(area, 2);
                if let Some(index) = buttons
                    .iter()
                    .position(|rect| contains(*rect, mouse.column, mouse.row))
                {
                    self.handle_modal(KeyEvent::new(
                        if index == 0 {
                            KeyCode::Enter
                        } else {
                            KeyCode::Esc
                        },
                        KeyModifiers::NONE,
                    ))?;
                }
            }
            return Ok(());
        }
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if let Some(Modal::Model(form)) = self.modal.as_mut() {
                    let inner = panel_inner(area);
                    let content_area = Rect::new(
                        inner.x,
                        inner.y,
                        inner.width,
                        inner.height.saturating_sub(2),
                    );
                    let (_, api_area) = model_form_areas(content_area, form.focus_api_search);
                    let visible_height =
                        usize::from(panel_inner(api_area).height.saturating_sub(2));
                    form.scroll_api_list(false, 3, visible_height);
                    return Ok(());
                }
                self.handle_modal(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))?;
                return Ok(());
            }
            MouseEventKind::ScrollDown => {
                if let Some(Modal::Model(form)) = self.modal.as_mut() {
                    let inner = panel_inner(area);
                    let content_area = Rect::new(
                        inner.x,
                        inner.y,
                        inner.width,
                        inner.height.saturating_sub(2),
                    );
                    let (_, api_area) = model_form_areas(content_area, form.focus_api_search);
                    let visible_height =
                        usize::from(panel_inner(api_area).height.saturating_sub(2));
                    form.scroll_api_list(true, 3, visible_height);
                    return Ok(());
                }
                self.handle_modal(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))?;
                return Ok(());
            }
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) => {}
            _ => return Ok(()),
        }
        if !contains(area, mouse.column, mouse.row) {
            return Ok(());
        }

        if matches!(self.modal, Some(Modal::Proxy(_))) {
            if mouse.kind == MouseEventKind::Drag(MouseButton::Left) {
                return Ok(());
            }
            if let Some((control, _)) = proxy_controls(area)
                .into_iter()
                .find(|(_, rect)| contains(*rect, mouse.column, mouse.row))
            {
                self.handle_modal(proxy_control_key(control))?;
                return Ok(());
            }
        } else if mouse.kind == MouseEventKind::Drag(MouseButton::Left) {
            return Ok(());
        }

        let button_count = match self.modal.as_ref() {
            Some(Modal::Help(_)) => 1,
            Some(Modal::Proxy(_)) => 0,
            Some(Modal::Model(_)) => 3,
            Some(_) => 2,
            None => 0,
        };
        if let Some(button) = modal_button_rects(area, button_count)
            .iter()
            .position(|rect| contains(*rect, mouse.column, mouse.row))
        {
            let key = match (self.modal.as_ref(), button) {
                (Some(Modal::Model(_)), 0) => {
                    self.fetch_api_models_for_form();
                    return Ok(());
                }
                (Some(Modal::Model(_)), 1) => {
                    KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
                }
                (Some(Modal::Model(_)), 2) => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                (Some(Modal::Import(_)), 0)
                | (Some(Modal::DeleteProfile | Modal::DeleteModel), 0) => {
                    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
                }
                (Some(Modal::Profile(_)), 0) => {
                    KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
                }
                (Some(Modal::Help(_)), 0) => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                (Some(_), 1) => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                _ => return Ok(()),
            };
            self.handle_modal(key)?;
            return Ok(());
        }

        let inner = panel_inner(area);
        let clicked_field = contains(inner, mouse.column, mouse.row)
            .then(|| usize::from(mouse.row.saturating_sub(inner.y)));
        match self.modal.as_mut() {
            Some(Modal::Profile(form)) => {
                let content = Rect::new(
                    inner.x,
                    inner.y,
                    inner.width,
                    inner.height.saturating_sub(4),
                );
                let (_, offset) = form_viewport(content, form.selected);
                if let Some(index) = clicked_field
                    .filter(|_| contains(content, mouse.column, mouse.row))
                    .map(|index| index + offset)
                    && index < form.fields.len()
                {
                    form.selected = index;
                    if !form.fields[index].choices.is_empty() {
                        cycle_choice(&mut form.fields[index], true);
                    }
                }
            }
            Some(Modal::Model(form)) => {
                let content_area = Rect::new(
                    inner.x,
                    inner.y,
                    inner.width,
                    inner.height.saturating_sub(2),
                );
                let (form_area, api_area) = model_form_areas(content_area, form.focus_api_search);
                if contains(form_area, mouse.column, mouse.row) {
                    form.focus_api_search = false;
                    let form_inner = panel_inner(form_area);
                    let clicked_field = contains(form_inner, mouse.column, mouse.row)
                        .then(|| usize::from(mouse.row.saturating_sub(form_inner.y)));
                    let (_, offset) = form_viewport(form_inner, form.selected);
                    if let Some(index) = clicked_field.map(|index| index + offset)
                        && index < form.fields.len()
                    {
                        form.selected = index;
                        if form.fields[index].toggle {
                            toggle_form_field(&mut form.fields[index]);
                        }
                    }
                } else if contains(api_area, mouse.column, mouse.row) {
                    let api_inner = panel_inner(api_area);
                    if mouse.row == api_inner.y {
                        form.focus_api_search = true;
                    } else if mouse.row >= api_inner.y.saturating_add(2) {
                        let list_row =
                            usize::from(mouse.row.saturating_sub(api_inner.y.saturating_add(2)));
                        let item_idx = form.api_scroll + list_row;
                        form.api_selected = item_idx;
                        form.pick_api_model(item_idx);
                        form.focus_api_search = false;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn handle_modal(&mut self, key: KeyEvent) -> Result<()> {
        let saved = self.modal.clone();
        if let Err(error) = self.handle_modal_inner(key) {
            self.modal = saved;
            self.set_error(format!("Could not save changes: {error:#}"));
        }
        Ok(())
    }

    pub(super) fn handle_modal_inner(&mut self, key: KeyEvent) -> Result<()> {
        let Some(mut modal) = self.modal.take() else {
            return Ok(());
        };
        match &mut modal {
            Modal::Import(candidate) => match key.code {
                KeyCode::Char('i') | KeyCode::Enter => {
                    let profile = candidate.profile.clone();
                    self.config = config::try_update(&self.paths.config, |latest| {
                        let id = unique_profile_id("imported", &latest.profiles);
                        latest.profiles.insert(id, profile);
                        Ok(())
                    })?;
                    self.profile_idx = 0;
                    self.model_idx = self.default_model_index();
                    self.status =
                        "Imported existing Claude settings; the source file was not changed".into();
                    self.status_error = false;
                    return Ok(());
                }
                KeyCode::Char('s') | KeyCode::Esc => return Ok(()),
                _ => {}
            },
            Modal::Help(help) => match key.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter => {
                    return Ok(());
                }
                KeyCode::Tab | KeyCode::Right => help.move_section(true),
                KeyCode::BackTab | KeyCode::Left => help.move_section(false),
                KeyCode::Down | KeyCode::PageDown => help.scroll(true),
                KeyCode::Up | KeyCode::PageUp => help.scroll(false),
                KeyCode::Char('1') => help.select(0),
                KeyCode::Char('2') => help.select(1),
                KeyCode::Char('3') => help.select(2),
                KeyCode::Char('4') => help.select(3),
                _ => {}
            },
            Modal::DeleteProfile => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    if let Some(id) = self.selected_profile_id() {
                        let original = self.config.profiles.get(&id).cloned();
                        self.config = config::try_update(&self.paths.config, |latest| {
                            if latest.profiles.get(&id) != original.as_ref() {
                                anyhow::bail!(
                                    "provider changed in another CCSW instance; reopen before deleting"
                                );
                            }
                            latest.profiles.remove(&id);
                            Ok(())
                        })?;
                        let cache_result = discovery::update_cache(&self.paths.cache, |cache| {
                            cache.profiles.remove(&id);
                        });
                        self.cache.profiles.remove(&id);
                        self.profile_idx = self
                            .profile_idx
                            .min(self.config.profiles.len().saturating_sub(1));
                        self.model_idx = 0;
                        if self.view_mode == ViewMode::Provider {
                            self.return_home();
                        }
                        self.status_error = false;
                        self.status = format!("Deleted profile {id}");
                        if let Err(error) = cache_result {
                            self.set_error(format!(
                                "Provider deleted, but cache cleanup failed: {error:#}"
                            ));
                        }
                    }
                    return Ok(());
                }
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                _ => {}
            },
            Modal::DeleteModel => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    if let (Some(profile_id), Some(model)) =
                        (self.selected_profile_id(), self.selected_model())
                    {
                        let model_base = canonical_model_id(&model.id);
                        let is_manual = self.config.profiles[&profile_id]
                            .models
                            .iter()
                            .any(|entry| canonical_model_id(&entry.id) == model_base);
                        if !is_manual {
                            self.set_error(
                                "Gateway models cannot be deleted · use Space to disable them",
                            );
                        } else {
                            let model_id = model.id.clone();
                            let deleting_default = canonical_model_id(
                                &self.config.profiles[&profile_id].default_model,
                            ) == model_base;
                            let replacement = self.provider_editor.as_ref().and_then(|editor| {
                                editor
                                    .catalog
                                    .iter()
                                    .find(|entry| {
                                        entry.id != model_base && editor.is_enabled(&entry.id)
                                    })
                                    .or_else(|| {
                                        editor.catalog.iter().find(|entry| entry.id != model_base)
                                    })
                                    .map(|entry| editor.effective_id(&entry.id))
                            });
                            if deleting_default && replacement.is_none() {
                                self.set_error(
                                    "The only model cannot be deleted · add another model first",
                                );
                                return Ok(());
                            }
                            let original = self.config.profiles[&profile_id].clone();
                            let mut edited = original.clone();
                            {
                                let profile = &mut edited;
                                profile
                                    .models
                                    .retain(|entry| canonical_model_id(&entry.id) != model_base);
                                profile
                                    .enabled_models
                                    .retain(|id| canonical_model_id(id) != model_base);
                                profile
                                    .disabled_models
                                    .retain(|id| canonical_model_id(id) != model_base);
                                for alias in [
                                    &mut profile.aliases.opus,
                                    &mut profile.aliases.sonnet,
                                    &mut profile.aliases.haiku,
                                    &mut profile.aliases.fable,
                                    &mut profile.subagent_model,
                                ] {
                                    if alias
                                        .as_ref()
                                        .is_some_and(|id| canonical_model_id(id) == model_base)
                                    {
                                        *alias = None;
                                    }
                                }
                                profile
                                    .fallback_models
                                    .retain(|id| canonical_model_id(id) != model_base);
                                if let Some(replacement) = &replacement
                                    && deleting_default
                                {
                                    let replacement_base = canonical_model_id(replacement);
                                    profile
                                        .disabled_models
                                        .retain(|id| canonical_model_id(id) != replacement_base);
                                    profile.default_model = replacement.clone();
                                }
                            }
                            self.config = config::update_profile(
                                &self.paths.config,
                                &profile_id,
                                &original,
                                &edited,
                            )?;
                            self.model_idx =
                                self.model_idx.min(self.models().len().saturating_sub(1));
                            self.status_error = false;
                            self.status = format!("Deleted custom model {model_id}");
                            self.init_provider_editor();
                        }
                    }
                    return Ok(());
                }
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                _ => {}
            },
            Modal::Proxy(manager) => {
                if let Some(port) = &mut manager.port_field {
                    if !self.background.proxy_running {
                        match handle_form_key(std::slice::from_mut(port), &mut 0, key) {
                            FormOutcome::Close => {
                                manager.port_field = None;
                                manager.error = false;
                                manager.message = "Port edit cancelled".into();
                            }
                            FormOutcome::Submit => {
                                self.modal = Some(modal);
                                self.start_proxy_action(ProxyControl::Port);
                                return Ok(());
                            }
                            FormOutcome::Stay => {}
                        }
                    }
                    self.modal = Some(modal);
                    return Ok(());
                }
                let control = match key.code {
                    KeyCode::Esc | KeyCode::Char('P') | KeyCode::Char('q') => return Ok(()),
                    KeyCode::Tab | KeyCode::Right | KeyCode::Down => {
                        manager.move_selection(true);
                        None
                    }
                    KeyCode::BackTab | KeyCode::Left | KeyCode::Up => {
                        manager.move_selection(false);
                        None
                    }
                    KeyCode::Enter | KeyCode::Char(' ') => Some(manager.selected_control()),
                    KeyCode::Char('s') => Some(ProxyControl::Start),
                    KeyCode::Char('x') => Some(ProxyControl::Stop),
                    KeyCode::Char('r') => Some(ProxyControl::Refresh),
                    KeyCode::Char('i') => Some(ProxyControl::EnableAtLogin),
                    KeyCode::Char('u') => Some(ProxyControl::DisableAtLogin),
                    KeyCode::Char('e') => Some(ProxyControl::Port),
                    _ => None,
                };
                if let Some(control) = control {
                    manager.selected = proxy_control_index(control);
                    if control == ProxyControl::Port {
                        if !self.background.proxy_running {
                            manager.edit_port();
                        }
                        self.modal = Some(modal);
                        return Ok(());
                    }
                    if control == ProxyControl::Close {
                        return Ok(());
                    }
                    self.modal = Some(modal);
                    self.start_proxy_action(control);
                    return Ok(());
                }
            }
            Modal::Profile(form) => {
                let outcome = handle_form_key(&mut form.fields, &mut form.selected, key);
                if outcome == FormOutcome::Close {
                    return Ok(());
                }
                if outcome == FormOutcome::Submit
                    || (key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('s'))
                {
                    match form.to_profile() {
                        Ok((id, profile)) => {
                            let original = form.original_id.clone();
                            let original_profile = form.original_profile.clone();
                            let update = config::try_update(&self.paths.config, |latest| {
                                let profile = if let (Some(original_id), Some(original_profile)) =
                                    (&original, &original_profile)
                                {
                                    let current = latest
                                        .profiles
                                        .get(original_id)
                                        .context("provider was removed in another CCSW instance")?;
                                    config::merge_profile(original_profile, &profile, current)?
                                } else {
                                    profile
                                };
                                if latest.profiles.contains_key(&id)
                                    && original.as_deref() != Some(id.as_str())
                                {
                                    anyhow::bail!("profile id '{id}' already exists");
                                }
                                if let Some(original) = &original
                                    && original != &id
                                {
                                    latest.profiles.remove(original);
                                }
                                latest.profiles.insert(id.clone(), profile);
                                Ok(())
                            });
                            self.config = match update {
                                Ok(config) => config,
                                Err(error) => {
                                    self.set_error(format!("Cannot save profile: {error:#}"));
                                    self.modal = Some(modal);
                                    return Ok(());
                                }
                            };
                            let current = &self.config.profiles[&id];
                            let connection_changed =
                                form.original_profile.as_ref().is_some_and(|old| {
                                    old.base_url != current.base_url
                                        || old.api_format != current.api_format
                                        || old.credential != current.credential
                                });
                            let mut cache_error = None;
                            if let Some(original) = &form.original_id
                                && (original != &id || connection_changed)
                            {
                                let result = discovery::update_cache(&self.paths.cache, |cache| {
                                    if let Some(cached) = cache.profiles.remove(original)
                                        && !connection_changed
                                    {
                                        cache.profiles.insert(id.clone(), cached);
                                    }
                                });
                                match result {
                                    Ok(cache) => self.cache = cache,
                                    Err(error) => {
                                        self.cache.profiles.remove(original);
                                        cache_error = Some(error);
                                    }
                                }
                            }
                            self.profile_idx = self
                                .profile_ids()
                                .iter()
                                .position(|candidate| candidate == &id)
                                .unwrap_or(0);
                            self.model_idx = self.default_model_index();
                            self.status_error = false;
                            self.status = format!("Saved profile {id}");
                            if let Some(error) = cache_error {
                                self.set_error(format!(
                                    "Provider saved, but cache update failed: {error:#}"
                                ));
                            }
                            self.init_provider_editor();
                            return Ok(());
                        }
                        Err(error) => self.set_error(format!("Cannot save profile: {error:#}")),
                    }
                }
            }
            Modal::Model(form) => {
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && (key.code == KeyCode::Char('f') || key.code == KeyCode::Char('r'))
                {
                    self.modal = Some(modal);
                    self.fetch_api_models_for_form();
                    return Ok(());
                }

                let modal_rect = centered_rect(
                    86.min(self.screen.width.saturating_sub(2)),
                    20.min(self.screen.height.saturating_sub(2)),
                    self.screen,
                );
                let inner = panel_inner(modal_rect);
                let content = Rect::new(
                    inner.x,
                    inner.y,
                    inner.width,
                    inner.height.saturating_sub(2),
                );
                let (_, api_area) = model_form_areas(content, form.focus_api_search);
                let visible = usize::from(panel_inner(api_area).height.saturating_sub(3)).max(1);
                let outcome = form.handle_key(key, visible);
                if outcome == FormOutcome::Close {
                    return Ok(());
                }
                if outcome == FormOutcome::Submit
                    || (key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('s'))
                {
                    let base_id = canonical_model_id(form.fields[0].value.trim());
                    if base_id.trim().is_empty() {
                        self.set_error("Model id cannot be empty");
                    } else if let Err(error) = form.validate_tokens() {
                        self.set_error(error.to_string());
                    } else if let Some(profile_id) = self.selected_profile_id() {
                        let model = form.to_model();
                        let saved_model = model.clone();
                        let enable_now = form.enable_now();
                        let original = form
                            .original_profile
                            .as_deref()
                            .cloned()
                            .unwrap_or_else(|| self.config.profiles[&profile_id].clone());
                        let mut edited = original.clone();
                        {
                            let profile = &mut edited;
                            let saved_base = canonical_model_id(&saved_model.id);
                            let old_base = form.original_model_id.as_deref().unwrap_or(&saved_base);
                            profile.models.retain(|entry| {
                                canonical_model_id(&entry.id) != saved_base
                                    && canonical_model_id(&entry.id) != old_base
                            });
                            profile.models.push(saved_model.clone());
                            for reference in std::iter::once(&mut profile.default_model)
                                .chain(
                                    [
                                        &mut profile.aliases.opus,
                                        &mut profile.aliases.sonnet,
                                        &mut profile.aliases.haiku,
                                        &mut profile.aliases.fable,
                                        &mut profile.subagent_model,
                                    ]
                                    .into_iter()
                                    .flatten(),
                                )
                                .chain(profile.fallback_models.iter_mut())
                            {
                                if canonical_model_id(reference) == saved_base
                                    || canonical_model_id(reference) == old_base
                                {
                                    *reference = saved_model.id.clone();
                                }
                            }
                            profile.enabled_models.retain(|id| {
                                canonical_model_id(id) != saved_base
                                    && canonical_model_id(id) != old_base
                            });
                            profile.disabled_models.retain(|id| {
                                canonical_model_id(id) != saved_base
                                    && canonical_model_id(id) != old_base
                            });
                            if enable_now {
                                if !profile.required_model_ids().contains(&saved_model.id) {
                                    profile.enabled_models.push(saved_model.id.clone());
                                }
                            } else {
                                profile.disabled_models.push(saved_model.id.clone());
                            }
                        }
                        self.config = config::update_profile(
                            &self.paths.config,
                            &profile_id,
                            &original,
                            &edited,
                        )?;
                        self.select_model_id(&model.id);
                        self.status_error = false;
                        self.status = format!(
                            "Saved model {} · {}",
                            model.id,
                            if enable_now { "enabled" } else { "disabled" }
                        );
                        self.init_provider_editor();
                        if let Some(editor) = &mut self.provider_editor
                            && let Some(index) = editor
                                .catalog
                                .iter()
                                .position(|entry| canonical_model_id(&entry.id) == base_id)
                        {
                            editor.selected = index;
                        }
                        return Ok(());
                    }
                }
            }
        }
        self.modal = Some(modal);
        Ok(())
    }
}
