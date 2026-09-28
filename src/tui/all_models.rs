use super::*;

#[derive(Default)]
pub(super) struct Filter {
    pub(super) query: String,
    pub(super) active: bool,
    pub(super) detail_scroll: usize,
}

pub(super) fn areas(panel: Rect) -> (Rect, Rect) {
    let height = if panel.height >= 7 { 3 } else { 1 }.min(panel.height);
    (
        Rect::new(panel.x, panel.y, panel.width, height),
        Rect::new(
            panel.x,
            panel.y + height,
            panel.width,
            panel.height.saturating_sub(height),
        ),
    )
}

pub(super) fn clear_area(search: Rect) -> Option<Rect> {
    let inner = if search.height >= 3 {
        panel_inner(search)
    } else {
        search
    };
    (inner.width >= 18 && inner.height > 0).then(|| Rect::new(inner.right() - 9, inner.y, 9, 1))
}

impl App {
    pub(super) fn filtered_global_models(&self) -> Vec<GlobalModelRef> {
        let terms: Vec<_> = self
            .all_models_filter
            .query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        self.all_managed_models()
            .into_iter()
            .filter(|entry| {
                if terms.is_empty() {
                    return true;
                }
                let haystack = format!(
                    "{} {} {} {}",
                    entry.profile_name,
                    entry.profile_id,
                    entry.model.label(),
                    entry.model.id
                )
                .to_lowercase();
                terms.iter().all(|word| haystack.contains(word))
            })
            .collect()
    }

    pub(super) fn reject_empty_global_filter(&mut self) -> bool {
        if self.view_mode == ViewMode::AllEnabled
            && !self.all_models_filter.query.is_empty()
            && self.filtered_global_models().get(self.model_idx).is_none()
        {
            self.set_error("No matching model · clear the filter or select a result");
            return true;
        }
        false
    }

    pub(super) fn update_global_filter(&mut self, query: String) {
        let previous = self
            .filtered_global_models()
            .get(self.model_idx)
            .map(|e| (e.profile_id.clone(), e.model.id.clone()));
        self.all_models_filter.query = query;
        let rows = self.filtered_global_models();
        self.model_idx = previous
            .and_then(|(provider, model)| {
                rows.iter()
                    .position(|e| e.profile_id == provider && e.model.id == model)
            })
            .unwrap_or(0);
        self.model_offset = 0;
        self.all_models_filter.detail_scroll = 0;
    }

    pub(super) fn global_filter_key(&mut self, key: KeyEvent) {
        let mut query = self.all_models_filter.query.clone();
        match key.code {
            KeyCode::Esc if !query.is_empty() => query.clear(),
            KeyCode::Esc | KeyCode::Enter => self.all_models_filter.active = false,
            KeyCode::Tab | KeyCode::Down | KeyCode::Up => {
                self.all_models_filter.active = false;
                if key.code != KeyCode::Tab {
                    self.move_selection(if key.code == KeyCode::Down { 1 } else { -1 });
                }
            }
            KeyCode::Backspace => {
                query.pop();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => query.clear(),
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                query.push(ch)
            }
            _ => {}
        }
        if query != self.all_models_filter.query {
            self.update_global_filter(query);
        }
    }

