use super::*;

impl ProxyManager {
    pub(super) const CONTROLS: [ProxyControl; 7] = [
        ProxyControl::Start,
        ProxyControl::Stop,
        ProxyControl::Refresh,
        ProxyControl::Port,
        ProxyControl::EnableAtLogin,
        ProxyControl::DisableAtLogin,
        ProxyControl::Close,
    ];

    pub(super) fn empty() -> Self {
        Self {
            instance: uuid::Uuid::new_v4(),
            port_field: None,
            port_changed: false,
            runtime: None,
            service: None,
            selected: 0,
            message: "Sync all starts the proxy; this page may be closed afterward.".into(),
            error: false,
        }
    }

    pub(super) fn selected_control(&self) -> ProxyControl {
        Self::CONTROLS[self.selected.min(Self::CONTROLS.len() - 1)]
    }

    pub(super) fn move_selection(&mut self, forward: bool) {
        if forward {
            self.selected = (self.selected + 1) % Self::CONTROLS.len();
        } else {
            self.selected = self
                .selected
                .checked_sub(1)
                .unwrap_or(Self::CONTROLS.len() - 1);
        }
    }

    pub(super) fn edit_port(&mut self) {
        let port = self
            .runtime
            .as_ref()
            .and_then(|status| status.listen.parse::<std::net::SocketAddr>().ok())
            .map(|address| address.port())
            .unwrap_or(17321);
        self.port_field = Some(field("Port", &port.to_string()));
        self.message =
            "Stop this proxy first. Save the port, then sync with p on the main screen.".into();
        self.error = false;
    }

    fn save_port(&mut self, paths: &AppPaths) -> Result<String> {
        let value = self
            .port_field
            .as_ref()
            .context("open the port editor first")?
            .value
            .trim();
        let port: u16 = value
            .parse()
            .context("port must be a number between 1 and 65535")?;
        let status = proxy::set_port(paths, port)?;
        self.port_field = None;
        self.port_changed = true;
        Ok(format!(
            "Saved {}. Close this panel and press p to start and sync Claude.",
            status.listen
        ))
    }

    pub(super) fn activate(&mut self, paths: &AppPaths, control: ProxyControl) -> bool {
        if control == ProxyControl::Close {
            return true;
        }
        self.port_changed = false;
        let result = match control {
            ProxyControl::Port => self.save_port(paths),
            ProxyControl::Start => proxy::start(paths, None)
                .map(|status| format!("Proxy started at {}", status.listen)),
            ProxyControl::Stop => proxy::stop(paths).map(|()| "Proxy stopped".into()),
            ProxyControl::Refresh => Ok("Status refreshed".into()),
            ProxyControl::EnableAtLogin => proxy::install(paths)
                .map(|path| format!("Start at login enabled · {}", path.display())),
            ProxyControl::DisableAtLogin => proxy::uninstall().map(|path| match path {
                Some(path) => format!("Start at login disabled · removed {}", path.display()),
                None => "Start at login is already disabled".into(),
            }),
            ProxyControl::Close => unreachable!(),
        };
        match result {
            Ok(message) => self.refresh_state(paths, Some((message, false))),
            Err(error) => self.refresh_state(paths, Some((format!("{error:#}"), true))),
        }
        false
    }

    pub(super) fn refresh_state(&mut self, paths: &AppPaths, message: Option<(String, bool)>) {
        let runtime = proxy::status(paths);
        let service = proxy::service_status();
        self.runtime = runtime.as_ref().ok().cloned();
        self.service = service.as_ref().ok().cloned();
        if let Some((message, error)) = message {
            self.message = message;
            self.error = error;
        } else if let Err(error) = runtime {
            self.message = format!("Could not read proxy status: {error:#}");
            self.error = true;
        } else if let Err(error) = service {
            self.message = format!("Could not read start-at-login status: {error:#}");
            self.error = true;
        }
    }
}

impl ProfileForm {
    pub(super) fn new() -> Self {
        Self::from_values(
            None,
            vec![],
            vec![],
            vec![],
            "",
            "",
            "https://",
            "anthropic",
            "bearer",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
        )
    }

