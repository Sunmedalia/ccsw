use super::*;
use crate::grok::{self as native, Preferences};

#[derive(Clone)]
pub(super) enum Dialog {
    Settings {
        fields: Vec<FormField>,
        selected: usize,
        original: Preferences,
        appearance: Option<theme::Appearance>,
        discard: bool,
    },
    Import {
        candidate: Box<native::Import>,
        original: native::Settings,
        scroll: u16,
    },
    Reconnect {
        conflicts: Vec<String>,
        preferred: Option<String>,
        scroll: u16,
    },
}
impl Dialog {
    fn buttons(&self) -> usize {
        2
    }
    fn message(&self) -> String {
        match self {
            Self::Import { candidate, .. } => candidate.preview.clone(),
            Self::Reconnect { conflicts, .. } => format!(
                "These Grok fields changed outside CCSW:\n\n{}\n\nEnter replaces these managed fields with CCSW values. Esc cancels.",
                conflicts.join("\n")
            ),
            Self::Settings { .. } => String::new(),
        }
    }
}
fn settings(fields: &[FormField]) -> Result<Preferences> {
    let string = |i: usize| {
        let v = fields[i].value.trim();
        (!v.is_empty() && v != "inherit").then(|| v.to_string())
    };
    let boolean = |i: usize| match fields[i].value.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    };
    let result = Preferences {
        default: string(0),
        web_search: string(1),
        fork_secondary_model: string(2),
        reasoning_effort: string(3),
        permission_mode: string(4),
        compact_mode: boolean(5),
        show_thinking_blocks: boolean(6),
    };
    result.validate()?;
    Ok(result)
}
impl App {
    pub(super) fn open_grok_preferences(&mut self, appearance: Option<theme::Appearance>) {
        let p = self.config.grok.preferences.clone();
        let mut fields = vec![
            field("Default model", p.default.as_deref().unwrap_or("")),
            field("Web search model", p.web_search.as_deref().unwrap_or("")),
            field(
                "Fork secondary model",
                p.fork_secondary_model.as_deref().unwrap_or(""),
            ),
            choice_field(
                "Reasoning effort",
                p.reasoning_effort.as_deref().unwrap_or("inherit"),
                &[
                    "inherit", "none", "minimal", "low", "medium", "high", "xhigh", "max",
                ],
            ),
            choice_field(
                "Permission mode",
                p.permission_mode.as_deref().unwrap_or("inherit"),
                &["inherit", "default", "ask", "auto", "always-approve"],
            ),
            choice_field(
                "Compact mode",
                p.compact_mode
                    .map(|v| if v { "true" } else { "false" })
                    .unwrap_or("inherit"),
                &["inherit", "true", "false"],
            ),
            choice_field(
                "Show thinking",
                p.show_thinking_blocks
                    .map(|v| if v { "true" } else { "false" })
                    .unwrap_or("inherit"),
                &["inherit", "true", "false"],
            ),
        ];
        // Alt+E enables custom reasoning effort names.
        fields[3].cursor = fields[3].char_count();
        self.modal = Some(Modal::Grok(Box::new(Dialog::Settings {
            fields,
            selected: 0,
            original: p,
            appearance,
            discard: false,
        })));
    }
    pub(super) fn handle_grok_key(&mut self, key: KeyEvent) -> Result<bool> {
        if !self.grok_enabled
            || self
                .provider_editor
                .as_ref()
                .is_some_and(|e| e.search_active)
        {
            return Ok(false);
        }
        if self.home_grok_oauth_selected()
            && matches!(
                key.code,
                KeyCode::Enter | KeyCode::Char('l' | 'e' | 'E' | 'p' | ' ')
            )
        {
            self.open_grok_auth();
            return Ok(true);
        }
        match key.code {
            KeyCode::Char('o') => self.open_grok_auth(),
            KeyCode::Char('i') => {
                let mut current = self.config.grok.clone();
                current.profiles = self.config.profiles.clone();
                let candidate = native::prepare_import(&self.grok_home, &current)?;
                self.modal = Some(Modal::Grok(Box::new(Dialog::Import {
                    candidate: Box::new(candidate),
                    original: current,
                    scroll: 0,
                })));
            }
            KeyCode::Char('p') => self.apply_grok(false),
            KeyCode::Char('s') => {
                self.status = native::status(&self.paths, &self.grok_home)?;
                self.status_error = false;
            }
            KeyCode::Char('D') => {
                let conflicts = native::disconnect(&self.paths, &self.grok_home)?;
                self.status = if conflicts.is_empty() {
                    "Grok disconnected; managed fields restored".into()
                } else {
                    format!(
                        "Grok disconnected; preserved external changes: {}",
                        conflicts.join(", ")
                    )
                };
                self.status_error = false;
            }
            KeyCode::Char('P') => {
                self.status = "Grok connects directly to providers".into();
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn grok_preferred(&self) -> Option<String> {
        if self.view_mode == ViewMode::AllEnabled {
            return self
                .all_managed_models()
                .get(self.model_idx)
                .filter(|e| {
                    self.config.profiles[&e.profile_id].enabled
                        && !self.config.profiles[&e.profile_id]
                            .disabled_models
                            .contains(&e.model.id)
                })
                .map(|e| native::model_key(&self.config.grok, &e.profile_id, &e.model.id));
        }
        self.selected_profile_id().and_then(|id| {
            let profile = &self.config.profiles[&id];
            let model = if self.view_mode == ViewMode::Home {
                profile.default_model.clone()
            } else {
                self.selected_model()
                    .map(|m| m.id.clone())
                    .unwrap_or_else(|| profile.default_model.clone())
            };
            (profile.enabled
                && discovery::active_models(profile, &[])
                    .iter()
                    .any(|m| m.id == model))
            .then(|| native::model_key(&self.config.grok, &id, &model))
        })
    }
    pub(super) fn apply_grok(&mut self, reconnect: bool) {
        if self.home_grok_oauth_selected() {
            self.open_grok_auth();
            return;
        }
        let preferred = self.grok_preferred();
        if !reconnect {
            match native::conflicts(&self.paths, &self.grok_home) {
                Ok(conflicts) if !conflicts.is_empty() => {
                    self.modal = Some(Modal::Grok(Box::new(Dialog::Reconnect {
                        conflicts,
                        preferred,
                        scroll: 0,
                    })));
                    return;
                }
                Err(e) => {
                    self.set_error(format!("Cannot connect Grok: {e:#}"));
                    return;
                }
                _ => {}
            }
        }
        self.write_grok(preferred, reconnect);
    }
    fn write_grok(&mut self, preferred: Option<String>, reconnect: bool) {
        let result = (|| -> Result<()> {
            self.config = self.load_client_config()?;
            // Persist the explicitly selected default for later automatic syncs.
            if let Some(preferred) = &preferred {
                self.config = self.update_client_config(|c| {
                    c.grok.preferences.default = Some(preferred.clone());
                    Ok(())
                })?;
            }
            let previous_default = self.config.grok.preferences.default.clone();
            let applied_default = native::apply(
                &self.paths,
                &self.grok_home,
                &self.config,
                preferred,
                reconnect,
            )?;
            if previous_default != applied_default
                && previous_default.as_deref().is_some_and(|key| {
                    key.starts_with("ccsw::") || self.config.grok.imports.values().any(|v| v == key)
                })
            {
                self.config = self.update_client_config(|c| {
                    if c.grok.preferences.default != previous_default {
                        anyhow::bail!("Grok default changed in another instance; reopen settings");
                    }
                    c.grok.preferences.default = applied_default;
                    Ok(())
                })?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.status =
                    "Grok synced · restart Grok, then use /model · saves now sync automatically"
                        .into();
                self.status_error = false;
            }
            Err(e) => self.set_error(format!("Local changes saved; Grok sync failed: {e:#}")),
        }
    }
    pub(super) fn sync_grok_after_edit(&mut self) {
        if self.grok_enabled && native::connected(&self.paths) {
            self.write_grok(None, false);
        }
    }
    pub(super) fn grok_dialog_key(&mut self, dialog: &mut Dialog, key: KeyEvent) -> Result<bool> {
        match dialog {
            Dialog::Settings {
                fields,
                selected,
                original,
                appearance,
                discard,
            } => {
                if *discard {
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Enter => {
                            if let Some(a) = appearance {
                                self.modal = Some(Modal::Appearance(a.clone()));
                            }
                            return Ok(true);
                        }
                        KeyCode::Esc | KeyCode::Char('n') => *discard = false,
                        _ => {}
                    }
                    return Ok(false);
                }
                if key.modifiers == KeyModifiers::ALT
                    && key.code == KeyCode::Char('e')
                    && *selected == 3
                {
                    fields[3].choices = &[];
                    fields[3].cursor = fields[3].char_count();
                    return Ok(false);
                }
                if key.modifiers == KeyModifiers::ALT
                    && key.code == KeyCode::Char('m')
                    && *selected < 3
                {
                    let native_settings = &self.config.grok;
                    let models: Vec<String> = self
                        .config
                        .profiles
                        .iter()
                        .flat_map(|(id, p)| {
                            discovery::active_models(p, &[])
                                .into_iter()
                                .map(move |m| native::model_key(native_settings, id, &m.id))
                        })
                        .collect();
                    if !models.is_empty() {
                        let next = fields[*selected].value.as_str();
                        let index = models
                            .iter()
                            .position(|m| m == next)
                            .map(|i| (i + 1) % models.len())
                            .unwrap_or(0);
                        fields[*selected].value = models[index].clone();
                        fields[*selected].cursor = fields[*selected].char_count();
                    }
                    return Ok(false);
                }
                match handle_form_key(fields, selected, key) {
                    FormOutcome::Close => {
                        if settings(fields).ok().as_ref() != Some(original) {
                            *discard = true;
                            return Ok(false);
                        }
                        if let Some(a) = appearance {
                            self.modal = Some(Modal::Appearance(a.clone()));
                        }
                        return Ok(true);
                    }
                    FormOutcome::Submit => {
                        let edited = settings(fields)?;
                        self.config = self.update_client_config(|c| {
                            if c.grok.preferences != *original && c.grok.preferences != edited {
                                anyhow::bail!(
                                    "Grok settings changed in another instance; reopen settings"
                                );
                            }
                            c.grok.preferences = edited.clone();
                            Ok(())
                        })?;
                        self.status =
                            "Grok settings saved · p connects · connected saves sync automatically"
                                .into();
                        self.status_error = false;
                        if let Some(a) = appearance {
                            self.modal = Some(Modal::Appearance(a.clone()));
                        }
                        return Ok(true);
                    }
                    _ => {}
                }
            }
            Dialog::Import {
                candidate,
                original,
                scroll,
            } => match key.code {
                KeyCode::Esc => return Ok(true),
                KeyCode::Enter | KeyCode::Char('i') => {
                    self.config = self.update_client_config(|c| {
                        if c.profiles != original.profiles
                            || c.grok.preferences != original.preferences
                            || c.grok.imports != original.imports
                        {
                            anyhow::bail!("Grok configuration changed; reopen import preview");
                        }
                        c.profiles = candidate.settings.profiles.clone();
                        c.grok.preferences = candidate.settings.preferences.clone();
                        c.grok.imports = candidate.settings.imports.clone();
                        Ok(())
                    })?;
                    self.profile_idx = 0;
                    self.model_idx = 0;
                    self.return_home();
                    self.status = "Imported Grok models and settings · p connects".into();
                    self.status_error = false;
                    return Ok(true);
                }
                KeyCode::Down | KeyCode::PageDown => *scroll = scroll.saturating_add(1),
                KeyCode::Up | KeyCode::PageUp => *scroll = scroll.saturating_sub(1),
                _ => {}
            },
            Dialog::Reconnect {
                preferred, scroll, ..
            } => match key.code {
                KeyCode::Esc => return Ok(true),
                KeyCode::Enter => {
                    self.write_grok(preferred.clone(), true);
                    return Ok(true);
                }
                KeyCode::Down | KeyCode::PageDown => *scroll = scroll.saturating_add(1),
                KeyCode::Up | KeyCode::PageUp => *scroll = scroll.saturating_sub(1),
                _ => {}
            },
        }
        Ok(false)
    }
    pub(super) fn grok_dialog_mouse(&mut self, mouse: MouseEvent, area: Rect) -> Result<()> {
        let Some(Modal::Grok(dialog)) = self.modal.as_ref() else {
            return Ok(());
        };
        let discard = matches!(dialog.as_ref(), Dialog::Settings { discard: true, .. });
        if let Some(button) = modal_button_rects(area, dialog.buttons())
            .iter()
            .position(|r| contains(*r, mouse.column, mouse.row))
        {
            let settings = matches!(dialog.as_ref(), Dialog::Settings { .. });
            let key = if discard {
                KeyEvent::new(
                    KeyCode::Char(if button == 0 { 'y' } else { 'n' }),
                    KeyModifiers::NONE,
                )
            } else if button == 1 {
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
            } else if settings {
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
            } else {
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
            };
            return self.handle_modal(key);
        }
        if let Some(Modal::Grok(dialog)) = self.modal.as_mut()
            && let Dialog::Settings {
                fields, selected, ..
            } = dialog.as_mut()
        {
            let inner = panel_inner(area);
            let content = Rect::new(
                inner.x,
                inner.y,
                inner.width,
                inner.height.saturating_sub(4),
            );
            if contains(content, mouse.column, mouse.row) {
                let (_, offset) = form_viewport(content, *selected);
                let index = offset + usize::from(mouse.row - content.y);
                if index < fields.len() {
                    *selected = index;
                    if !fields[index].choices.is_empty() {
                        cycle_choice(&mut fields[index], true);
                    }
                }
            }
        }
        Ok(())
    }
}
pub(super) fn draw_dialog(frame: &mut ratatui::Frame, area: Rect, dialog: &Dialog) {
    match dialog {
        Dialog::Settings {
            fields,
            selected,
            discard,
            ..
        } => {
            if *discard {
                frame.render_widget(
                    Paragraph::new(
                        "Discard unsaved Grok settings? · Enter/y discard · n keep editing",
                    )
                    .block(panel(" Grok settings ", true))
                    .wrap(Wrap { trim: false }),
                    area,
                );
                draw_modal_buttons(frame, area, &["Discard", "Keep editing"]);
            } else {
                draw_form(
                    frame,
                    area,
                    " Grok client settings ",
                    fields,
                    *selected,
                    false,
                );
                let inner = panel_inner(area);
                frame.render_widget(
                    Paragraph::new(
                        "Alt+M model choices · Alt+E custom effort · blank/inherit uses native defaults",
                    )
                    .style(Style::default().fg(MUTED))
                    .wrap(Wrap { trim: false }),
                    Rect::new(inner.x, inner.bottom().saturating_sub(4), inner.width, 2),
                );
                draw_modal_buttons(frame, area, &["Save (Ctrl+S)", "Cancel"]);
            }
        }
        Dialog::Import { scroll, .. } | Dialog::Reconnect { scroll, .. } => {
            frame.render_widget(
                panel(
                    if matches!(dialog, Dialog::Import { .. }) {
                        " Import Grok configuration "
                    } else {
                        " Reconnect Grok "
                    },
                    true,
                ),
                area,
            );
            let inner = panel_inner(area);
            frame.render_widget(
                Paragraph::new(dialog.message())
                    .scroll((*scroll, 0))
                    .wrap(Wrap { trim: false }),
                Rect::new(
                    inner.x,
                    inner.y,
                    inner.width,
                    inner.height.saturating_sub(3),
                ),
            );
            draw_modal_buttons(frame, area, &["Confirm", "Cancel"]);
        }
    }
}
