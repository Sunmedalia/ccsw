use super::*;
use crate::claude_preferences::{PRESETS, Settings};

#[derive(Clone)]
pub(super) struct PreferencesForm {
    original: Settings,
    pub fields: Vec<FormField>,
    pub selected: usize,
    current: serde_json::Value,
    pub(super) discard: bool,
    message: String,
}
impl PreferencesForm {
    pub fn new(settings: Settings, current: serde_json::Value) -> Self {
        let mut fields = vec![choice_field(
            "AI attribution",
            match settings.hide_attribution {
                None => "inherit",
                Some(true) => "hide",
                Some(false) => "show",
            },
            &["inherit", "hide", "show"],
        )];
        for (label, key) in PRESETS {
            let choices: &'static [&'static str] = match key {
                "ENABLE_TOOL_SEARCH" => &["inherit", "true", "false", "auto"],
                "CLAUDE_CODE_EFFORT_LEVEL" => {
                    &["inherit", "auto", "low", "medium", "high", "xhigh", "max"]
                }
                _ => &["inherit", "1", "0"],
            };
            fields.push(choice_field(
                label,
                settings
                    .env
                    .get(key)
                    .map(String::as_str)
                    .unwrap_or("inherit"),
                choices,
            ));
        }
        for (key, value) in &settings.env {
            if !PRESETS.iter().any(|(_, k)| k == key) {
                fields.push(field("Variable name", key));
                fields.push(secret_field("Value", value));
            }
        }
        Self {
            original: settings,
            fields,
            selected: 0,
            current,
            discard: false,
            message: String::new(),
        }
    }
    fn settings(&self) -> Result<Settings> {
        let mut settings = Settings {
            env: BTreeMap::new(),
            hide_attribution: match self.fields[0].value.as_str() {
                "hide" => Some(true),
                "show" => Some(false),
                _ => None,
            },
        };
        for (i, (_, key)) in PRESETS.iter().enumerate() {
            let value = &self.fields[i + 1].value;
            if value != "inherit" {
                settings.env.insert((*key).into(), value.clone());
            }
        }
        for pair in self.fields[6..].as_chunks::<2>().0 {
            let key = pair[0].value.trim();
            if key.is_empty() {
                anyhow::bail!("Enter a variable name or remove the empty row with Alt+D");
            }
            if PRESETS.iter().any(|(_, k)| *k == key) {
                anyhow::bail!("{key} has a preset above; edit that preset instead");
            }
            if settings
                .env
                .insert(key.into(), pair[1].value.clone())
                .is_some()
            {
                anyhow::bail!("Duplicate variable name: {key}");
            }
        }
        settings.validate()?;
        Ok(settings)
    }
    fn add(&mut self) {
        self.selected = self.fields.len();
        self.fields.push(field("Variable name", ""));
        self.fields.push(secret_field("Value", ""));
    }
    fn fill(&mut self) {
        for (i, v) in ["hide", "1", "true", "max", "1", "1"].iter().enumerate() {
            self.fields[i].value = (*v).into();
        }
        self.message = "Presets filled in draft; Ctrl+S saves".into();
    }
    fn dirty(&self) -> bool {
        self.settings().ok().as_ref() != Some(&self.original)
    }
}