    pub(super) fn edit(id: String, profile: &Profile) -> Self {
        let (kind, credential) = match &profile.credential {
            Credential::Bearer { value } => ("bearer", value.as_str()),
            Credential::XApiKey { value } => ("x-api-key", value.as_str()),
            Credential::ApiKey { value } => ("api-key", value.as_str()),
            Credential::None => ("none", ""),
        };
        let format = match profile.api_format {
            ApiFormat::Anthropic => "anthropic",
            ApiFormat::OpenaiChat => "openai-chat",
            ApiFormat::OpenaiResponses => "openai-responses",
        };
        let mut form = Self::from_values(
            Some(id.clone()),
            profile.models.clone(),
            profile.enabled_models.clone(),
            profile.disabled_models.clone(),
            &id,
            &profile.name,
            &profile.base_url,
            format,
            kind,
            credential,
            &profile.default_model,
            profile.aliases.opus.as_deref().unwrap_or(""),
            profile.aliases.sonnet.as_deref().unwrap_or(""),
            profile.aliases.haiku.as_deref().unwrap_or(""),
            profile.aliases.fable.as_deref().unwrap_or(""),
            profile.subagent_model.as_deref().unwrap_or(""),
            &profile.fallback_models.join(","),
        );
        form.provider_enabled = profile.enabled;
        form.original_profile = Some(profile.clone());
        form
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn from_values(
        original_id: Option<String>,
        models: Vec<ModelEntry>,
        enabled_models: Vec<String>,
        disabled_models: Vec<String>,
        id: &str,
        name: &str,
        base_url: &str,
        api_format: &str,
        kind: &str,
        credential: &str,
        default: &str,
        opus: &str,
        sonnet: &str,
        haiku: &str,
        fable: &str,
        subagent: &str,
        fallback: &str,
    ) -> Self {
        let fields = vec![
            field("ID", id),
            field("Name", name),
            choice_field(
                "API format",
                api_format,
                &["anthropic", "openai-chat", "openai-responses"],
            ),
            field("Base URL", base_url),
            choice_field(
                "Auth kind",
                kind,
                &["bearer", "x-api-key", "api-key", "none"],
            ),
            secret_field("Credential", credential),
            field("Default model", default),
            field("Opus", opus),
            field("Sonnet", sonnet),
            field("Haiku", haiku),
            field("Fable", fable),
            field("Subagent", subagent),
            field("Fallbacks (comma)", fallback),
        ];
        Self {
            original_id,
            original_profile: None,
            provider_enabled: true,
            models,
            enabled_models,
            disabled_models,
            fields,
            selected: 0,
        }
    }

    pub(super) fn to_profile(&self) -> Result<(String, Profile)> {
        let value = |index: usize| self.fields[index].value.trim().to_owned();
        let id = value(0);
        config::validate_profile_id(&id)?;
        let api_format = match value(2).as_str() {
            "anthropic" => ApiFormat::Anthropic,
            "openai-chat" => ApiFormat::OpenaiChat,
            "openai-responses" => ApiFormat::OpenaiResponses,
            other => anyhow::bail!("unknown API format {other}"),
        };
        let credential = match value(4).as_str() {
            "bearer" => Credential::Bearer { value: value(5) },
            "x-api-key" => Credential::XApiKey { value: value(5) },
            "api-key" => Credential::ApiKey { value: value(5) },
            "none" | "" => Credential::None,
            other => anyhow::bail!("unknown auth kind {other}"),
        };
        let optional = |index: usize| {
            let value = value(index);
            (!value.is_empty()).then_some(value)
        };
        let profile = Profile {
            name: value(1),
            enabled: self.provider_enabled,
            base_url: value(3),
            api_format,
            credential,
            default_model: value(6),
            aliases: RoleModels {
                opus: optional(7),
                sonnet: optional(8),
                haiku: optional(9),
                fable: optional(10),
            },
            subagent_model: optional(11),
            fallback_models: value(12)
                .split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .collect(),
            enabled_models: self.enabled_models.clone(),
            disabled_models: self.disabled_models.clone(),
            models: self.models.clone(),
        };
        profile.validate()?;
        Ok((id, profile))
    }
}

impl ModelForm {
    pub(super) fn handle_key(&mut self, key: KeyEvent, visible: usize) -> FormOutcome {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
            return FormOutcome::Submit;
        }
        if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
            if key.code == KeyCode::Tab {
                if self.focus_api_search {
                    self.focus_api_search = false;
                    self.selected = 0;
                } else if self.selected + 1 < self.fields.len() {
                    self.selected += 1;
                } else {
                    self.focus_api_search = true;
                }
            } else if self.focus_api_search {
                self.focus_api_search = false;
                self.selected = self.fields.len() - 1;
            } else if self.selected > 0 {
                self.selected -= 1;
            } else {
                self.focus_api_search = true;
            }
            return FormOutcome::Stay;
        }
        if !self.focus_api_search {
            return handle_form_key(&mut self.fields, &mut self.selected, key);
        }
        match key.code {
            KeyCode::Esc => {
                if self.api_query.is_empty() {
                    self.focus_api_search = false;
                } else {
                    self.api_query.clear();
                    self.api_query_cursor = 0;
                    self.api_scroll = 0;
                    self.api_selected = 0;
                }
            }
            KeyCode::Up => self.move_api_selection(false, visible),
            KeyCode::Down => self.move_api_selection(true, visible),
            KeyCode::PageUp => self.scroll_api_list(false, visible, visible),
            KeyCode::PageDown => self.scroll_api_list(true, visible, visible),
            KeyCode::Enter => {
                self.pick_api_model(self.api_selected);
                self.focus_api_search = false;
            }
            _ => {
                let mut query = field("Search", &self.api_query);
                query.cursor = self.api_query_cursor;
                handle_form_key(std::slice::from_mut(&mut query), &mut 0, key);
                if query.value != self.api_query {
                    self.api_scroll = 0;
                    self.api_selected = 0;
                }
                self.api_query = query.value;
                self.api_query_cursor = query.cursor;
            }
        }
        FormOutcome::Stay
    }

    #[cfg(test)]
    pub(super) fn new() -> Self {
        Self::with_api_models(vec![])
    }

    pub(super) fn with_api_models(api_models: Vec<ModelEntry>) -> Self {
        let api_models = config::deduplicate_model_entries(api_models);
        let count = api_models.len();
        let api_status = if count > 0 {
            format!("{count} cached API models · search or click to fill fields")
        } else {
            "Fetch API to load models from this provider".into()
        };
        Self {
            fields: vec![
                field("Model ID", ""),
                field("Label", ""),
                field("Description", ""),
                toggle_field("1M context", false),
                toggle_field("Enable now", true),
            ],
            selected: 0,
            api_models,
            instance: uuid::Uuid::new_v4(),
            original_profile: None,
            api_query: String::new(),
            api_query_cursor: 0,
            api_scroll: 0,
            api_selected: 0,
            focus_api_search: false,
            api_status,
        }
    }

    pub(super) fn filtered_api_models(&self) -> Vec<&ModelEntry> {
        let q = self.api_query.trim().to_lowercase();
        self.api_models
            .iter()
            .filter(|m| {
                q.is_empty()
                    || m.id.to_lowercase().contains(&q)
                    || m.label
                        .as_deref()
                        .is_some_and(|l| l.to_lowercase().contains(&q))
                    || m.description
                        .as_deref()
                        .is_some_and(|d| d.to_lowercase().contains(&q))
            })
            .collect()
    }

    pub(super) fn scroll_api_list(&mut self, down: bool, delta: usize, visible_height: usize) {
        let total = self.filtered_api_models().len();
        if total == 0 {
            self.api_scroll = 0;
            self.api_selected = 0;
            return;
        }
        let max_scroll = total.saturating_sub(visible_height.max(1));
        if down {
            self.api_scroll = (self.api_scroll + delta).min(max_scroll);
        } else {
            self.api_scroll = self.api_scroll.saturating_sub(delta);
        }
        if self.api_selected < self.api_scroll {
            self.api_selected = self.api_scroll;
        } else if self.api_selected >= self.api_scroll + visible_height.max(1) {
            self.api_selected = self.api_scroll + visible_height.max(1) - 1;
        }
    }

    pub(super) fn move_api_selection(&mut self, down: bool, visible_height: usize) {
        let total = self.filtered_api_models().len();
        if total == 0 {
            self.api_scroll = 0;
            self.api_selected = 0;
            return;
        }
        if down {
            if self.api_selected + 1 < total {
                self.api_selected += 1;
            }
        } else {
            self.api_selected = self.api_selected.saturating_sub(1);
        }
        let h = visible_height.max(1);
        if self.api_selected >= self.api_scroll + h {
            self.api_scroll = self.api_selected + 1 - h;
        }
        if self.api_selected < self.api_scroll {
            self.api_scroll = self.api_selected;
        }
    }

    pub(super) fn pick_api_model(&mut self, index: usize) {
        let model = self.filtered_api_models().get(index).copied().cloned();
        if let Some(model) = model {
            let base_id = canonical_model_id(&model.id);
            self.fields[0].value = base_id.clone();
            self.fields[0].cursor = self.fields[0].char_count();
            if let Some(label) = &model.label {
                self.fields[1].value = label.clone();
                self.fields[1].cursor = self.fields[1].char_count();
            } else {
                self.fields[1].value = base_id.clone();
                self.fields[1].cursor = self.fields[1].char_count();
            }
            self.fields[2].value = model.description.clone().unwrap_or_default();
            self.fields[2].cursor = self.fields[2].char_count();
            self.fields[3].value = has_1m_suffix(&model.id).to_string();
            self.api_status = format!("Selected API model: {base_id}");
        }
    }

    pub(super) fn to_model(&self) -> ModelEntry {
        let optional = |index: usize| {
            let value = self.fields[index].value.trim();
            (!value.is_empty()).then(|| value.to_owned())
        };
        let base_id = canonical_model_id(self.fields[0].value.trim());
        let one_m = self.fields[3].value == "true";
        let label = optional(1).map(|label| {
            if one_m && !label.to_ascii_lowercase().contains("1m") {
                format!("{label} · 1M")
            } else {
                label
            }
        });
        ModelEntry {
            id: if one_m && !base_id.is_empty() {
                format!("{base_id}[1m]")
            } else {
                base_id
            },
            label,
            description: optional(2),
        }
    }

    pub(super) fn enable_now(&self) -> bool {
        self.fields[4].value == "true"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FormOutcome {
    Stay,
    Close,
    Submit,
}

impl FormField {
    pub(super) fn char_count(&self) -> usize {
        self.value.chars().count()
    }

    pub(super) fn clamp_cursor(&mut self) {
        let count = self.char_count();
        if self.cursor > count {
            self.cursor = count;
        }
    }

    pub(super) fn insert_char(&mut self, ch: char) {
        self.clamp_cursor();
        let byte_offset = self
            .value
            .char_indices()
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.value.len());
        self.value.insert(byte_offset, ch);
        self.cursor += 1;
    }

    pub(super) fn delete_backward(&mut self) {
        self.clamp_cursor();
        if self.cursor > 0 {
            self.cursor -= 1;
            let byte_offset = self
                .value
                .char_indices()
                .nth(self.cursor)
                .map(|(i, _)| i)
                .unwrap_or(self.value.len());
            self.value.remove(byte_offset);
        }
    }

    pub(super) fn delete_forward(&mut self) {
        self.clamp_cursor();
        if self.cursor < self.char_count() {
            let byte_offset = self
                .value
                .char_indices()
                .nth(self.cursor)
                .map(|(i, _)| i)
                .unwrap_or(self.value.len());
            self.value.remove(byte_offset);
        }
    }

    pub(super) fn clear_text(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }
}

