use super::*;

impl App {
    pub(super) fn draw(&mut self, frame: &mut ratatui::Frame) {
        let area = frame.area();
        self.screen = area;
        if area.width < 40 || area.height < 12 {
            frame.render_widget(
                Paragraph::new(
                    "CCSW · Terminal too small\nResize to at least 40 × 12\nq / Ctrl+C to quit",
                )
                .wrap(Wrap { trim: false }),
                area,
            );
            return;
        }
        if self.codex_ui.enabled && self.codex_ui.accounts {
            self.draw_codex_accounts(frame, area);
            return;
        }
        let rows = app_rows(area);
        self.draw_route(frame, rows[0]);
        let ui = ui_areas(area, self.focus, self.view_mode);
        if let Some(profiles) = ui.profiles {
            self.draw_profiles(frame, profiles);
        }
        if let Some(models) = ui.models {
            self.draw_models(frame, models);
        }
        if let Some(details) = ui.details {
            self.draw_details(frame, details, self.focus == Focus::Details);
        }
        self.draw_status(frame, ui.footer, area.width < 100);
        if let Some(modal) = &self.modal {
            self.draw_modal(frame, modal);
        }
        self.draw_codex_overlay(frame, area);
    }

    pub(super) fn draw_route(&self, frame: &mut ratatui::Frame, area: Rect) {
        self.draw_client_tabs(frame, area);
        let area = Rect::new(
            area.x,
            area.y.saturating_add(1),
            area.width,
            area.height.saturating_sub(1),
        );
        let profile = self
            .selected_profile()
            .map(|profile| profile.name.as_str())
            .unwrap_or("no profile");
        let model = self
            .selected_model()
            .map(|model| model.label().to_owned())
            .unwrap_or_else(|| "no model".into());
        let line = if self.pi_enabled {
            Line::from(vec![
                Span::styled(" CCSW · Pi API ", Style::default().fg(ROUTE)),
                Span::raw("F2 Claude · i Import · p Sync · s Status · D Disconnect"),
            ])
        } else if self.codex_ui.enabled {
            Line::from(vec![
                Span::styled(" CCSW · Codex API ", Style::default().fg(ROUTE)),
                Span::raw("F2 Pi · F3 Accounts · p Apply"),
            ])
        } else {
            match self.view_mode {
                ViewMode::Home => Line::from(vec![
                    Span::styled(
                        " CCSW ",
                        Style::default()
                            .fg(Color::Black)
                            .bg(ROUTE)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        "  Providers · F2 Codex",
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("  ·  {} providers", self.config.profiles.len()),
                        Style::default().fg(MUTED),
                    ),
                ]),
                ViewMode::Provider => Line::from(vec![
                    Span::styled(
                        " ‹ Back (Esc) ",
                        Style::default()
                            .fg(Color::Black)
                            .bg(ROUTE)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("  {profile}  "),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("→", Style::default().fg(ROUTE)),
                    Span::styled(
                        format!("  {model}  "),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                ]),
                ViewMode::AllEnabled => Line::from(vec![
                    Span::styled(
                        " ‹ Back (Esc) ",
                        Style::default()
                            .fg(Color::Black)
                            .bg(ROUTE)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        "  All Models",
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("  ·  {} models", self.all_enabled_model_count()),
                        Style::default().fg(CONNECTED),
                    ),
                ]),
            }
        };
        frame.render_widget(
            Paragraph::new(line)
                .wrap(Wrap { trim: true })
                .block(Block::default().borders(if area.height > 1 {
                    Borders::BOTTOM
                } else {
                    Borders::NONE
                })),
            area,
        );
    }

    pub(super) fn draw_profiles(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let ids = self.profile_ids();
        let is_home = self.view_mode == ViewMode::Home;
        let content_width = area.width.saturating_sub(2);
        let mut item_heights = Vec::with_capacity(ids.len().saturating_add(1));
        let mut items = Vec::with_capacity(ids.len().saturating_add(1));
        if is_home {
            let lines = all_enabled_lines(
                self.config
                    .profiles
                    .values()
                    .filter(|profile| profile.enabled)
                    .count(),
                self.all_enabled_model_count(),
                self.all_managed_models().len(),
                content_width,
            );
            item_heights.push(lines.len());
            items.push(ListItem::new(lines));
        }
        items.extend(ids.iter().map(|id| {
            let profile = &self.config.profiles[id];
            if is_home {
                let discovered = self
                    .cache
                    .profiles
                    .get(id)
                    .map(|cached| cached.models.as_slice())
                    .unwrap_or_default();
                let enabled_count = discovery::active_models(profile, discovered).len();
                let lines = home_profile_lines(id, profile, enabled_count, content_width);
                item_heights.push(lines.len());
                ListItem::new(lines)
            } else {
                item_heights.push(1);
                ListItem::new(Line::from(vec![
                    Span::styled("● ", Style::default().fg(CONNECTED)),
                    Span::raw(profile.name.clone()),
                    Span::styled(format!("  {id}"), Style::default().fg(MUTED)),
                ]))
            }
        }));
        let title = " Providers ";
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
                .highlight_style(
                    Style::default()
                        .fg(Color::White)
                        .bg(SELECTION)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol(" "),
            area,
            &mut state,
        );
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
            let models = self.all_managed_models();
            let enabled_count = models.iter().filter(|entry| entry.enabled).count();
            let items = models
                .iter()
                .map(|entry| {
                    ListItem::new(vec![
                        Line::from(vec![
                            Span::styled(
                                if entry.enabled { "● " } else { "○ " },
                                Style::default().fg(if entry.enabled { CONNECTED } else { MUTED }),
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
            let title = format!(" All models · {enabled_count}/{} enabled ", models.len());
            frame.render_stateful_widget(
                List::new(items)
                    .block(panel(&title, true))
                    .highlight_style(
                        Style::default()
                            .fg(Color::White)
                            .bg(SELECTION)
                            .add_modifier(Modifier::BOLD),
                    )
                    .highlight_symbol(" "),
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
                    Paragraph::new(
                        "No managed models.\nAdd models in a provider, or enable a provider from Home.",
                    )
                    .style(Style::default().fg(MUTED))
                    .alignment(Alignment::Center)
                    .block(panel(&title, true)),
                    area,
                );
            }
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
                let stats_text =
                    format!("({filtered_len}/{total_len} models · {enabled_len} enabled)");
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
                        Span::styled("▌", Style::default().fg(CONNECTED))
                    } else {
                        Span::raw("")
                    },
                    if search_inner.width >= 55 {
                        Span::styled(format!("  {stats_text}"), Style::default().fg(MUTED))
                    } else {
                        Span::raw("")
                    },
                ]);
                let add_btn = catalog_add_button_rect(search_area);
                let text_width = if let Some(btn) = add_btn {
                    search_inner
                        .width
                        .saturating_sub(btn.width.saturating_add(1))
                } else {
                    search_inner.width
                };
                let text_area = Rect::new(
                    search_inner.x,
                    search_inner.y,
                    text_width,
                    search_inner.height,
                );
                frame.render_widget(Paragraph::new(line), text_area);
                if let Some(btn_rect) = add_btn {
                    let label = if btn_rect.width >= 14 {
                        "[+ Add model (a)]"
                    } else {
                        "[+ a]"
                    };
                    frame.render_widget(
                        Paragraph::new(label).alignment(Alignment::Center).style(
                            Style::default()
                                .fg(Color::Black)
                                .bg(CONNECTED)
                                .add_modifier(Modifier::BOLD),
                        ),
                        btn_rect,
                    );
                }
            }

            let filtered = editor.filtered_indices();
            let list_title = " Models · ◆ default  ● enabled  ○ disabled ";
            let block = panel(
                list_title,
                self.focus == Focus::Models && !editor.search_active,
            );
            let inner = panel_inner(list_area);
            frame.render_widget(block, list_area);
            if inner.width == 0 || inner.height == 0 {
                return;
            }

            let manual_map = self
                .selected_profile()
                .map(|p| {
                    p.models
                        .iter()
                        .map(|m| (config::canonical_model_id(&m.id), ()))
                        .collect::<BTreeMap<_, _>>()
                })
                .unwrap_or_default();

            let items: Vec<ListItem> = filtered
                .iter()
                .map(|&idx| {
                    let model = &editor.catalog[idx];
                    let is_en = editor.is_enabled(&model.id);
                    let is_1m = editor.one_m.contains(&model.id);
                    let is_def = model.id == editor.default_model;
                    let is_man = manual_map.contains_key(model.id.as_str());
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
                        WARNING
                    } else if is_def && !is_en {
                        MUTED
                    } else if is_req || is_en {
                        CONNECTED
                    } else {
                        MUTED
                    };

                    let mut spans = vec![
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
                            Style::default().fg(CONNECTED),
                        ),
                    ];
                    if is_man && inner.width >= 60 {
                        spans.push(Span::styled(" [custom]", Style::default().fg(ROUTE)));
                    }
                    ListItem::new(Line::from(spans))
                })
                .collect();

            let offset = route_editor_offset(editor, inner.height);
            let mut state = ListState::default()
                .with_offset(offset)
                .with_selected((!items.is_empty()).then_some(editor.selected));
            frame.render_stateful_widget(
                List::new(items)
                    .highlight_style(
                        Style::default()
                            .fg(Color::White)
                            .bg(SELECTION)
                            .add_modifier(Modifier::BOLD),
                    )
                    .highlight_symbol(" "),
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
        let title = " Enabled models ";
        let mut state = ListState::default()
            .with_offset(self.model_offset)
            .with_selected((!items.is_empty()).then_some(self.model_idx));
        frame.render_stateful_widget(
            List::new(items)
                .block(panel(title, self.focus == Focus::Models))
                .highlight_style(
                    Style::default()
                        .fg(Color::White)
                        .bg(SELECTION)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol(" "),
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
        let is_manual = profile
            .models
            .iter()
            .any(|m| canonical_model_id(&m.id) == model.id);
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
                Span::styled(" Model: ", Style::default().fg(MUTED)),
                Span::styled(
                    model.label(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                if is_default {
                    Span::styled(
                        "  ◆ Default model",
                        Style::default().fg(WARNING).add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::raw("")
                },
            ]),
            Line::from(vec![
                Span::styled(" ID: ", Style::default().fg(MUTED)),
                Span::styled(
                    &effective_id,
                    Style::default().fg(ROUTE).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Status: ", Style::default().fg(MUTED)),
                Span::styled(
                    if is_enabled {
                        "● Enabled"
                    } else {
                        "○ Disabled"
                    },
                    Style::default()
                        .fg(if is_enabled { CONNECTED } else { MUTED })
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Context: ", Style::default().fg(MUTED)),
                Span::styled(
                    if is_1m { "1M" } else { "Standard" },
                    Style::default().fg(if is_1m { CONNECTED } else { MUTED }),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Source: ", Style::default().fg(MUTED)),
                Span::raw(if is_manual { "Custom" } else { "Gateway" }),
                if let Some(alias) = alias {
                    Span::styled(format!("   Role: {alias}"), Style::default().fg(ROUTE))
                } else {
                    Span::raw("")
                },
            ]),
        ];
        let controls = showcase_controls(area);
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
                        "[Space Disable]"
                    } else {
                        "[Space Enable]"
                    },
                    Style::default()
                        .fg(Color::Black)
                        .bg(ROUTE)
                        .add_modifier(Modifier::BOLD),
                ),
                ShowcaseControl::Default => (
                    "[d Set default]",
                    Style::default().fg(WARNING).add_modifier(Modifier::BOLD),
                ),
                ShowcaseControl::OneM => (
                    if is_1m {
                        "[1 Disable 1M]"
                    } else {
                        "[1 Enable 1M]"
                    },
                    Style::default().fg(ROUTE).add_modifier(Modifier::BOLD),
                ),
                ShowcaseControl::Delete => (
                    if is_manual {
                        "[x Delete]"
                    } else {
                        "[Gateway model]"
                    },
                    Style::default().fg(if is_manual { ERROR } else { MUTED }),
                ),
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
    ) {
        let title = if active {
            " Provider · r fetch · E edit "
        } else {
            " Provider connection "
        };
        frame.render_widget(panel(title, active), area);
        let inner = panel_inner(area);
        let button_rows = 1;
        let content = Rect::new(
            inner.x,
            inner.y,
            inner.width,
            inner.height.saturating_sub(button_rows),
        );
        let api_format = if self.pi_enabled {
            format!("{} · direct API", profile.api_format.label())
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
        let mut lines = vec![
            detail(
                "Provider",
                if profile.enabled {
                    "● enabled"
                } else {
                    "○ disabled"
                },
            ),
            detail("API format", &api_format),
            detail("Endpoint", &profile.base_url),
            detail("Credential", &profile.credential.masked()),
            detail("Default", &profile.default_model),
            detail(
                "Models",
                &format!(
                    "{} enabled / {} available",
                    self.models().len(),
                    self.catalog_models().len()
                ),
            ),
            detail(
                match self.client_tab() {
                    ClientTab::Claude => "Claude /model",
                    ClientTab::Codex => "Codex models",
                    ClientTab::Pi => "Pi /model",
                },
                &format!(
                    "{} models across {} providers",
                    self.all_enabled_model_count(),
                    self.config.profiles.len()
                ),
            ),
            Line::raw(""),
        ];
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
            match self.client_tab() {
                ClientTab::Claude => "Sync changes, then run Claude from your terminal.",
                ClientTab::Codex => "Apply changes, then start a new Codex session.",
                ClientTab::Pi => "Sync changes, then open /model in Pi.",
            },
            Style::default().fg(MUTED),
        ));
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), content);
        draw_detail_controls(frame, area);
    }

    pub(super) fn draw_details(&self, frame: &mut ratatui::Frame, area: Rect, active: bool) {
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
            self.draw_provider_details(frame, provider_card, profile, active);
            return;
        }

        self.draw_provider_details(frame, area, profile, active);
    }

    pub(super) fn draw_status(&self, frame: &mut ratatui::Frame, area: Rect, compact: bool) {
        for (control, rect) in footer_controls(area, compact, self.view_mode) {
            let (label, style) = self.footer_control_style(control, compact, area.width < 55);
            frame.render_widget(
                Paragraph::new(label)
                    .alignment(Alignment::Center)
                    .style(style),
                rect,
            );
        }
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
                        "Pi · direct API"
                    } else if self.codex_ui.enabled {
                        "Codex · restart after apply"
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
        if area.height > 1 {
            frame.render_widget(
                Paragraph::new(footer).wrap(Wrap { trim: true }),
                Rect {
                    y: area.y.saturating_add(1),
                    height: area.height.saturating_sub(1),
                    ..area
                },
            );
        }
    }

    pub(super) fn footer_control_style(
        &self,
        control: FooterControl,
        compact: bool,
        tiny: bool,
    ) -> (String, Style) {
        if self.pi_enabled && control == FooterControl::Sync {
            return (
                "Sync Pi".into(),
                Style::default().fg(Color::Black).bg(ROUTE),
            );
        }
        if self.codex_ui.enabled && control == FooterControl::Sync {
            return (
                if tiny {
                    "Apply".into()
                } else {
                    "Apply Codex".into()
                },
                Style::default().fg(Color::Black).bg(ROUTE),
            );
        }
        let selected = match control {
            FooterControl::Models => self.focus == Focus::Models,
            FooterControl::Details => self.focus == Focus::Details,
            _ => false,
        };
        let dot = if selected { '●' } else { '○' };
        let label = match (control, compact, tiny) {
            (FooterControl::Back, _, true) => "‹".into(),
            (FooterControl::Models, _, true) => format!("{dot} M"),
            (FooterControl::Details, _, true) => format!("{dot} D"),
            (FooterControl::AddProfile, _, true) => "+".into(),
            (FooterControl::Sync, _, true) => "⇄".into(),
            (FooterControl::Proxy, _, true) => "Px".into(),
            (FooterControl::Help, _, true) => "?".into(),
            (FooterControl::Quit, _, true) => "×".into(),
            (FooterControl::Back, true, false) => "‹ Back".into(),
            (FooterControl::Back, false, false) => "‹ Back (Esc)".into(),
            (FooterControl::Models, _, false) => format!("{dot} Models"),
            (FooterControl::Details, _, false) => format!("{dot} Details"),
            (FooterControl::AddProfile, true, false) => "+ Provider".into(),
            (FooterControl::AddProfile, false, false) => "+ Provider (n)".into(),
            (FooterControl::Sync, true, false) => "⇄ Sync".into(),
            (FooterControl::Sync, false, false) => "⇄ Sync all".into(),
            (FooterControl::Proxy, true, false) => format!(
                "{} Px",
                if self
                    .proxy_status
                    .as_ref()
                    .is_some_and(|status| status.running)
                {
                    '●'
                } else {
                    '○'
                }
            ),
            (FooterControl::Proxy, false, false) => format!(
                "{} Proxy",
                if self
                    .proxy_status
                    .as_ref()
                    .is_some_and(|status| status.running)
                {
                    '●'
                } else {
                    '○'
                }
            ),
            (FooterControl::Help, true, false) => "?".into(),
            (FooterControl::Help, false, false) => "Help".into(),
            (FooterControl::Quit, true, false) => "×".into(),
            (FooterControl::Quit, false, false) => "Quit".into(),
        };
        let style = if control == FooterControl::Back {
            Style::default()
                .fg(Color::Black)
                .bg(ROUTE)
                .add_modifier(Modifier::BOLD)
        } else if matches!(control, FooterControl::AddProfile | FooterControl::Sync) {
            Style::default()
                .fg(Color::Black)
                .bg(if control == FooterControl::AddProfile {
                    CONNECTED
                } else {
                    ROUTE
                })
                .add_modifier(Modifier::BOLD)
        } else if selected {
            Style::default()
                .fg(Color::Black)
                .bg(CONNECTED)
                .add_modifier(Modifier::BOLD)
        } else if control == FooterControl::Proxy {
            Style::default().fg(
                if self
                    .proxy_status
                    .as_ref()
                    .is_some_and(|status| status.running)
                {
                    CONNECTED
                } else {
                    WARNING
                },
            )
        } else {
            Style::default().fg(MUTED)
        };
        (label, style)
    }

    pub(super) fn draw_modal(&self, frame: &mut ratatui::Frame, modal: &Modal) {
        let area = modal_area_for(modal, frame.area());
        frame.render_widget(Clear, area);
        match modal {
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
                    Line::styled("Enter/i import · s skip", Style::default().fg(WARNING)),
                ]);
                frame.render_widget(
                    Paragraph::new(lines).block(panel(" Import preview ", true)),
                    area,
                );
                draw_modal_buttons(frame, area, &["Import", "Skip"]);
            }
            Modal::Profile(form) => {
                draw_form(
                    frame,
                    area,
                    " Profile · click a field to edit ",
                    &form.fields,
                    form.selected,
                );
                draw_modal_buttons(frame, area, &["Save", "Cancel"]);
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
                            "Fetch API (Ctrl+F)"
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
                if self.codex_ui.enabled {
                    "Codex"
                } else {
                    "Claude Code"
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
                    "Delete this manual model? Gateway models cannot be deleted here.",
                );
                draw_modal_buttons(frame, area, &["Delete", "Cancel"]);
            }
            Modal::Help(help) => draw_help(frame, area, help),
        }
        if self.status_error && area.height >= 5 {
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
