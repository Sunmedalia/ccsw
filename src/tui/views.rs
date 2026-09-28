use super::*;

impl App {
    pub(super) fn draw(&mut self, frame: &mut ratatui::Frame) {
        self.draw_content(frame);
        let theme = match &self.modal {
            Some(Modal::Appearance(form)) => form.theme,
            Some(Modal::Preferences(form)) => form.return_theme.unwrap_or(self.theme),
            _ => self.theme,
        };
        theme.apply(frame.buffer_mut());
        if let Some(Modal::Appearance(form)) = &self.modal {
            theme::draw_preview(
                frame,
                modal_area_for(self.modal.as_ref().unwrap(), frame.area()),
                form,
            );
        }
    }

    fn draw_content(&mut self, frame: &mut ratatui::Frame) {
        let area = frame.area();
        self.screen = area;
        if area.width < 40 || area.height < 12 {
            frame.render_widget(
                Paragraph::new(
                    if self.view_mode == ViewMode::Home
                        && self.modal.is_none()
                        && !self.codex_ui.accounts
                        && self.grok_auth.page.is_none()
                    {
                        "CCSW · Terminal too small\nResize to at least 40 × 12\nq / Ctrl+C to quit"
                    } else {
                        "CCSW · Terminal too small\nResize to at least 40 × 12\nEsc to go back"
                    },
                )
                .wrap(Wrap { trim: false }),
                area,
            );
            return;
        }
        if matches!(self.modal, Some(Modal::Appearance(_) | Modal::Proxy(_))) {
            self.draw_client_tabs(frame, area);
            if let Some(modal) = &self.modal {
                self.draw_modal(frame, modal);
                self.draw_page_header_actions(frame, modal_area_for(modal, area));
            }
            return;
        }
        if matches!(self.modal, Some(Modal::Help(_))) {
            if let Some(previous) = &self.help_return {
                self.draw_client_tabs(frame, area);
                self.draw_modal(frame, previous);
                self.draw_page_header_actions(frame, modal_area_for(previous, area));
                self.draw_modal(frame, self.modal.as_ref().unwrap());
                return;
            }
        }
        if self.usage.active {
            self.draw_client_tabs(frame, area);
            if let Some(page) = &self.usage.page {
                self.draw_usage(frame, usage::page_area(area), page);
            }
            if let Some(modal) = &self.modal {
                self.draw_modal(frame, modal);
            }
            return;
        }
        if provider_workspace(area)
            && self.codex_ui.enabled
            && self.codex_ui.accounts
            && !self.home_account_selected()
        {
            self.return_home();
            self.select_home_index(1);
            self.focus = Focus::Details;
        }
        if provider_workspace(area)
            && self.grok_enabled
            && self.grok_auth.page.is_some()
            && !self.home_grok_oauth_selected()
        {
            self.return_home();
            self.select_home_index(1);
            self.focus = Focus::Details;
        }
        if self.codex_ui.enabled && self.codex_ui.accounts && !provider_workspace(area) {
            self.draw_codex_accounts(frame, area);
            self.draw_page_header_actions(frame, workspace_content_area(area));
            if let Some(modal) = &self.modal {
                self.draw_modal(frame, modal);
            }
            return;
        }
        if self.grok_enabled && self.grok_auth.page.is_some() && !provider_workspace(area) {
            self.draw_grok_accounts(frame, area);
            self.draw_page_header_actions(frame, workspace_content_area(area));
            if let Some(modal) = &self.modal {
                self.draw_modal(frame, modal);
            }
            return;
        }
        // Enter the selected workspace in place; resizing never replaces the selection.
        if provider_workspace(area)
            && self.view_mode == ViewMode::Home
            && !self.home_account_selected()
            && !self.home_grok_oauth_selected()
            && (!self.config.profiles.is_empty() || self.home_all_selected)
        {
            self.select_sidebar_index(self.home_selected_index());
        } else if !provider_workspace(area)
            && self.view_mode != ViewMode::Home
            && self.focus == Focus::Profiles
        {
            self.focus = Focus::Models;
        }
        self.draw_client_tabs(frame, area);
        let page = self.provider_page_layout(area);
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .style(Style::default().bg(theme::SURFACE))
                .border_style(Style::default().fg(theme::ACTIVE_EDGE))
                .title(self.provider_page_title()),
            page.shell,
        );
        self.draw_provider_actions(frame, &page);
        let ui = ui_content_areas(area, page.content, page.status, self.focus, self.view_mode);
        if let Some(profiles) = ui.profiles {
            self.draw_profiles(frame, profiles);
        }
        if let Some(models) = ui.models {
            self.draw_models(frame, models);
        }
        if let Some(details) = ui.details {
            if self.view_mode == ViewMode::AllEnabled {
                self.draw_global_model_details(frame, details);
            } else {
                self.draw_details(frame, details, self.focus == Focus::Details);
            }
        }
        self.draw_status(frame, ui.footer);
        if let Some(modal) = &self.modal {
            self.draw_modal(frame, modal);
        }
        self.draw_codex_overlay(frame, area);
    }

    fn draw_page_header_actions(&self, frame: &mut ratatui::Frame, area: Rect) {
        let [(help, back)] = page_header_actions(area);
        frame.render_widget(
            Paragraph::new(toolbar::action_line("Help [?]", ROUTE, false, self.theme))
                .alignment(Alignment::Center),
            help,
        );
        frame.render_widget(
            Paragraph::new(toolbar::action_line(
                if area.width < 56 {
                    "Back [q]"
                } else {
                    "Back [Esc/q]"
                },
                FIELD_LABEL,
                false,
                self.theme,
            ))
            .alignment(Alignment::Center),
            back,
        );
    }

    fn provider_page_title(&self) -> Line<'static> {
        let mut title = vec![
            Span::styled(
                format!(" {} / Providers ", self.config_tab().label()),
                Style::default().fg(ROUTE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · ", Style::default().fg(MUTED)),
        ];
        let line = match self.view_mode {
            ViewMode::Home => Line::from(vec![
                Span::styled(
                    "PROVIDERS  ",
                    Style::default()
                        .fg(FIELD_LABEL)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{} configured", self.config.profiles.len()),
                    Style::default().fg(MUTED),
                ),
            ]),
            ViewMode::Provider => Line::from(vec![
                Span::styled(
                    self.selected_profile()
                        .map_or("No provider", |p| p.name.as_str())
                        .to_owned(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  /  ", Style::default().fg(MUTED)),
                Span::styled(
                    self.selected_model()
                        .map_or_else(|| "No model".into(), |m| m.label().to_owned()),
                    Style::default()
                        .fg(DEFAULT_MODEL)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            ViewMode::AllEnabled => Line::from(vec![
                Span::styled(
                    "ALL MODELS  ",
                    Style::default()
                        .fg(FIELD_LABEL)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{} enabled · all providers", self.all_enabled_model_count()),
                    Style::default().fg(ENABLED),
                ),
            ]),
        };
        title.extend(line.spans);
        title.push(Span::raw(" "));
        Line::from(title)
    }

    pub(super) fn draw_profiles(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let ids = self.profile_ids();
        let sidebar = provider_workspace(self.screen);
        let is_home = self.view_mode == ViewMode::Home || sidebar;
        let content_width = area
            .width
            .saturating_sub(2 + u16::from(self.theme.terminal_background()));
        let mut item_heights = Vec::with_capacity(ids.len().saturating_add(1));
        let mut items = Vec::with_capacity(ids.len().saturating_add(1));
        if is_home {
            let lines = if sidebar {
                vec![
                    Line::styled("All Models", Style::default().add_modifier(Modifier::BOLD)),
                    Line::from(vec![
                        Span::styled(
                            format!("{} enabled", self.all_enabled_model_count()),
                            Style::default().fg(ENABLED),
                        ),
                        Span::styled(" · all providers", Style::default().fg(MUTED)),
                    ]),
                ]
            } else {
                self.all_models_home_lines(content_width)
            };
            item_heights.push(lines.len());
            items.push(ListItem::new(lines));
            if self.codex_ui.enabled {
                let account = if sidebar {
                    vec![
                        Line::styled(
                            "ChatGPT Account",
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                        Line::styled(
                            "Subscription · saved logins",
                            Style::default().fg(FIELD_LABEL),
                        ),
                    ]
                } else {
                    self.chatgpt_provider_lines(content_width)
                };
                item_heights.push(account.len());
                items.push(ListItem::new(account));
            }
        }
        if is_home && self.grok_enabled {
            let account = if sidebar {
                vec![
                    Line::styled("Grok OAuth", Style::default().add_modifier(Modifier::BOLD)),
                    Line::styled("Account · native models", Style::default().fg(FIELD_LABEL)),
                ]
            } else {
                self.grok_oauth_provider_lines(content_width)
            };
            item_heights.push(account.len());
            items.push(ListItem::new(account));
        }
        items.extend(ids.iter().map(|id| {
            let profile = &self.config.profiles[id];
            if is_home {
                let mut lines = if sidebar {
                    vec![
                        Line::from(vec![
                            Span::styled(
                                if self.pi_enabled || profile.enabled {
                                    "● "
                                } else {
                                    "○ "
                                },
                                Style::default().fg(if self.pi_enabled || profile.enabled {
                                    ENABLED
                                } else {
                                    MUTED
                                }),
                            ),
                            Span::styled(
                                profile.name.clone(),
                                Style::default().add_modifier(Modifier::BOLD),
                            ),
                        ]),
                        Line::from(vec![
                            Span::styled(
                                format!("{}  ", profile.api_format.label()),
                                Style::default().fg(FIELD_LABEL),
                            ),
                            Span::styled(
                                format!("{} models", self.catalog_models_for(id).len()),
                                Style::default().fg(MUTED),
                            ),
                        ]),
                    ]
                } else {
                    self.provider_home_lines(id, content_width)
                };
                // Keep identity, default model and availability visible when a
                // full provider card is taller than the short viewport.
                if area.height < 12
                    && let Some(endpoint) = lines
                        .iter()
                        .position(|line| line.to_string().trim_start().starts_with("Endpoint:"))
                {
                    lines.truncate(endpoint);
                }
                item_heights.push(lines.len());
                ListItem::new(lines)
            } else {
                item_heights.push(1);
                ListItem::new(Line::from(vec![
                    Span::styled("● ", Style::default().fg(ENABLED)),
                    Span::raw(profile.name.clone()),
                    Span::styled(format!("  {id}"), Style::default().fg(MUTED)),
                ]))
            }
        }));
        let title = if sidebar {
            " Providers · a add "
        } else {
            " Providers "
        };
        let selected = if is_home {
            self.home_selected_index()
        } else {
            self.profile_idx
        };
        let mut state = ListState::default()
            .with_offset(self.profile_offset)
            .with_selected((!items.is_empty()).then_some(selected));
        let item_count = items.len();
        frame.render_stateful_widget(
            List::new(items)
                .block(panel(title, self.focus == Focus::Profiles))
                .highlight_style(Style::default().bg(theme::PROVIDER_SELECTION))
                .highlight_symbol(self.theme.selection_symbol()),
            area,
            &mut state,
        );
        draw_header_add_button(frame, area);
        self.profile_offset = state.offset();
        let visible_items = if is_home {
            visible_variable_items(
                &item_heights,
                self.profile_offset,
                usize::from(area.height.saturating_sub(2)),
            )
        } else {
            usize::from(area.height.saturating_sub(2))
        };
        draw_scrollbar(frame, area, item_count, selected, visible_items);
    }

    pub(super) fn draw_models(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        if self.view_mode == ViewMode::AllEnabled {
            let models = self.filtered_global_models();
            self.model_idx = self.model_idx.min(models.len().saturating_sub(1));
            let (search, area) = all_models::areas(area);
            self.draw_global_filter(frame, search, models.len(), self.all_managed_models().len());
            let enabled_count = models.iter().filter(|entry| entry.enabled).count();
            let items = models
                .iter()
                .map(|entry| {
                    ListItem::new(vec![
                        Line::from(vec![
                            Span::styled(
                                if entry.enabled { "● " } else { "○ " },
                                Style::default().fg(if entry.enabled { ENABLED } else { MUTED }),
                            ),
                            Span::styled(
                                entry.profile_name.clone(),
                                Style::default().fg(ROUTE).add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!("  {}", entry.profile_id),
                                Style::default().fg(MUTED),
                            ),
                            Span::raw("  ·  "),
                            Span::styled(
                                entry.model.label().to_owned(),
                                Style::default().add_modifier(Modifier::BOLD),
                            ),
                        ]),
                        Line::from(vec![
                            Span::styled("  ", Style::default()),
                            Span::styled(entry.model.id.clone(), Style::default().fg(MUTED)),
                        ]),
                    ])
                })
                .collect::<Vec<_>>();
            let mut state = ListState::default()
                .with_offset(self.model_offset)
                .with_selected((!items.is_empty()).then_some(self.model_idx));
            let title = if self.pi_enabled {
                format!(" All models · {} configured ", models.len())
            } else {
                format!(" All models · {enabled_count}/{} enabled ", models.len())
            };
            frame.render_stateful_widget(
                List::new(items)
                    .block(panel(&title, self.focus == Focus::Models))
                    .highlight_style(Style::default().bg(theme::PROVIDER_SELECTION))
                    .highlight_symbol(self.theme.selection_symbol()),
                area,
                &mut state,
            );
            self.model_offset = state.offset();
            draw_scrollbar(
                frame,
                area,
                models.len(),
                self.model_idx,
                usize::from(area.height.saturating_sub(2) / 2),
            );
            if models.is_empty() {
                frame.render_widget(
                    Paragraph::new("No matching models.\nClear the filter or enable a provider.")
                        .style(Style::default().fg(MUTED))
                        .alignment(Alignment::Center)
                        .block(panel(&title, self.focus == Focus::Models)),
                    area,
                );
            }
            draw_header_add_button(frame, area);
            return;
        }
        if self.view_mode == ViewMode::Provider {
            let fallback;
            let editor = match &self.provider_editor {
                Some(e) => e,
                None => {
                    fallback = self.create_route_editor();
                    match &fallback {
                        Some(e) => e,
                        None => {
                            frame.render_widget(
                                Paragraph::new("No provider selected")
                                    .style(Style::default().fg(MUTED))
                                    .alignment(Alignment::Center)
                                    .block(panel(" Models ", false)),
                                area,
                            );
                            return;
                        }
                    }
                }
            };

            let search_area = Rect::new(area.x, area.y, area.width, 3);
            let list_area = Rect::new(
                area.x,
                area.y + 3,
                area.width,
                area.height.saturating_sub(3),
            );

            let search_title = if editor.search_active {
                " Search · Esc clear / leave "
            } else {
                " Search · / to type "
            };
            let search_block = panel(search_title, editor.search_active);
            let search_inner = panel_inner(search_area);
            frame.render_widget(search_block, search_area);
            if search_inner.width > 0 && search_inner.height > 0 {
                let filtered_len = editor.filtered_indices().len();
                let total_len = editor.catalog.len();
                let enabled_len = editor
                    .catalog
                    .iter()
                    .filter(|m| editor.is_enabled(&m.id))
                    .count();
                let stats_text = if self.pi_enabled {
                    format!("({filtered_len}/{total_len} models)")
                } else {
                    format!("({filtered_len}/{total_len} models · {enabled_len} enabled)")
                };
                let query_text = if editor.query.is_empty() {
                    if editor.search_active {
                        "Search by name or model ID…".to_owned()
                    } else {
                        "/ Filter models…".to_owned()
                    }
                } else {
                    editor.query.clone()
                };
                let line = Line::from(vec![
                    Span::styled(" ", Style::default()),
                    Span::styled(
                        query_text,
                        if editor.query.is_empty() {
                            Style::default().fg(MUTED)
                        } else {
                            Style::default()
                                .fg(Color::White)
                                .add_modifier(Modifier::BOLD)
                        },
                    ),
                    if editor.search_active {
                        Span::styled("▌", Style::default().fg(ROUTE))
                    } else {
                        Span::raw("")
                    },
                    if search_inner.width >= 55 {
                        Span::styled(format!("  {stats_text}"), Style::default().fg(MUTED))
                    } else {
                        Span::raw("")
                    },
                ]);
                frame.render_widget(Paragraph::new(line), search_inner);
            }

            let filtered = editor.filtered_indices();
            let list_title = if self.pi_enabled {
                " Models · a add · ◆ default  ● configured "
            } else {
                " Models · a add · ◆ default  ● enabled  ○ disabled "
            };
            let block = panel(
                list_title,
                self.focus == Focus::Models && !editor.search_active,
            );
            let inner = panel_inner(list_area);
            frame.render_widget(block, list_area);
            draw_header_add_button(frame, list_area);
            if inner.width == 0 || inner.height == 0 {
                return;
            }

            let items: Vec<ListItem> = filtered
                .iter()
                .map(|&idx| {
                    let model = &editor.catalog[idx];
                    let is_en = editor.is_enabled(&model.id);
                    let is_1m = editor.one_m.contains(&model.id);
                    let is_def = model.id == editor.default_model;
                    let is_req = editor.is_required(&model.id) && !is_def;

                    let marker = if is_def && is_en {
                        "◆"
                    } else if is_def && !is_en {
                        "◇"
                    } else if is_req {
                        "◈"
                    } else if is_en {
                        "●"
                    } else {
                        "○"
                    };
                    let marker_color = if is_def && is_en {
                        DEFAULT_MODEL
                    } else if is_def && !is_en {
                        MUTED
                    } else if is_req || is_en {
                        ENABLED
                    } else {
                        MUTED
                    };

                    let spans = vec![
                        Span::styled(
                            format!("{marker} "),
                            Style::default()
                                .fg(marker_color)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(model.label(), Style::default().fg(Color::White)),
                        Span::styled(
                            if model.label() != model.id {
                                format!("  {}", model.id)
                            } else {
                                String::new()
                            },
                            Style::default().fg(MUTED),
                        ),
                        Span::styled(
                            if is_1m { "  [1M]" } else { "" },
                            Style::default().fg(DEFAULT_MODEL),
                        ),
                    ];
                    ListItem::new(Line::from(spans))
                })
                .collect();

            let offset = route_editor_offset(editor, inner.height);
            let mut state = ListState::default()
                .with_offset(offset)
                .with_selected((!items.is_empty()).then_some(editor.selected));
            frame.render_stateful_widget(
                List::new(items)
                    .highlight_style(Style::default().bg(theme::PROVIDER_SELECTION))
                    .highlight_symbol(self.theme.selection_symbol()),
                inner,
                &mut state,
            );
            draw_scrollbar(
                frame,
                list_area,
                filtered.len(),
                editor.selected,
                usize::from(inner.height),
            );
            return;
        }

        let models = self.models();
        let manual: BTreeMap<_, _> = self
            .selected_profile()
            .map(|profile| {
                profile
                    .models
                    .iter()
                    .map(|model| (model.id.as_str(), ()))
                    .collect()
            })
            .unwrap_or_default();
        let items = models
            .iter()
            .map(|model| {
                let source = if manual.contains_key(model.id.as_str()) {
                    "manual"
                } else {
                    "gateway"
                };
                ListItem::new(vec![
                    Line::from(Span::raw(model.label())),
                    Line::from(vec![
                        Span::styled(&model.id, Style::default().fg(MUTED)),
                        Span::styled(format!("  {source}"), Style::default().fg(ROUTE)),
                    ]),
                ])
            })
            .collect::<Vec<_>>();
        let title = if self.pi_enabled {
            " Configured models "
        } else {
            " Enabled models "
        };
        let mut state = ListState::default()
            .with_offset(self.model_offset)
            .with_selected((!items.is_empty()).then_some(self.model_idx));
        frame.render_stateful_widget(
            List::new(items)
                .block(panel(title, self.focus == Focus::Models))
                .highlight_style(Style::default().bg(theme::PROVIDER_SELECTION))
                .highlight_symbol(self.theme.selection_symbol()),
            area,
            &mut state,
        );
        self.model_offset = state.offset();
        draw_scrollbar(
            frame,
            area,
            models.len(),
            self.model_idx,
            usize::from(area.height.saturating_sub(2) / 2),
        );
    }

    pub(super) fn draw_showcase(
        &self,
        frame: &mut ratatui::Frame,
        area: Rect,
        editor: &RouteEditor,
        profile: &Profile,
    ) {
        frame.render_widget(panel(" Selected model · e edit ", true), area);
        let inner = panel_inner(area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let filtered = editor.filtered_indices();
        let Some(&idx) = filtered.get(editor.selected) else {
            frame.render_widget(
                Paragraph::new("No matching model · clear the search or add a model")
                    .style(Style::default().fg(MUTED))
                    .alignment(Alignment::Center),
                inner,
            );
            return;
        };
        let model = &editor.catalog[idx];
        let effective_id = editor.effective_id(&model.id);
        let is_default = model.id == editor.default_model;
        let is_enabled = editor.is_enabled(&model.id);
        let is_1m = editor.one_m.contains(&model.id);
        let alias = profile
            .aliases
            .iter()
            .find(|(_, id)| canonical_model_id(id) == model.id)
            .map(|(role, _)| role)
            .filter(|_| self.client_tab() == ClientTab::Claude);

        let lines = vec![
            Line::from(format!(
                " Output cap: {} · Context: {}",
                model
                    .max_output_tokens
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "unset".into()),
                model
                    .context_window
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "unset".into())
            )),
            Line::from(vec![
                Span::styled(" Model: ", Style::default().fg(FIELD_LABEL)),
                Span::styled(
                    model.label(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                if is_default {
                    Span::styled(
                        "  ◆ Default model",
                        Style::default()
                            .fg(DEFAULT_MODEL)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::raw("")
                },
            ]),
            Line::from(vec![
                Span::styled(" ID: ", Style::default().fg(FIELD_LABEL)),
                Span::styled(
                    &effective_id,
                    Style::default().fg(ROUTE).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Status: ", Style::default().fg(FIELD_LABEL)),
                Span::styled(
                    if self.pi_enabled {
                        "● Configured"
                    } else if is_enabled {
                        "● Enabled"
                    } else {
                        "○ Disabled"
                    },
                    Style::default()
                        .fg(if is_enabled { ENABLED } else { MUTED })
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Context: ", Style::default().fg(FIELD_LABEL)),
                Span::styled(
                    if is_1m { "1M" } else { "Standard" },
                    Style::default().fg(if is_1m { DEFAULT_MODEL } else { MUTED }),
                ),
            ]),
            Line::from(vec![if let Some(alias) = alias {
                Span::styled(format!("   Role: {alias}"), Style::default().fg(ROUTE))
            } else {
                Span::raw("")
            }]),
        ];
        let controls = showcase_controls(area, self.pi_enabled);
        let controls_top = controls
            .iter()
            .map(|(_, rect)| rect.y)
            .min()
            .unwrap_or(inner.y.saturating_add(inner.height));
        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            Rect::new(
                inner.x,
                inner.y,
                inner.width,
                controls_top.saturating_sub(inner.y),
            ),
        );
        for (control, rect) in controls {
            let (label, style) = match control {
                ShowcaseControl::Toggle => (
                    if is_enabled {
                        "[● Disable (Space)]"
                    } else {
                        "[○ Enable (Space)]"
                    },
                    Style::default()
                        .fg(if is_enabled { ROUTE } else { MUTED })
                        .add_modifier(if is_enabled {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                ShowcaseControl::Default => (
                    if self.pi_enabled {
                        "[p Set default]"
                    } else {
                        "[d Set default]"
                    },
                    button_style(false, false, false),
                ),
                ShowcaseControl::OneM => (
                    if is_1m {
                        "[● 1M (1)]"
                    } else {
                        "[○ 1M (1)]"
                    },
                    Style::default()
                        .fg(if is_1m { ROUTE } else { MUTED })
                        .add_modifier(if is_1m {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                ShowcaseControl::Test => ("[F5 Test model]", button_style(false, false, false)),
                ShowcaseControl::Delete => ("[Delete model]", button_style(false, false, true)),
            };
            frame.render_widget(
                Paragraph::new(label)
                    .alignment(Alignment::Center)
                    .style(style),
                rect,
            );
        }
    }

    pub(super) fn draw_provider_details(
        &self,
        frame: &mut ratatui::Frame,
        area: Rect,
        profile: &Profile,
        active: bool,
        selected: bool,
    ) {
        let title = if selected {
            " Provider · click again to edit "
        } else if active {
            " Provider · click twice / E edit "
        } else {
            " Provider connection "
        };
        frame.render_widget(panel(title, active), area);
        let inner = panel_inner(area);
        let button_rows = 1;
        let content = Rect::new(
            inner.x,
            inner.y + button_rows,
            inner.width,
            inner.height.saturating_sub(button_rows),
        );
        let pi_proxy = self
            .pi_enabled
            .then(|| {
                self.selected_profile_id().and_then(|id| {
                    crate::pi::native::proxy_endpoint(&self.pi_home, &id)
                        .ok()
                        .flatten()
                })
            })
            .flatten();
        let api_format = if self.pi_enabled {
            format!(
                "{} · {}",
                profile.api_format.label(),
                if pi_proxy.is_some() {
                    "CCSW proxy"
                } else {
                    "direct API"
                }
            )
        } else if profile.api_format.is_openai() {
            let proxy = self
                .proxy_status
                .as_ref()
                .map(|status| {
                    if status.running {
                        format!("proxy ● {}", status.listen)
                    } else {
                        "proxy ○ stopped".into()
                    }
                })
                .unwrap_or_else(|| "proxy ?".into());
            format!("{} · {proxy}", profile.api_format.label())
        } else {
            profile.api_format.label().into()
        };
        let provider_identity = detail(
            "Provider",
            if self.pi_enabled {
                "● configured"
            } else if profile.enabled {
                "● enabled"
            } else {
                "○ disabled"
            },
        );
        let provider_identity = if selected {
            provider_identity.style(
                Style::default()
                    .bg(theme::PROVIDER_SELECTION)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            provider_identity
        };
        let mut lines = vec![
            provider_identity,
            detail("Calls", &self.provider_usage_label(false)),
            detail("Tokens", &self.provider_usage_label(true)),
            detail("API format", &api_format),
            detail("Endpoint", &profile.base_url),
            detail("Credential", &profile.credential.masked()),
            detail_value("Default", &profile.default_model, DEFAULT_MODEL),
            detail_value(
                "Models",
                &if self.pi_enabled {
                    format!("{} configured", profile.models.len())
                } else {
                    format!(
                        "{} enabled / {} available",
                        self.models().len(),
                        self.catalog_models().len()
                    )
                },
                ENABLED,
            ),
            detail(
                match self.config_tab() {
                    ClientTab::Claude => "Claude /model",
                    ClientTab::Codex => "Codex models",
                    ClientTab::Pi => "Pi /model",
                    ClientTab::Grok => "Grok /model",
                    ClientTab::Usage | ClientTab::Settings => unreachable!(),
                },
                &format!(
                    "{} models across {} providers",
                    self.all_enabled_model_count(),
                    self.config.profiles.len()
                ),
            ),
            Line::raw(""),
        ];
        if self.pi_enabled {
            lines.insert(
                5,
                detail("Pi proxy API", pi_proxy.as_deref().unwrap_or("P to enable")),
            );
        }
        if self.client_tab() == ClientTab::Claude {
            for (role, model) in profile.aliases.iter() {
                lines.push(detail(&format!("{role} alias"), model));
            }
            if let Some(model) = &profile.subagent_model {
                lines.push(detail("subagent", model));
            }
            if !profile.fallback_models.is_empty() {
                lines.push(detail("fallback", &profile.fallback_models.join(" → ")));
            }
        }
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            match self.config_tab() {
                ClientTab::Claude => "Sync changes, then run Claude from your terminal.",
                ClientTab::Codex => "Sync with p, restart to load models, then switch with /model.",
                ClientTab::Pi => "Changes save directly · P toggles proxy API · open /model in Pi.",
                ClientTab::Grok => {
                    "Press p to connect; saved changes sync automatically. Restart Grok."
                }
                ClientTab::Usage | ClientTab::Settings => unreachable!(),
            },
            Style::default().fg(MUTED),
        ));
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), content);
        draw_detail_controls(frame, area, self.theme);
    }

    pub(super) fn draw_details(&self, frame: &mut ratatui::Frame, area: Rect, active: bool) {
        if self.home_grok_oauth_selected() {
            self.draw_grok_accounts_embedded(frame, area);
            return;
        }
        if self.home_account_selected() {
            self.draw_codex_accounts_embedded(frame, area);
            return;
        }
        let Some(profile) = self.selected_profile() else {
            let title = if active {
                " Provider details · Esc back "
            } else {
                " Provider details "
            };
            frame.render_widget(panel(title, active), area);
            let inner = panel_inner(area);
            frame.render_widget(
                Paragraph::new("Add a provider for this client to get started.")
                    .style(Style::default().fg(MUTED))
                    .wrap(Wrap { trim: true }),
                inner,
            );
            return;
        };

        if self.view_mode == ViewMode::Provider {
            let fallback;
            let editor = match &self.provider_editor {
                Some(e) => e,
                None => {
                    fallback = self.create_route_editor();
                    match &fallback {
                        Some(e) => e,
                        None => return,
                    }
                }
            };

            let (showcase_card, provider_card) = provider_detail_cards(area);
            self.draw_showcase(frame, showcase_card, editor, profile);
            self.draw_provider_details(
                frame,
                provider_card,
                profile,
                active,
                self.provider_card_selected,
            );
            return;
        }

        self.draw_provider_details(frame, area, profile, active, false);
    }

    pub(super) fn draw_status(&self, frame: &mut ratatui::Frame, area: Rect) {
        let color = if self.status_error { ERROR } else { MUTED };
        let sync_color = match self.background.status {
            sync::Status::Synced => CONNECTED,
            sync::Status::Failed | sync::Status::Paused => ERROR,
            sync::Status::Pending | sync::Status::Syncing => WARNING,
            sync::Status::NotConnected => MUTED,
        };
        let footer = Line::from(vec![
            Span::styled(
                format!(
                    " {} · ",
                    if self.pi_enabled {
                        "Pi · P proxy API"
                    } else if self.grok_enabled {
                        "Grok · o OAuth · p connect · i import"
                    } else if self.codex_ui.enabled {
                        "Codex · /model switches loaded models"
                    } else {
                        self.background.status.label()
                    }
                ),
                Style::default().fg(sync_color),
            ),
            Span::styled(
                format!(" {} ", if self.status_error { "!" } else { "·" }),
                Style::default().fg(color),
            ),
            Span::styled(&self.status, Style::default().fg(color)),
        ]);
        frame.render_widget(Paragraph::new(footer), area);
    }

    pub(super) fn footer_control_name(
        &self,
        control: FooterControl,
    ) -> (&'static str, &'static str) {
        match control {
            FooterControl::AddProfile => (
                "Add Provider",
                if self.view_mode == ViewMode::Home || self.focus == Focus::Profiles {
                    "a"
                } else {
                    ""
                },
            ),
            FooterControl::AddModel => (
                "Add Model",
                if self.focus != Focus::Profiles {
                    "a"
                } else {
                    ""
                },
            ),
            FooterControl::DeleteProfile => ("Delete", "x"),
            FooterControl::Back => (if self.is_root_layer() { "Quit" } else { "Back" }, "Esc/q"),
            FooterControl::Models => ("Models", "h"),
            FooterControl::Details => ("Details", "l"),
            FooterControl::Sync => (
                if self.pi_enabled {
                    "Set default"
                } else if self.codex_ui.enabled || self.grok_enabled {
                    "Apply"
                } else {
                    "Sync"
                },
                "p",
            ),
            FooterControl::Proxy => (
                if self.pi_enabled {
                    "Proxy API"
                } else {
                    "Proxy"
                },
                "P",
            ),
            FooterControl::Disconnect => ("Disconnect", "D"),
            FooterControl::Settings => ("Settings", "F4"),
            FooterControl::Help => ("Help", "?"),
            FooterControl::Quit => ("Quit", "q"),
        }
    }

    pub(super) fn provider_action_text(&self, control: FooterControl) -> String {
        let (name, shortcut) = self.footer_control_name(control);
        let name = name;
        if shortcut.is_empty() {
            name.into()
        } else {
            format!("{name} [{shortcut}]")
        }
    }

    fn draw_provider_actions(&self, frame: &mut ratatui::Frame, page: &ProviderPageLayout) {
        for (control, rect) in &page.controls {
            let selected = match control {
                FooterControl::Models => self.focus == Focus::Models,
                FooterControl::Details => self.focus == Focus::Details,
                _ => false,
            };
            let color = if selected {
                ROUTE
            } else {
                match control {
                    FooterControl::Sync => CONNECTED,
                    FooterControl::DeleteProfile | FooterControl::Disconnect => ERROR,
                    FooterControl::Back | FooterControl::Settings | FooterControl::Help => {
                        FIELD_LABEL
                    }
                    FooterControl::Quit => Color::White,
                    _ => DATA_SECONDARY,
                }
            };
            frame.render_widget(
                Paragraph::new(toolbar::action_line(
                    &self.provider_action_text(*control),
                    color,
                    selected,
                    self.theme,
                ))
                .alignment(Alignment::Center),
                *rect,
            );
        }
    }

    #[cfg(test)]
    pub(super) fn footer_control_style(
        &self,
        control: FooterControl,
        _compact: bool,
        tiny: bool,
    ) -> (String, Style) {
        let selected = match control {
            FooterControl::Models => self.focus == Focus::Models,
            FooterControl::Details => self.focus == Focus::Details,
            _ => false,
        };
        let (name, shortcut) = self.footer_control_name(control);
        let label = if tiny && matches!(control, FooterControl::Sync) {
            name.to_owned()
        } else if tiny {
            format!("({shortcut})")
        } else {
            format!("{name} ({shortcut})")
        };
        let style = button_style(selected, false, control == FooterControl::DeleteProfile);
        (label, style)
    }

    pub(super) fn draw_modal(&self, frame: &mut ratatui::Frame, modal: &Modal) {
        let area = modal_area_for(modal, frame.area());
        frame.render_widget(Clear, area);
        match modal {
            Modal::Grok(dialog) => grok::draw_dialog(frame, area, dialog),
            Modal::Appearance(form) => theme::draw(
                frame,
                area,
                form,
                (!self.pi_enabled && !self.codex_ui.enabled).then_some(if self.grok_enabled {
                    "Grok settings (c)"
                } else {
                    "Claude settings (c)"
                }),
            ),
            Modal::Import(candidate) => {
                let mut lines = vec![
                    Line::styled(
                        "Existing Claude settings found",
                        Style::default().fg(ROUTE).add_modifier(Modifier::BOLD),
                    ),
                    Line::raw(""),
                ];
                lines.extend(candidate.summary().into_iter().map(Line::raw));
                lines.extend([
                    Line::raw(""),
                    Line::styled(
                        "The source file will not be changed.",
                        Style::default().fg(MUTED),
                    ),
                    Line::raw(""),
                    Line::styled("Enter/i import · s skip", Style::default().fg(ROUTE)),
                ]);
                frame.render_widget(
                    Paragraph::new(lines).block(panel(" Import preview ", true)),
                    area,
                );
                draw_modal_buttons(frame, area, &["Import", "Skip"]);
            }
            Modal::Preferences(form) => draw_preferences(frame, area, form),
            Modal::Profile(form) => {
                if let Some(selected) = form.template_selected {
                    frame.render_widget(panel(" New provider · choose a template ", true), area);
                    let inner = panel_inner(area);
                    let mut lines = vec![
                        Line::styled(
                            " j/k select · l/Enter use · h/Esc back",
                            Style::default().fg(MUTED),
                        ),
                        Line::raw(""),
                    ];
                    for index in 0..=PROVIDER_TEMPLATES.len() {
                        let label = if index == 0 {
                            "Custom · enter URL and settings manually".to_owned()
                        } else {
                            let template = &PROVIDER_TEMPLATES[index - 1];
                            format!("{} · {}", template.title, template.format)
                        };
                        lines.push(Line::styled(
                            format!("{} {label}", if selected == index { "▶" } else { " " }),
                            Style::default()
                                .fg(if selected == index {
                                    ROUTE
                                } else {
                                    Color::White
                                })
                                .add_modifier(if selected == index {
                                    Modifier::BOLD
                                } else {
                                    Modifier::empty()
                                }),
                        ));
                    }
                    frame.render_widget(
                        Paragraph::new(lines),
                        Rect::new(
                            inner.x,
                            inner.y,
                            inner.width,
                            inner.height.saturating_sub(2),
                        ),
                    );
                    if let Some(template) = selected
                        .checked_sub(1)
                        .and_then(|index| PROVIDER_TEMPLATES.get(index))
                    {
                        frame.render_widget(
                            Paragraph::new(format!(
                                "Name: {}\nURL: {}\nFill Key and model, then Ctrl+S to save.",
                                template.name, template.url
                            ))
                            .wrap(Wrap { trim: false }),
                            Rect::new(
                                inner.x,
                                inner.y + 7,
                                inner.width,
                                inner.height.saturating_sub(9),
                            ),
                        );
                    }
                    draw_modal_buttons(frame, area, &["Use template", "Custom", "Cancel"]);
                    return;
                }
                if let Some(picker) = &form.picker {
                    frame.render_widget(
                        panel(
                            &format!(
                                " {} · {} ",
                                form.fields[form.model_target_field()].label,
                                if form.picker_search {
                                    "Search · Esc: navigation"
                                } else {
                                    "click: fill · j/k: select · l: use · h: back · /: search"
                                }
                            ),
                            true,
                        ),
                        area,
                    );
                    let inner = panel_inner(area);
                    draw_api_models(
                        frame,
                        Rect::new(
                            inner.x,
                            inner.y,
                            inner.width,
                            inner.height.saturating_sub(2),
                        ),
                        picker,
                    );
                    draw_modal_buttons(frame, area, &["Refresh", "Select", "Back"]);
                    return;
                }
                draw_form(
                    frame,
                    area,
                    " Provider · F5: test · Alt+1: 1M ",
                    &form.fields,
                    form.selected,
                    true,
                );
                let inner = panel_inner(area);
                frame.render_widget(
                    Paragraph::new(
                        form.test_message
                            .as_ref()
                            .map_or("", |(message, _)| message.as_str()),
                    )
                    .style(Style::default().fg(
                        if form.test_message.as_ref().is_some_and(|(_, error)| *error) {
                            ERROR
                        } else {
                            CONNECTED
                        },
                    ))
                    .wrap(Wrap { trim: false }),
                    Rect::new(inner.x, inner.bottom().saturating_sub(4), inner.width, 2),
                );
                draw_modal_buttons(
                    frame,
                    area,
                    &[
                        if form.picker.is_some() {
                            "Refresh models"
                        } else if !form.fetched_models.is_empty()
                            && form.fetched_profile.as_deref()
                                == form.discovery_profile().ok().as_ref()
                        {
                            "Choose cached models"
                        } else if area.width < 64 {
                            "Fetch models"
                        } else {
                            "Fetch models (Alt+F)"
                        },
                        "Save",
                        "Cancel",
                    ],
                );
            }
            Modal::Model(form) => {
                draw_model_form(frame, area, form);
                draw_modal_buttons(
                    frame,
                    area,
                    &[
                        if area.width < 64 {
                            "Fetch API"
                        } else {
                            "Fetch API (Alt+F)"
                        },
                        "Save",
                        "Cancel",
                    ],
                );
            }
            Modal::Proxy(manager) => draw_proxy_manager(
                frame,
                area,
                manager,
                if self.grok_enabled {
                    "Grok"
                } else if self.pi_enabled {
                    "Pi"
                } else if self.codex_ui.enabled {
                    "Codex"
                } else {
                    "Claude"
                },
            ),
            Modal::DeleteProfile => {
                draw_confirmation(
                    frame,
                    area,
                    "Delete this profile? Cached model data will also be removed.",
                );
                draw_modal_buttons(frame, area, &["Delete", "Cancel"]);
            }
            Modal::DeleteModel => {
                draw_confirmation(
                    frame,
                    area,
                    "Delete this model from the provider? Its role assignments will also be removed.",
                );
                draw_modal_buttons(frame, area, &["Delete", "Cancel"]);
            }
            Modal::Help(help) => draw_help(frame, area, help, self.theme),
        }
        if self.status_error && area.height >= 5 && !matches!(modal, Modal::Profile(_)) {
            let rect = Rect::new(
                area.x + 1,
                area.y + area.height - 4,
                area.width.saturating_sub(2),
                2,
            );
            frame.render_widget(Clear, rect);
            frame.render_widget(
                Paragraph::new(format!("! {}", self.status))
                    .style(Style::default().fg(ERROR))
                    .wrap(Wrap { trim: false }),
                rect,
            );
        }
    }
}