pub(super) fn handle_form_key(
    fields: &mut [FormField],
    selected: &mut usize,
    key: KeyEvent,
) -> FormOutcome {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
        return FormOutcome::Submit;
    }
    match key.code {
        KeyCode::Esc => return FormOutcome::Close,
        KeyCode::Tab | KeyCode::Down => {
            *selected = (*selected + 1) % fields.len();
            fields[*selected].clamp_cursor();
        }
        KeyCode::BackTab | KeyCode::Up => {
            *selected = selected.checked_sub(1).unwrap_or(fields.len() - 1);
            fields[*selected].clamp_cursor();
        }
        KeyCode::Enter => {
            if *selected + 1 < fields.len() {
                *selected += 1;
                fields[*selected].clamp_cursor();
            } else {
                return FormOutcome::Submit;
            }
        }
        KeyCode::Left | KeyCode::Right if fields[*selected].toggle => {
            toggle_form_field(&mut fields[*selected])
        }
        KeyCode::Char(' ') if fields[*selected].toggle => {
            toggle_form_field(&mut fields[*selected]);
        }
        KeyCode::Char(' ') if !fields[*selected].choices.is_empty() => {
            cycle_choice(&mut fields[*selected], true);
        }
        KeyCode::Right if !fields[*selected].choices.is_empty() => {
            cycle_choice(&mut fields[*selected], true);
        }
        KeyCode::Right if !fields[*selected].toggle => {
            let max = fields[*selected].char_count();
            fields[*selected].cursor = (fields[*selected].cursor + 1).min(max);
        }
        KeyCode::Left if !fields[*selected].choices.is_empty() => {
            cycle_choice(&mut fields[*selected], false);
        }
        KeyCode::Left if !fields[*selected].toggle => {
            fields[*selected].cursor = fields[*selected].cursor.saturating_sub(1);
        }
        KeyCode::Home if !fields[*selected].toggle && fields[*selected].choices.is_empty() => {
            fields[*selected].cursor = 0;
        }
        KeyCode::Char('a')
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && !fields[*selected].toggle
                && fields[*selected].choices.is_empty() =>
        {
            fields[*selected].cursor = 0;
        }
        KeyCode::End if !fields[*selected].toggle && fields[*selected].choices.is_empty() => {
            fields[*selected].cursor = fields[*selected].char_count();
        }
        KeyCode::Char('e')
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && !fields[*selected].toggle
                && fields[*selected].choices.is_empty() =>
        {
            fields[*selected].cursor = fields[*selected].char_count();
        }
        KeyCode::Char('u')
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && !fields[*selected].toggle
                && fields[*selected].choices.is_empty() =>
        {
            fields[*selected].clear_text();
        }
        KeyCode::Backspace if !fields[*selected].toggle && fields[*selected].choices.is_empty() => {
            fields[*selected].delete_backward();
        }
        KeyCode::Delete if !fields[*selected].toggle && fields[*selected].choices.is_empty() => {
            fields[*selected].delete_forward();
        }
        KeyCode::Char(ch)
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !fields[*selected].toggle
                && fields[*selected].choices.is_empty() =>
        {
            fields[*selected].insert_char(ch);
        }
        _ => {}
    }
    FormOutcome::Stay
}