    pub(super) fn draw_global_filter(
        &self,
        frame: &mut ratatui::Frame,
        area: Rect,
        count: usize,
        total: usize,
    ) {
        let inner = if area.height >= 3 {
            frame.render_widget(
                panel(
                    if self.all_models_filter.active {
                        " Filter · Enter done / Esc clear "
                    } else {
                        " Filter · / model or provider "
                    },
                    self.all_models_filter.active,
                ),
                area,
            );
            panel_inner(area)
        } else {
            area
        };
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let has_query = !self.all_models_filter.query.is_empty();
        let clear = has_query.then(|| clear_area(area)).flatten();
        let available = inner
            .width
            .saturating_sub(if clear.is_some() { 10 } else { 0 });
        let count_text = format!(" {count}/{total}");
        let input_width =
            usize::from(available).saturating_sub(UnicodeWidthStr::width(count_text.as_str()) + 1);
        let query = if has_query || self.all_models_filter.active {
            &self.all_models_filter.query
        } else {
            "model / provider"
        };
        let mut tail = String::new();
        let mut used = usize::from(self.all_models_filter.active);
        for ch in query.chars().rev() {
            let size = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + size > input_width {
                break;
            }
            tail.push(ch);
            used += size;
        }
        let input: String = tail.chars().rev().collect();
        let input = format!(
            "{input}{}{}",
            if self.all_models_filter.active {
                "▏"
            } else {
                ""
            },
            " ".repeat(input_width.saturating_sub(used))
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    input,
                    Style::default().fg(if has_query || self.all_models_filter.active {
                        ROUTE
                    } else {
                        MUTED
                    }),
                ),
                Span::styled(count_text, Style::default().fg(FIELD_LABEL)),
            ])),
            Rect::new(inner.x, inner.y, available, 1),
        );
        if let Some(rect) = clear {
            frame.render_widget(
                Paragraph::new(toolbar::action_line("Clear", MUTED, false, self.theme))
                    .alignment(Alignment::Center),
                rect,
            );
        }
    }

    pub(super) fn draw_global_model_details(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let active = self.focus == Focus::Details;
        frame.render_widget(panel(" Model configuration ", active), area);
        let inner = panel_inner(area);
        let Some(entry) = self.filtered_global_models().get(self.model_idx).cloned() else {
            frame.render_widget(
                Paragraph::new("No matching model · clear the filter")
                    .style(Style::default().fg(MUTED))
                    .wrap(Wrap { trim: false }),
                inner,
            );
            return;
        };
        let profile = &self.config.profiles[&entry.profile_id];
        let is_default =
            canonical_model_id(&entry.model.id) == canonical_model_id(&profile.default_model);
        let field = |name: &str, value: String, color| {
            Line::from(vec![
                Span::styled(format!("{name}  "), Style::default().fg(FIELD_LABEL)),
                Span::styled(
                    value,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
            ])
        };
        let mut lines = vec![
            field("Model", entry.model.label().to_owned(), Color::White),
            field("ID", entry.model.id.clone(), ROUTE),
            field("Provider", entry.profile_name, Color::White),
            field("Provider ID", entry.profile_id, MUTED),
            field("API", profile.api_format.label().into(), DATA_SECONDARY),
            field("Base URL", profile.base_url.clone(), Color::White),
            field(
                "Status",
                if self.pi_enabled {
                    "Configured"
                } else if entry.enabled {
                    "Enabled"
                } else {
                    "Disabled"
                }
                .into(),
                if self.pi_enabled || entry.enabled {
                    ENABLED
                } else {
                    MUTED
                },
            ),
            field(
                "Default",
                if is_default { "Yes" } else { "No" }.into(),
                if is_default { DEFAULT_MODEL } else { MUTED },
            ),
            field(
                "1M",
                if entry.model.id.ends_with("[1m]") {
                    "On"
                } else {
                    "Off"
                }
                .into(),
                DEFAULT_MODEL,
            ),
            field(
                "Context",
                entry
                    .model
                    .context_window
                    .map_or("Unset".into(), |v| v.to_string()),
                DATA_SECONDARY,
            ),
            field(
                "Output cap",
                entry
                    .model
                    .max_output_tokens
                    .map_or("Unset".into(), |v| v.to_string()),
                DATA_SECONDARY,
            ),
        ];
        if let Some(reasoning) = &entry.model.reasoning_max {
            lines.push(field("Reasoning", reasoning.clone(), DATA_SECONDARY));
        }
        let roles: Vec<_> = profile
            .aliases
            .iter()
            .filter(|(_, id)| canonical_model_id(id) == canonical_model_id(&entry.model.id))
            .map(|(role, _)| role)
            .collect();
        if !roles.is_empty() {
            lines.push(field("Roles", roles.join(", "), DEFAULT_MODEL));
        }
        if let Some(description) = &entry.model.description {
            lines.push(field("Description", description.clone(), MUTED));
        }
        lines.push(Line::styled(
            "Enter: open provider · Tab: change panel",
            Style::default().fg(MUTED),
        ));
        let lines: Vec<_> = lines
            .into_iter()
            .flat_map(|line| {
                let segments = line
                    .spans
                    .into_iter()
                    .map(|s| (s.content.into_owned(), s.style))
                    .collect();
                wrap_styled_segments(segments, inner.width)
            })
            .collect();
        self.all_models_filter.detail_scroll = self
            .all_models_filter
            .detail_scroll
            .min(lines.len().saturating_sub(inner.height as usize));
        let line_count = lines.len();
        frame.render_widget(
            Paragraph::new(lines).scroll((
                self.all_models_filter.detail_scroll.min(u16::MAX as usize) as u16,
                0,
            )),
            inner,
        );
        draw_scrollbar(
            frame,
            area,
            line_count,
            self.all_models_filter.detail_scroll,
            usize::from(inner.height),
        );
    }
}