impl App {
    pub(super) fn open_preferences(&mut self) {
        if self.pi_enabled || self.codex_ui.enabled {
            return;
        }
        match claude_config::settings_path().and_then(|path| {
            if path.exists() {
                Ok(serde_json::from_slice(&std::fs::read(path)?)?)
            } else {
                Ok(serde_json::json!({}))
            }
        }) {
            Ok(current) => {
                self.modal = Some(Modal::Preferences(PreferencesForm::new(
                    self.config.claude.clone(),
                    current,
                )))
            }
            Err(e) => self.set_error(format!("Could not read Claude settings: {e:#}")),
        }
    }
    pub(super) fn preferences_key(&mut self, form: &mut PreferencesForm, key: KeyEvent) -> bool {
        if form.discard {
            match key.code {
                KeyCode::Char('y') => return true,
                _ => {
                    form.discard = false;
                    form.message.clear();
                    return false;
                }
            }
        }
        if key.code == KeyCode::Esc {
            if !form.dirty() {
                return true;
            }
            form.discard = true;
            form.message =
                "Discard unsaved changes? y: discard · any other key: keep editing".into();
            return false;
        }
        if key.modifiers.contains(KeyModifiers::ALT) && key.code == KeyCode::Char('x') {
            if form.dirty() {
                form.message = "Save or discard this draft before disconnecting".into();
                return false;
            }
            if self.background.sync_running || self.background.proxy_running {
                form.message = "Wait for the current sync to finish".into();
                return false;
            }
            match claude_config::settings_path()
                .and_then(|path| sync::disconnect(&self.paths, &path))
            {
                Ok(conflicts) => {
                    self.background.connected = false;
                    self.background.queued_sync = None;
                    self.background.status = sync::Status::NotConnected;
                    self.status = if conflicts.is_empty() {
                        "Disconnected; previous Claude preferences restored".into()
                    } else {
                        format!(
                            "Disconnected; external edits preserved: {}",
                            conflicts.join(", ")
                        )
                    };
                    return true;
                }
                Err(e) => {
                    form.message = format!("Cannot disconnect: {e:#}");
                    return false;
                }
            }
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
            if let Some((index, preset)) = form.fields[6..]
                .as_chunks::<2>()
                .0
                .iter()
                .enumerate()
                .find_map(|(i, pair)| {
                    PRESETS
                        .iter()
                        .position(|(_, key)| *key == pair[0].value.trim())
                        .map(|preset| (6 + i * 2, preset + 1))
                })
            {
                let value = form.fields[index + 1].value.clone();
                form.fields[preset].value = value;
                form.fields.drain(index..index + 2);
                form.selected = preset;
                form.message = "This variable has a preset; review it above and save again".into();
                return false;
            }
            let result = form.settings().and_then(|edited| {
                self.update_client_config(|latest| {
                    // Merge unrelated preference edits; report conflicts without exposing values.
                    let mut merged = latest.claude.clone();
                    let keys: BTreeSet<_> =
                        form.original.env.keys().chain(edited.env.keys()).collect();
                    for key in keys {
                        let old = form.original.env.get(key);
                        let new = edited.env.get(key);
                        if old == new {
                            continue;
                        }
                        if merged.env.get(key) != old && merged.env.get(key) != new {
                            anyhow::bail!("{key} changed in another instance; reopen settings");
                        }
                        match new {
                            Some(v) => {
                                merged.env.insert(key.clone(), v.clone());
                            }
                            None => {
                                merged.env.remove(key);
                            }
                        }
                    }
                    if edited.hide_attribution != form.original.hide_attribution {
                        if merged.hide_attribution != form.original.hide_attribution
                            && merged.hide_attribution != edited.hide_attribution
                        {
                            anyhow::bail!(
                                "Attribution changed in another instance; reopen settings"
                            );
                        }
                        merged.hide_attribution = edited.hide_attribution;
                    }
                    latest.claude = merged;
                    Ok(())
                })
            });
            match result {
                Ok(config) => {
                    self.config = config;
                    self.status = "Claude settings saved · pending sync · p connects · restart Claude for startup settings".into();
                    self.status_error = false;
                    self.queue_sync(false, None);
                    return true;
                }
                Err(e) => {
                    form.message = format!("Cannot save: {e:#}");
                    return false;
                }
            }
        }
        if key.modifiers.contains(KeyModifiers::ALT) {
            match key.code {
                KeyCode::Char('p') => form.fill(),
                KeyCode::Char('n') => form.add(),
                KeyCode::Char('d') if form.selected >= 6 => {
                    let index = 6 + (form.selected - 6) / 2 * 2;
                    form.fields.drain(index..index + 2);
                    form.selected = index.min(form.fields.len() - 1);
                }
                KeyCode::Char('v') if form.selected >= 6 => {
                    let index = 7 + (form.selected - 6) / 2 * 2;
                    form.fields[index].secret = !form.fields[index].secret;
                }
                _ => {}
            }
        } else {
            let _ = handle_form_key(&mut form.fields, &mut form.selected, key);
        }
        false
    }
}

pub(super) fn draw_preferences(frame: &mut ratatui::Frame, area: Rect, form: &PreferencesForm) {
    draw_form(
        frame,
        area,
        " Claude settings · Ctrl+S save · Esc back ",
        &form.fields,
        form.selected,
        false,
    );
    let inner = panel_inner(area);
    let current = if form.selected == 0 {
        form.current
            .get("attribution")
            .map(|v| v.to_string())
            .unwrap_or_else(|| "not set".into())
    } else {
        let key = if form.selected < 6 {
            PRESETS[form.selected - 1].1
        } else {
            &form.fields[6 + (form.selected - 6) / 2 * 2].value
        };
        match form.current.get("env").and_then(|v| v.get(key)) {
            None => "not set".into(),
            Some(v) if form.selected < 6 => v.to_string(),
            Some(_) => "•••• (existing value)".into(),
        }
    };
    let status = if form.message.is_empty() {
        format!("Claude file: {current} · inherit = no override")
    } else {
        form.message.clone()
    };
    frame.render_widget(
        Paragraph::new(vec![Line::raw(status)]),
        Rect::new(inner.x, inner.bottom().saturating_sub(4), inner.width, 2),
    );
    for ((label, _), rect) in [("Delete", 'd'), ("Show", 'v'), ("Disconnect", 'x')]
        .into_iter()
        .zip(preference_actions(area))
    {
        frame.render_widget(
            Paragraph::new(format!("[{label}]")).style(button_style(
                false,
                false,
                label == "Delete" || label == "Disconnect",
            )),
            rect,
        );
    }
    draw_modal_buttons(
        frame,
        area,
        if form.discard {
            &["Discard", "Keep editing"]
        } else if area.width < 55 {
            &["Presets", "Add", "Save", "Back"]
        } else {
            &["Fill presets", "Add variable", "Save", "Cancel"]
        },
    );
}

pub(super) fn preference_actions(area: Rect) -> Vec<Rect> {
    modal_button_rects(area, 3)
        .into_iter()
        .map(|mut rect| {
            rect.y = area.bottom().saturating_sub(4);
            rect
        })
        .collect()
}