pub(super) fn field(label: &'static str, value: &str) -> FormField {
    let count = value.chars().count();
    FormField {
        label,
        value: value.into(),
        cursor: count,
        secret: false,
        toggle: false,
        choices: &[],
    }
}

pub(super) fn secret_field(label: &'static str, value: &str) -> FormField {
    let count = value.chars().count();
    FormField {
        label,
        value: value.into(),
        cursor: count,
        secret: true,
        toggle: false,
        choices: &[],
    }
}

pub(super) fn toggle_field(label: &'static str, enabled: bool) -> FormField {
    FormField {
        label,
        value: enabled.to_string(),
        cursor: 0,
        secret: false,
        toggle: true,
        choices: &[],
    }
}

pub(super) fn choice_field(
    label: &'static str,
    value: &str,
    choices: &'static [&'static str],
) -> FormField {
    FormField {
        label,
        value: value.into(),
        cursor: 0,
        secret: false,
        toggle: false,
        choices,
    }
}

pub(super) fn cycle_choice(field: &mut FormField, forward: bool) {
    let current = field
        .choices
        .iter()
        .position(|choice| *choice == field.value)
        .unwrap_or(0);
    let index = if forward {
        (current + 1) % field.choices.len()
    } else {
        current.checked_sub(1).unwrap_or(field.choices.len() - 1)
    };
    field.value = field.choices[index].into();
    field.cursor = 0;
}

pub(super) fn toggle_form_field(field: &mut FormField) {
    field.value = (field.value != "true").to_string();
    field.cursor = 0;
}

pub(super) fn form_viewport(area: Rect, selected: usize) -> (Rect, usize) {
    let height = usize::from(area.height);
    (
        area,
        selected.saturating_add(1).saturating_sub(height.max(1)),
    )
}

pub(super) fn input_window(
    value: &str,
    cursor: Option<usize>,
    width: usize,
    secret: bool,
) -> String {
    if width == 0 {
        return String::new();
    }
    let chars: Vec<char> = value
        .chars()
        .map(|ch| if secret { '•' } else { ch })
        .collect();
    let cursor = cursor.map(|cursor| cursor.min(chars.len()));
    let mut start = 0;
    if let Some(cursor) = cursor {
        let mut used = 1;
        start = cursor;
        while start > 0 {
            let next = UnicodeWidthChar::width(chars[start - 1]).unwrap_or(0);
            if used + next > width {
                break;
            }
            used += next;
            start -= 1;
        }
    }
    let mut shown = String::new();
    let mut used = 0;
    for index in start..=chars.len() {
        if cursor == Some(index) && used < width {
            shown.push('▌');
            used += 1;
        }
        let Some(ch) = chars.get(index) else {
            break;
        };
        let next = UnicodeWidthChar::width(*ch).unwrap_or(0);
        if used + next > width {
            break;
        }
        shown.push(*ch);
        used += next;
    }
    shown
}

pub(super) fn draw_fields(
    frame: &mut ratatui::Frame,
    area: Rect,
    fields: &[FormField],
    selected: usize,
    active: bool,
) {
    let (area, offset) = form_viewport(area, selected);
    let label_width = usize::from((area.width / 3).min(17));
    let input_width = usize::from(area.width).saturating_sub(label_width + 2);
    for (row, (index, field)) in fields
        .iter()
        .enumerate()
        .skip(offset)
        .take(usize::from(area.height))
        .enumerate()
    {
        let current = active && index == selected;
        let value = if !field.choices.is_empty() {
            format!("‹ {} ›", field.value)
        } else if field.toggle {
            if field.value == "true" {
                "● On".into()
            } else {
                "○ Off".into()
            }
        } else {
            field.value.clone()
        };
        let cursor = (current && field.choices.is_empty() && !field.toggle).then_some(field.cursor);
        let shown = input_window(&value, cursor, input_width, field.secret);
        let label = input_window(field.label, None, label_width, false);
        let line = Line::from(vec![
            Span::styled(
                format!("{label:>label_width$}  "),
                Style::default().fg(if current { ROUTE } else { MUTED }),
            ),
            Span::styled(
                shown,
                Style::default().add_modifier(if current {
                    Modifier::REVERSED
                } else {
                    Modifier::empty()
                }),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(line),
            Rect::new(area.x, area.y + row as u16, area.width, 1),
        );
    }
}

pub(super) fn draw_form(
    frame: &mut ratatui::Frame,
    area: Rect,
    title: &str,
    fields: &[FormField],
    selected: usize,
) {
    frame.render_widget(Clear, area);
    frame.render_widget(panel(title, true), area);
    let inner = panel_inner(area);
    let content = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(4),
    );
    draw_fields(frame, content, fields, selected, true);
}

pub(super) fn model_form_areas(area: Rect, focus_api: bool) -> (Rect, Rect) {
    if area.width < 60 || area.height < 12 {
        let hidden = Rect::new(area.x, area.y, 0, 0);
        return if focus_api {
            (hidden, area)
        } else {
            (area, hidden)
        };
    }
    if area.width >= 80 {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(38), Constraint::Min(36)])
            .split(area);
        (cols[0], cols[1])
    } else {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(8), Constraint::Min(6)])
            .split(area);
        (rows[0], rows[1])
    }
}

pub(super) fn draw_model_form(frame: &mut ratatui::Frame, area: Rect, form: &ModelForm) {
    frame.render_widget(Clear, area);
    frame.render_widget(
        panel(
            if area.width < 60 {
                " Add model · Tab: fields / API "
            } else {
                " Add model · enter an ID or choose an API model "
            },
            true,
        ),
        area,
    );
    let inner = panel_inner(area);
    let content_area = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(2),
    );
    let (form_area, api_area) = model_form_areas(content_area, form.focus_api_search);

    frame.render_widget(panel(" Model details ", !form.focus_api_search), form_area);
    draw_fields(
        frame,
        panel_inner(form_area),
        &form.fields,
        form.selected,
        !form.focus_api_search,
    );

    let api_count = form.api_models.len();
    let filtered = form.filtered_api_models();
    let filtered_count = filtered.len();
    let api_title = if form.api_query.is_empty() {
        format!(" Available API models ({api_count}) ")
    } else {
        format!(" Available API models ({filtered_count}/{api_count}) ")
    };
    frame.render_widget(panel(&api_title, form.focus_api_search), api_area);
    let api_inner = panel_inner(api_area);
    if form.api_models.is_empty() {
        let msg = vec![
            Line::raw(""),
            Line::styled(" No cached API models", Style::default().fg(MUTED)),
            Line::raw(""),
            Line::styled(" Use Fetch API (Ctrl+F)", Style::default().fg(ROUTE)),
            Line::styled(
                " to load the provider’s model catalog.",
                Style::default().fg(ROUTE),
            ),
            Line::raw(""),
            Line::styled(
                format!(" Status: {}", form.api_status),
                Style::default().fg(WARNING),
            ),
        ];
        frame.render_widget(Paragraph::new(msg).wrap(Wrap { trim: false }), api_inner);
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .split(api_inner);

        let search_text = if form.api_query.is_empty() && !form.focus_api_search {
            "Click or Tab to search…".into()
        } else {
            input_window(
                &form.api_query,
                form.focus_api_search.then_some(form.api_query_cursor),
                usize::from(chunks[0].width.saturating_sub(9)),
                false,
            )
        };
        let search_line = Line::from(vec![
            Span::styled(
                " Search: ",
                Style::default().fg(if form.focus_api_search { ROUTE } else { MUTED }),
            ),
            Span::styled(
                search_text,
                Style::default()
                    .fg(if form.focus_api_search {
                        Color::White
                    } else {
                        MUTED
                    })
                    .add_modifier(if form.focus_api_search {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
        ]);
        frame.render_widget(Paragraph::new(search_line), chunks[0]);

        let div_text = "─".repeat(usize::from(chunks[1].width));
        frame.render_widget(
            Paragraph::new(div_text).style(Style::default().fg(Color::DarkGray)),
            chunks[1],
        );

        let list_area = chunks[2];
        let list_height = usize::from(list_area.height);

        if filtered.is_empty() {
            let empty_msg = vec![
                Line::raw(""),
                Line::styled(
                    format!(" No models match \"{}\"", form.api_query),
                    Style::default().fg(WARNING),
                ),
                Line::styled(" Press Esc to clear the search", Style::default().fg(MUTED)),
            ];
            frame.render_widget(Paragraph::new(empty_msg), list_area);
        } else {
            let visible_items: Vec<ListItem> = filtered
                .iter()
                .enumerate()
                .skip(form.api_scroll)
                .take(list_height)
                .map(|(idx, model)| {
                    let is_active = form.focus_api_search && idx == form.api_selected;
                    let mut spans = vec![
                        Span::styled(
                            if is_active { "▶ " } else { "● " },
                            Style::default().fg(if is_active { WARNING } else { CONNECTED }),
                        ),
                        Span::styled(
                            &model.id,
                            Style::default()
                                .fg(if is_active { WARNING } else { Color::White })
                                .add_modifier(Modifier::BOLD),
                        ),
                    ];
                    if let Some(label) = &model.label {
                        spans.push(Span::styled(
                            format!(" ({label})"),
                            Style::default().fg(MUTED),
                        ));
                    }
                    ListItem::new(Line::from(spans))
                })
                .collect();
            frame.render_widget(List::new(visible_items), list_area);

            if filtered_count > list_height {
                draw_scrollbar(
                    frame,
                    list_area,
                    filtered_count,
                    form.api_scroll,
                    list_height,
                );
            }
        }
    }
}

pub(super) fn draw_proxy_manager(frame: &mut ratatui::Frame, area: Rect, manager: &ProxyManager) {
    if let Some(port) = &manager.port_field {
        draw_form(
            frame,
            area,
            " Proxy listen port · Enter save / Esc cancel ",
            std::slice::from_ref(port),
            0,
        );
        let inner = panel_inner(area);
        let message = Rect::new(
            inner.x,
            inner.y.saturating_add(2),
            inner.width,
            inner.height.saturating_sub(4),
        );
        frame.render_widget(
            Paragraph::new(manager.message.as_str())
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(if manager.error { ERROR } else { MUTED })),
            message,
        );
        draw_modal_buttons(frame, area, &["Save port", "Cancel"]);
        return;
    }
    frame.render_widget(panel(" Proxy control · e edit port ", true), area);
    let inner = panel_inner(area);
    let running = manager
        .runtime
        .as_ref()
        .is_some_and(|status| status.running);
    let installed = manager
        .service
        .as_ref()
        .is_some_and(|status| status.installed);
    let runtime_color = if running { CONNECTED } else { WARNING };
    let rail = Line::from(vec![
        Span::styled("Claude Code", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled("  ──▶  ", Style::default().fg(MUTED)),
        Span::styled(
            format!("CCSW proxy {}", if running { '●' } else { '○' }),
            Style::default()
                .fg(runtime_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ──▶  ", Style::default().fg(MUTED)),
        Span::styled("Provider APIs", Style::default().fg(ROUTE)),
    ]);
    frame.render_widget(
        Paragraph::new(rail).alignment(Alignment::Center),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );

    let runtime = manager.runtime.as_ref();
    let service = manager.service.as_ref();
    let details = vec![
        Line::raw(""),
        detail(
            "Runtime",
            if running {
                "● Running in background"
            } else {
                "○ Stopped"
            },
        ),
        detail(
            "Listen",
            runtime
                .map(|status| status.listen.as_str())
                .unwrap_or("unknown"),
        ),
        detail(
            "PID / routes",
            &runtime
                .map(|status| {
                    format!(
                        "{} / {}",
                        status
                            .pid
                            .map(|pid| pid.to_string())
                            .unwrap_or_else(|| "—".into()),
                        status.routes
                    )
                })
                .unwrap_or_else(|| "unknown".into()),
        ),
        Line::raw(""),
        detail(
            "Start at login",
            if installed {
                "● Enabled"
            } else {
                "○ Disabled"
            },
        ),
        detail(
            "Service",
            service.map(|status| status.manager).unwrap_or("unknown"),
        ),
        detail(
            "Definition",
            &service
                .map(|status| status.path.display().to_string())
                .unwrap_or_else(|| "unknown".into()),
        ),
        Line::raw(""),
        Line::styled(
            "Sync all starts the proxy automatically. You can close the TUI afterward.",
            Style::default().fg(MUTED),
        ),
    ];
    frame.render_widget(
        Paragraph::new(details).wrap(Wrap { trim: false }),
        Rect::new(
            inner.x,
            inner.y.saturating_add(1),
            inner.width,
            inner.height.saturating_sub(6),
        ),
    );

    frame.render_widget(
        Paragraph::new(manager.message.as_str())
            .alignment(Alignment::Center)
            .style(Style::default().fg(if manager.error { ERROR } else { runtime_color })),
        Rect::new(
            inner.x,
            area.y.saturating_add(area.height).saturating_sub(6),
            inner.width,
            1,
        ),
    );
    draw_proxy_controls(frame, area, manager);
}

pub(super) fn proxy_controls(area: Rect) -> Vec<(ProxyControl, Rect)> {
    let bottom = area.y.saturating_add(area.height);
    let first = [
        (ProxyControl::Start, "Start"),
        (ProxyControl::Stop, "Stop"),
        (ProxyControl::Refresh, "Refresh"),
        (ProxyControl::Port, "Port (e)"),
    ];
    let second = [
        (ProxyControl::EnableAtLogin, "Enable at login"),
        (ProxyControl::DisableAtLogin, "Disable at login"),
        (ProxyControl::Close, "Close"),
    ];
    proxy_button_row_rects(area, bottom.saturating_sub(4), &first)
        .into_iter()
        .chain(proxy_button_row_rects(
            area,
            bottom.saturating_sub(2),
            &second,
        ))
        .collect()
}

pub(super) fn proxy_button_row_rects(
    area: Rect,
    y: u16,
    buttons: &[(ProxyControl, &str)],
) -> Vec<(ProxyControl, Rect)> {
    if buttons.is_empty() || area.height == 0 || y < area.y || y >= area.bottom() {
        return vec![];
    }
    let inner = panel_inner(area);
    let gap = 1_u16;
    let available = inner
        .width
        .saturating_sub(gap * buttons.len().saturating_sub(1) as u16);
    let natural: Vec<u16> = buttons
        .iter()
        .map(|(_, label)| label.width() as u16 + 4)
        .collect();
    let natural_total: u16 = natural.iter().sum();
    let widths = if natural_total <= available {
        natural
    } else {
        vec![available / buttons.len() as u16; buttons.len()]
    };
    let total: u16 = widths.iter().sum::<u16>() + gap * buttons.len().saturating_sub(1) as u16;
    let mut x = inner.x + inner.width.saturating_sub(total) / 2;
    buttons
        .iter()
        .zip(widths)
        .map(|((control, _), width)| {
            let rect = Rect::new(x, y, width, 1).intersection(area);
            x = x.saturating_add(width).saturating_add(gap);
            (*control, rect)
        })
        .collect()
}

pub(super) fn proxy_control_index(control: ProxyControl) -> usize {
    ProxyManager::CONTROLS
        .iter()
        .position(|candidate| *candidate == control)
        .unwrap_or(0)
}

pub(super) fn proxy_control_key(control: ProxyControl) -> KeyEvent {
    let code = match control {
        ProxyControl::Port => KeyCode::Char('e'),
        ProxyControl::Start => KeyCode::Char('s'),
        ProxyControl::Stop => KeyCode::Char('x'),
        ProxyControl::Refresh => KeyCode::Char('r'),
        ProxyControl::EnableAtLogin => KeyCode::Char('i'),
        ProxyControl::DisableAtLogin => KeyCode::Char('u'),
        ProxyControl::Close => KeyCode::Esc,
    };
    KeyEvent::new(code, KeyModifiers::NONE)
}

pub(super) fn draw_proxy_controls(frame: &mut ratatui::Frame, area: Rect, manager: &ProxyManager) {
    let running = manager
        .runtime
        .as_ref()
        .is_some_and(|status| status.running);
    let installed = manager
        .service
        .as_ref()
        .is_some_and(|status| status.installed);
    for (control, rect) in proxy_controls(area) {
        let label = match control {
            ProxyControl::Port => "Port (e)",
            ProxyControl::Start => "Start",
            ProxyControl::Stop => "Stop",
            ProxyControl::Refresh => "Refresh",
            ProxyControl::EnableAtLogin => {
                if rect.width < 18 {
                    "Login on"
                } else {
                    "Enable at login"
                }
            }
            ProxyControl::DisableAtLogin => {
                if rect.width < 19 {
                    "Login off"
                } else {
                    "Disable at login"
                }
            }
            ProxyControl::Close => "Close",
        };
        let disabled = matches!(control, ProxyControl::Start) && running
            || matches!(control, ProxyControl::Stop) && !running
            || matches!(control, ProxyControl::EnableAtLogin) && installed
            || matches!(control, ProxyControl::DisableAtLogin) && !installed;
        let selected = manager.selected_control() == control;
        let style = if selected {
            Style::default()
                .fg(Color::Black)
                .bg(if disabled { MUTED } else { ROUTE })
                .add_modifier(Modifier::BOLD)
        } else if disabled {
            Style::default().fg(MUTED).add_modifier(Modifier::DIM)
        } else if matches!(control, ProxyControl::Start | ProxyControl::EnableAtLogin) {
            Style::default().fg(CONNECTED)
        } else if matches!(control, ProxyControl::Stop | ProxyControl::DisableAtLogin) {
            Style::default().fg(WARNING)
        } else {
            Style::default().fg(MUTED)
        };
        frame.render_widget(
            Paragraph::new(format!("[{label}]"))
                .alignment(Alignment::Center)
                .style(style),
            rect,
        );
    }
}

pub(super) fn draw_confirmation(frame: &mut ratatui::Frame, area: Rect, message: &str) {
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw(message),
            Line::raw(""),
            Line::styled(
                "Enter/y confirm · n/Esc cancel",
                Style::default().fg(WARNING),
            ),
        ])
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .block(panel(" Confirm ", true)),
        area,
    );
}
