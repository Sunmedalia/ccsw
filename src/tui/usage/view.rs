use super::*;
use ratatui::widgets::{Cell, Row, Table, TableState};

// The selected range is a token ledger: a large total, its input/output
// composition, then the detailed tables beneath it.
struct Areas {
    clients: Rect,
    date: Rect,
    range: Rect,
    summary: Rect,
    sections: Rect,
    body: Rect,
    footer: Rect,
}
fn areas(area: Rect) -> Areas {
    let mut inner = panel_inner(area);
    if inner.width >= 60 {
        inner.x += 1;
        inner.width = inner.width.saturating_sub(2);
    }
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(if inner.height >= 17 { 4 } else { 1 }),
        Constraint::Length(1),
        Constraint::Min(2),
        Constraint::Length(1),
    ])
    .split(inner);
    Areas {
        clients: rows[0],
        date: rows[1],
        range: rows[2],
        summary: rows[3],
        sections: rows[4],
        body: rows[5],
        footer: rows[6],
    }
}

#[derive(Clone, Copy)]
enum Action {
    Client(usize),
    Previous,
    Range(usize),
    DateLabel,
    Metric(bool),
    Next,
    Today,
    All,
    Section(usize),
    Refresh,
    Close,
    SessionSort,
}
fn buttons(area: Rect, labels: &[(&str, Action)]) -> Vec<(String, Action, Rect)> {
    let padding = u16::from(area.width >= 70);
    let gap = if area.width >= 70 { 2 } else { 1 };
    let mut x = area.x;
    labels
        .iter()
        .filter_map(|(label, action)| {
            if label.is_empty() {
                return None;
            }
            let width = UnicodeWidthStr::width(*label) as u16 + padding * 2;
            if x + width > area.right() || area.height == 0 {
                return None;
            }
            let rect = Rect::new(x, area.y, width, 1);
            x += width + gap;
            Some((label.to_string(), *action, rect))
        })
        .collect()
}
fn controls(a: &Areas, page: &UsagePage) -> Vec<(String, Action, Rect)> {
    let mut result = buttons(
        a.clients,
        &[
            ("All", Action::Client(0)),
            ("Claude", Action::Client(1)),
            ("Codex", Action::Client(2)),
            ("Pi", Action::Client(3)),
            (
                if a.clients.width >= 70 {
                    "Refresh r"
                } else {
                    "r"
                },
                Action::Refresh,
            ),
            ("Back", Action::Close),
        ],
    );
    result.extend(buttons(
        a.date,
        &[
            ("‹", Action::Previous),
            (&page.day, Action::DateLabel),
            ("›", Action::Next),
            ("Today", Action::Today),
            (
                if page.provider.is_some() {
                    "All ×"
                } else {
                    ""
                },
                Action::All,
            ),
        ],
    ));
    result.extend(buttons(
        a.range,
        &[
            ("1 day", Action::Range(0)),
            ("1 week", Action::Range(1)),
            ("1 month", Action::Range(2)),
            ("All time", Action::Range(3)),
        ],
    ));
    let narrow = a.sections.width < 94;
    let tiny = a.sections.width < 50;
    result.extend(buttons(
        a.sections,
        &[
            (
                if tiny {
                    "Pr1"
                } else if narrow {
                    "Prov1"
                } else {
                    "Providers 1"
                },
                Action::Section(0),
            ),
            (
                if tiny {
                    "Dy2"
                } else if narrow {
                    "Days2"
                } else {
                    "History 2"
                },
                Action::Section(1),
            ),
            (
                if tiny {
                    "In3"
                } else if narrow {
                    "Info3"
                } else {
                    "Details 3"
                },
                Action::Section(2),
            ),
            (
                if narrow { "Models4" } else { "Models 4" },
                Action::Section(3),
            ),
            (
                if narrow { "Chart5" } else { "Chart 5" },
                Action::Section(4),
            ),
            (
                if tiny {
                    "Ses6"
                } else if narrow {
                    "Sessions6"
                } else {
                    "Sessions 6"
                },
                Action::Section(5),
            ),
        ],
    ));
    if page.section == 4 {
        result.extend(buttons(
            a.body,
            &[
                ("Calls c", Action::Metric(false)),
                ("Tokens v", Action::Metric(true)),
            ],
        ));
    }
    if page.section == 5 {
        result.extend(buttons(
            a.body,
            &[(
                if page.session_sort_tokens {
                    "Sort: tokens s"
                } else {
                    "Sort: recent s"
                },
                Action::SessionSort,
            )],
        ));
    }

    result
}

struct ProviderRow {
    client: String,
    id: String,
    name: String,
    daily: Totals,
    total: Totals,
}

struct ModelRow {
    client: String,
    provider: String,
    model: String,
    daily: Totals,
    total: Totals,
}

fn number(n: i64) -> String {
    let raw = n.to_string();
    let mut out = String::new();
    for (index, ch) in raw.chars().enumerate() {
        if index > 0 && (raw.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}
fn compact(n: i64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1_000_000_000.)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.)
    } else if n >= 10_000 {
        format!("{:.1}K", n as f64 / 1_000.)
    } else {
        number(n)
    }
}
fn tokens(t: &Totals) -> String {
    if t.calls > 0 && t.unknown == t.calls {
        return "unknown".into();
    }
    format!(
        "{}{}",
        compact(t.input + t.output),
        if t.unknown > 0 { " + ?" } else { "" }
    )
}
fn numeric(value: String, color: Color) -> Cell<'static> {
    Cell::from(Line::styled(value, Style::default().fg(color)).alignment(Alignment::Right))
}
fn heading(label: &str) -> Cell<'static> {
    Cell::from(
        Line::styled(label.to_owned(), Style::default().fg(MUTED)).alignment(Alignment::Right),
    )
}

impl App {
    fn session_rows(&self, page: &UsagePage) -> Vec<&crate::sessions::Session> {
        let zone = chrono::FixedOffset::east_opt(self.usage.snapshot.offset)
            .unwrap_or_else(|| chrono::FixedOffset::east_opt(0).unwrap());
        let mut rows: Vec<_> = self
            .usage
            .sessions
            .rows
            .iter()
            .filter(|s| {
                page.client().is_none_or(|c| c == s.client)
                    && (page.range == 3
                        || chrono::DateTime::from_timestamp(s.updated, 0).is_some_and(|t| {
                            page.includes(&t.with_timezone(&zone).format("%Y-%m-%d").to_string())
                        }))
            })
            .collect();
        rows.sort_by(|a, b| {
            if page.session_sort_tokens {
                b.tokens
                    .known
                    .cmp(&a.tokens.known)
                    .then_with(|| b.tokens.total().cmp(&a.tokens.total()))
                    .then_with(|| b.updated.cmp(&a.updated))
                    .then_with(|| a.id.cmp(&b.id))
            } else {
                b.updated.cmp(&a.updated).then_with(|| a.id.cmp(&b.id))
            }
        });
        rows
    }

    fn draw_sessions(&self, frame: &mut ratatui::Frame, a: &Areas, page: &UsagePage) {
        let sessions = self.session_rows(page);
        let mut summary = vec![Line::styled(
            if a.summary.width < 52 {
                format!("{} sessions · lifetime tokens", sessions.len())
            } else {
                format!("{} sessions · local logs · lifetime tokens", sessions.len())
            },
            Style::default().fg(ROUTE),
        )];
        if a.summary.height >= 3 {
            summary.push(Line::raw(
                "Date filters last activity; input includes cache. Independent of proxy ledger.",
            ));
            summary.push(Line::styled(
                "Child sessions listed separately; forks may include inherited usage (*).",
                Style::default().fg(MUTED),
            ));
        }
        frame.render_widget(Paragraph::new(summary), a.summary);
        let body = Rect {
            y: a.body.y.saturating_add(1),
            height: a.body.height.saturating_sub(1),
            ..a.body
        };
        let detail_height = if body.height >= 10 {
            5
        } else if body.height >= 5 {
            2
        } else {
            0
        };
        let table_area = Rect {
            height: body.height.saturating_sub(detail_height),
            ..body
        };
        let wide = body.width >= 90;
        let medium = body.width >= 60;
        let mut headers = vec![
            Cell::from("Session / project"),
            Cell::from("Client"),
            heading("Tokens"),
        ];
        let mut widths = vec![
            Constraint::Min(10),
            Constraint::Length(6),
            Constraint::Length(11),
        ];
        if medium {
            headers.extend([heading("Input"), heading("Output")]);
            widths.extend([Constraint::Length(10), Constraint::Length(10)]);
        }
        if wide {
            headers.extend([heading("Cache read"), Cell::from("Last active")]);
            widths.extend([Constraint::Length(10), Constraint::Length(16)]);
        }
        let zone = chrono::FixedOffset::east_opt(self.usage.snapshot.offset)
            .unwrap_or_else(|| chrono::FixedOffset::east_opt(0).unwrap());
        let rows = sessions
            .iter()
            .map(|s| {
                let project = std::path::Path::new(&s.project)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                let id: String = s.id.chars().take(8).collect();
                let value = |n| {
                    if s.tokens.known {
                        format!("{}{}", compact(n), if s.incomplete { "?" } else { "" })
                    } else {
                        "?".into()
                    }
                };
                let mut cells = vec![
                    Cell::from(format!(
                        "{id} {project}{}{}",
                        if s.child { " [child]" } else { "" },
                        if s.fork { " *" } else { "" }
                    )),
                    Cell::from(s.client),
                    numeric(value(s.tokens.total()), ROUTE),
                ];
                if medium {
                    cells.extend([
                        numeric(value(s.tokens.input), Color::White),
                        numeric(value(s.tokens.output), Color::White),
                    ]);
                }
                if wide {
                    cells.push(numeric(value(s.tokens.read), MUTED));
                    cells.push(Cell::from(
                        chrono::DateTime::from_timestamp(s.updated, 0)
                            .map(|t| t.with_timezone(&zone).format("%m-%d %H:%M").to_string())
                            .unwrap_or_default(),
                    ));
                }
                Row::new(cells)
            })
            .collect();
        draw_table(
            frame,
            table_area,
            page,
            rows,
            headers,
            widths,
            if self.usage.sessions_updated.is_none() {
                "Loading local sessions…"
            } else {
                "No local sessions in this range. Try All time (y) or another client."
            },
        );
        if detail_height > 0
            && let Some(s) = sessions.get(page.scroll.min(page.limit.get()) as usize)
        {
            let count = |n| {
                if s.tokens.known {
                    number(n)
                } else {
                    "unknown".into()
                }
            };
            let lines = vec![
                Line::styled(
                    format!(
                        "{} · {}{}",
                        s.client,
                        s.id,
                        if s.fork {
                            " · fork: may include inherited tokens"
                        } else {
                            ""
                        }
                    ),
                    Style::default().fg(ROUTE),
                ),
                Line::raw(format!(
                    "Input {} · Output {} · Total {}{}",
                    count(s.tokens.input),
                    count(s.tokens.output),
                    count(s.tokens.total()),
                    if s.incomplete { " + ? (partial)" } else { "" }
                )),
                Line::raw(if s.tokens.cache_known {
                    format!(
                        "Cache reuse {} · read {} · write {} (in input)",
                        s.tokens
                            .cache_reuse_percent()
                            .map_or("—".into(), |rate| format!("{rate:.1}%")),
                        count(s.tokens.read),
                        count(s.tokens.write)
                    )
                } else {
                    "Cache usage unavailable in local log".into()
                }),
                Line::raw(format!("Project: {}", s.project)),
                Line::styled(
                    format!(
                        "Models: {}",
                        s.models.iter().cloned().collect::<Vec<_>>().join(", ")
                    ),
                    Style::default().fg(MUTED),
                ),
            ];
            frame.render_widget(
                Paragraph::new(lines),
                Rect::new(body.x, table_area.bottom(), body.width, detail_height),
            );
        }
        frame.render_widget(
            Paragraph::new(if self.usage.sessions.warnings > 0 {
                "Some logs unavailable · cached/partial data · r retry"
            } else {
                "↑↓ select · s sort · 6 Sessions · r refresh · Esc back"
            })
            .style(Style::default().fg(if self.usage.sessions.warnings > 0 {
                WARNING
            } else {
                MUTED
            })),
            a.footer,
        );
    }

    fn usage_models(&self, page: &UsagePage) -> Vec<ModelRow> {
        let mut models: BTreeMap<(String, String, String), ModelRow> = BTreeMap::new();
        for row in &self.usage.snapshot.rows {
            if row.kind != "generation"
                || page.client().is_some_and(|c| c != row.client)
                || page.provider.as_deref().is_some_and(|p| p != row.provider)
            {
                continue;
            }
            let model = models
                .entry((row.client.clone(), row.provider.clone(), row.model.clone()))
                .or_insert_with(|| ModelRow {
                    client: row.client.clone(),
                    provider: row.provider.clone(),
                    model: row.model.clone(),
                    daily: Totals::default(),
                    total: Totals::default(),
                });
            model.total.add(&row.totals);
            if page.includes(&row.day) {
                model.daily.add(&row.totals);
            }
        }
        let mut rows: Vec<_> = models.into_values().collect();
        rows.sort_by(|a, b| {
            b.daily
                .calls
                .cmp(&a.daily.calls)
                .then_with(|| b.total.calls.cmp(&a.total.calls))
                .then_with(|| {
                    (&a.client, &a.provider, &a.model).cmp(&(&b.client, &b.provider, &b.model))
                })
        });
        rows
    }

    fn draw_usage_models(&self, frame: &mut ratatui::Frame, area: Rect, page: &UsagePage) {
        let models = self.usage_models(page);
        // Keep the selected model's full identity below the table, including on narrow terminals.
        let footer_height = if area.height >= 7 {
            if area.width >= 66 { 2 } else { 3 }
        } else if area.height >= 4 {
            2
        } else {
            0
        };
        let table_area = Rect {
            height: area.height.saturating_sub(footer_height),
            ..area
        };
        let medium = area.width >= 68;
        let wide = area.width >= 100;
        let mut headers = vec![Cell::from("Model"), heading("Tokens"), heading("Calls")];
        let mut widths = vec![
            Constraint::Min(12),
            Constraint::Length(12),
            Constraint::Length(8),
        ];
        if medium {
            headers.extend([heading("All tokens"), heading("All calls")]);
            widths.extend([Constraint::Length(12), Constraint::Length(9)]);
        }
        if wide {
            headers.push(Cell::from("Provider / client"));
            widths.push(Constraint::Length(23));
        }
        let rows = models
            .iter()
            .map(|model| {
                let mut cells = vec![
                    Cell::from(if model.model.is_empty() {
                        "(unspecified)".into()
                    } else {
                        model.model.clone()
                    }),
                    numeric(tokens(&model.daily), ROUTE),
                    numeric(number(model.daily.calls), Color::White),
                ];
                if medium {
                    cells.extend([
                        numeric(tokens(&model.total), Color::White),
                        numeric(number(model.total.calls), MUTED),
                    ]);
                }
                if wide {
                    cells.push(Cell::from(format!("{} / {}", model.provider, model.client)));
                }
                Row::new(cells)
            })
            .collect();
        draw_table(
            frame,
            table_area,
            page,
            rows,
            headers,
            widths,
            "No model calls yet.",
        );
        if footer_height > 0
            && let Some(model) = models.get(page.scroll.min(page.limit.get()) as usize)
        {
            let mut lines = vec![Line::styled(
                format!("{} · {} / {}", model.model, model.provider, model.client),
                Style::default().fg(ROUTE),
            )];
            if !medium {
                lines.push(Line::raw(format!(
                    "Tokens {} range / {} total",
                    model.daily.tokens_label(),
                    model.total.tokens_label()
                )));
            }
            if model.total.failed > 0 {
                lines.push(Line::styled(
                    format!(
                        "Failed {} range / {} total",
                        model.daily.failed, model.total.failed
                    ),
                    Style::default().fg(ERROR),
                ));
            }
            frame.render_widget(
                Paragraph::new(lines).wrap(Wrap { trim: false }),
                Rect::new(area.x, table_area.bottom(), area.width, footer_height),
            );
        }
    }

    fn usage_providers(&self, page: &UsagePage) -> Vec<ProviderRow> {
        let mut providers = BTreeMap::new();
        for row in &self.usage.snapshot.rows {
            if page.client().is_none_or(|c| c == row.client)
                && page.provider.as_deref().is_none_or(|p| p == row.provider)
            {
                providers.insert((row.client.clone(), row.provider.clone()), row.name.clone());
            }
        }
        let current = if self.codex_ui.enabled {
            "Codex"
        } else {
            "Claude"
        };
        if !self.pi_enabled && page.client().is_none_or(|c| c == current) {
            for (id, profile) in &self.config.profiles {
                if page.provider.as_deref().is_none_or(|p| p == id) {
                    providers.insert((current.into(), id.clone()), profile.name.clone());
                }
            }
        }
        let mut rows: Vec<_> = providers
            .into_iter()
            .map(|((client, id), name)| ProviderRow {
                daily: page.range_total(
                    &self.usage.snapshot,
                    Some(&client),
                    Some(&id),
                    "generation",
                ),
                total: self
                    .usage
                    .snapshot
                    .total(Some(&client), Some(&id), None, "generation"),
                client,
                id,
                name,
            })
            .collect();
        rows.sort_by(|a, b| {
            b.daily
                .calls
                .cmp(&a.daily.calls)
                .then_with(|| a.client.cmp(&b.client))
                .then_with(|| a.id.cmp(&b.id))
        });
        rows
    }

    fn usage_history(&self, page: &UsagePage) -> Vec<(String, Totals)> {
        let mut days: BTreeMap<String, Totals> = BTreeMap::new();
        for row in &self.usage.snapshot.rows {
            if row.kind == "generation"
                && page.includes(&row.day)
                && page.client().is_none_or(|c| c == row.client)
                && page.provider.as_deref().is_none_or(|p| p == row.provider)
            {
                days.entry(row.day.clone()).or_default().add(&row.totals);
            }
        }
        days.into_iter().rev().collect()
    }

    pub(in crate::tui) fn usage_enter(&self, page: &mut UsagePage, key: KeyEvent) {
        if key.code != KeyCode::Enter {
            return;
        }
        if page.section == 0 {
            if let Some(row) = self.usage_providers(page).get(page.scroll as usize) {
                page.provider = Some(row.id.clone());
                page.client = if row.client == "Codex" { 2 } else { 1 };
                page.section = 3;
                page.scroll = 0;
            }
        } else if page.section == 1
            && let Some((day, _)) = self.usage_history(page).get(page.scroll as usize)
        {
            page.day = day.clone();
            page.follow_today = page.day == self.usage.snapshot.today();
            page.section = 3;
            page.range = 0;
            page.scroll = 0;
        }
    }

    pub(in crate::tui) fn usage_mouse(&mut self, mouse: MouseEvent, screen: Rect) {
        let Some(mut page) = self.usage.page.take() else {
            return;
        };
        let area = page_area(screen);
        let a = areas(area);
        let mut close = false;
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                page.key(
                    KeyEvent::new(
                        if mouse.kind == MouseEventKind::ScrollUp {
                            KeyCode::Up
                        } else {
                            KeyCode::Down
                        },
                        KeyModifiers::NONE,
                    ),
                    &self.usage.snapshot,
                );
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some((_, action, _)) = controls(&a, &page)
                    .into_iter()
                    .find(|(_, _, rect)| contains(*rect, mouse.column, mouse.row))
                {
                    page.clicked = None;
                    match action {
                        Action::DateLabel => {}
                        Action::Range(range) => {
                            page.range = range;
                            page.scroll = 0;
                        }
                        Action::Metric(tokens) => page.chart_tokens = tokens,
                        Action::Client(client) => {
                            page.client = client;
                            page.provider = None;
                            page.scroll = 0;
                        }
                        Action::Section(section) => {
                            page.section = section;
                            if section == 5 {
                                page.provider = None;
                            }
                            page.scroll = 0;
                        }
                        Action::SessionSort => {
                            page.session_sort_tokens = !page.session_sort_tokens;
                            page.scroll = 0;
                        }
                        Action::All => {
                            page.provider = None;
                            page.scroll = 0;
                        }
                        Action::Refresh => {
                            self.usage.updated = None;
                            self.usage.sessions_updated = None;
                        }
                        Action::Close => {
                            close = page.key(
                                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                                &self.usage.snapshot,
                            )
                        }
                        other => {
                            let key = match other {
                                Action::Previous => KeyCode::Left,
                                Action::Next => KeyCode::Right,
                                _ => KeyCode::Char('t'),
                            };
                            page.key(KeyEvent::new(key, KeyModifiers::NONE), &self.usage.snapshot);
                        }
                    }
                } else if page.section != 2
                    && page.section != 4
                    && contains(page.table_area.get(), mouse.column, mouse.row)
                    && mouse.row > page.table_area.get().y
                {
                    let index =
                        page.offset.get() + usize::from(mouse.row - page.table_area.get().y - 1);
                    let count = if page.section == 5 {
                        self.session_rows(&page).len()
                    } else if page.section == 3 {
                        self.usage_models(&page).len()
                    } else if page.section == 0 {
                        self.usage_providers(&page).len()
                    } else {
                        self.usage_history(&page).len()
                    };
                    if index < count {
                        if page.clicked == Some((page.section, index)) {
                            self.usage_enter(
                                &mut page,
                                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                            );
                            page.clicked = None;
                        } else {
                            page.clicked = Some((page.section, index));
                            page.scroll = index.min(u16::MAX as usize) as u16;
                        }
                    }
                }
            }
            _ => {}
        }
        self.usage.active = !close;
        self.usage.page = Some(page);
    }

    pub(in crate::tui) fn draw_usage(
        &self,
        frame: &mut ratatui::Frame,
        area: Rect,
        page: &UsagePage,
    ) {
        let snapshot = &self.usage.snapshot;
        let zone = chrono::FixedOffset::east_opt(snapshot.offset)
            .unwrap_or_else(|| chrono::FixedOffset::east_opt(0).unwrap());

        let a = areas(area);
        for (label, action, rect) in controls(&a, page) {
            let active = match action {
                Action::Client(client) => page.client == client,
                Action::Section(section) => page.section == section,
                Action::Today => page.follow_today,
                Action::Range(range) => page.range == range,
                Action::Metric(tokens) => page.chart_tokens == tokens,
                _ => false,
            };
            frame.render_widget(
                Paragraph::new(label)
                    .alignment(Alignment::Center)
                    .style(button_style(
                        active,
                        (matches!(action, Action::Next) && page.day >= snapshot.today())
                            || (page.range == 3
                                && matches!(
                                    action,
                                    Action::Previous | Action::Next | Action::DateLabel
                                )),
                        false,
                    )),
                rect,
            );
        }
        let scope = page
            .provider
            .as_deref()
            .map(|id| {
                let client = if self.codex_ui.enabled {
                    "Codex"
                } else {
                    "Claude"
                };
                if !self.pi_enabled && page.client() == Some(client) {
                    self.config
                        .profiles
                        .get(id)
                        .map(|p| format!("{} · {id}", p.name))
                        .unwrap_or_else(|| id.into())
                } else {
                    id.into()
                }
            })
            .unwrap_or_else(|| "All providers".into());
        frame.render_widget(
            panel(
                &format!(
                    " Usage · {} ",
                    if page.section == 5 {
                        "Local sessions"
                    } else {
                        &scope
                    }
                ),
                true,
            ),
            area,
        );
        if page.section == 5 {
            self.draw_sessions(frame, &a, page);
            return;
        }
        if self.usage.query.is_some() && self.usage.updated.is_none() {
            frame.render_widget(
                Paragraph::new("Loading usage…").style(Style::default().fg(MUTED)),
                a.summary,
            );
            page.limit.set(0);
            page.table_area.set(Rect::default());
            return;
        }
        let daily = page.range_total(
            snapshot,
            page.client(),
            page.provider.as_deref(),
            "generation",
        );
        let total = snapshot.total(page.client(), page.provider.as_deref(), None, "generation");
        let tracked = page.client() != Some("Pi");
        draw_summary(
            frame,
            a.summary,
            &daily,
            &total,
            tracked,
            page.range_label(),
        );
        if !tracked {
            frame.render_widget(
                Paragraph::new(vec![Line::styled(
                    "Pi: not tracked",
                    Style::default().fg(WARNING),
                )])
                .wrap(Wrap { trim: false }),
                a.body,
            );
            page.limit.set(0);
        } else {
            match page.section {
                0 => self.draw_usage_providers(frame, a.body, page),
                1 => self.draw_usage_history(frame, a.body, page),
                3 => self.draw_usage_models(frame, a.body, page),
                4 => self.draw_usage_chart(frame, a.body, page),
                _ => self.draw_usage_details(frame, a.body, page, &daily, &total),
            }
        }
        let hint = if page.section == 0 || page.section == 1 {
            "↑↓ scroll · Enter models · Esc back"
        } else {
            "↑↓ scroll · Esc back"
        };
        let lines = if self.usage.error.is_some() {
            vec![Line::styled(
                "Usage unavailable · showing cached data · r retry",
                Style::default().fg(ERROR),
            )]
        } else {
            vec![Line::styled(
                if a.footer.width >= 70 {
                    format!(
                        "{hint} · UTC{zone}{}",
                        if total.unknown > 0 {
                            " · ? unknown"
                        } else {
                            ""
                        }
                    )
                } else {
                    hint.into()
                },
                Style::default().fg(MUTED),
            )]
        };
        frame.render_widget(Paragraph::new(lines), a.footer);
    }

    fn draw_usage_providers(&self, frame: &mut ratatui::Frame, area: Rect, page: &UsagePage) {
        let providers = self.usage_providers(page);
        let wide = area.width >= 90;
        let medium = area.width >= 64;
        let mut headers = vec![Cell::from("Provider"), heading("Tokens"), heading("Calls")];
        let mut widths = vec![
            Constraint::Min(10),
            Constraint::Length(12),
            Constraint::Length(8),
        ];
        if medium {
            headers.extend([heading("All tokens"), heading("All calls")]);
            widths.extend([Constraint::Length(12), Constraint::Length(9)]);
        }
        if wide {
            headers.push(heading("Failed"));
            headers.push(heading("Stopped"));
            widths.extend([Constraint::Length(7), Constraint::Length(7)]);
        }
        let rows: Vec<_> = providers
            .iter()
            .map(|p| {
                let label = if page.client().is_none() {
                    format!("{} · {}", p.name, p.client)
                } else {
                    p.name.clone()
                };
                let mut cells = vec![
                    Cell::from(label),
                    numeric(tokens(&p.daily), ROUTE),
                    numeric(number(p.daily.calls), Color::White),
                ];
                if medium {
                    cells.push(numeric(tokens(&p.total), Color::White));
                    cells.push(numeric(number(p.total.calls), MUTED));
                }
                if wide {
                    cells.push(numeric(
                        number(p.daily.failed),
                        if p.daily.failed > 0 { ERROR } else { MUTED },
                    ));
                    cells.push(numeric(
                        number(p.daily.interrupted),
                        if p.daily.interrupted > 0 {
                            WARNING
                        } else {
                            MUTED
                        },
                    ));
                }
                Row::new(cells)
            })
            .collect();
        draw_table(frame, area, page, rows, headers, widths, "No calls yet.");
    }

    fn draw_usage_history(&self, frame: &mut ratatui::Frame, area: Rect, page: &UsagePage) {
        let history = self.usage_history(page);
        let wide = area.width >= 80;
        let medium = area.width >= 56;
        let mut headers = vec![Cell::from("Date"), heading("Tokens"), heading("Calls")];
        let mut widths = vec![
            Constraint::Min(10),
            Constraint::Length(12),
            Constraint::Length(10),
        ];
        if medium {
            headers.extend([heading("Success"), heading("Failed")]);
            widths.extend([Constraint::Length(9), Constraint::Length(8)]);
        }
        if wide {
            headers.extend([heading("Stopped"), heading("Pending")]);
            widths.extend([Constraint::Length(9), Constraint::Length(9)]);
        }
        let rows = history
            .iter()
            .map(|(day, t)| {
                let mut cells = vec![
                    Cell::from(day.clone()),
                    numeric(tokens(t), ROUTE),
                    numeric(number(t.calls), Color::White),
                ];
                if medium {
                    cells.extend([
                        numeric(number(t.success), CONNECTED),
                        numeric(number(t.failed), if t.failed > 0 { ERROR } else { MUTED }),
                    ]);
                }
                if wide {
                    cells.extend([
                        numeric(number(t.interrupted), WARNING),
                        numeric(number(t.pending), MUTED),
                    ]);
                }
                Row::new(cells).style(if day == &page.day {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                })
            })
            .collect();
        draw_table(frame, area, page, rows, headers, widths, "No history yet.");
    }

    fn draw_usage_details(
        &self,
        frame: &mut ratatui::Frame,
        area: Rect,
        page: &UsagePage,
        daily: &Totals,
        total: &Totals,
    ) {
        let compact_day = page.range_total(
            &self.usage.snapshot,
            page.client(),
            page.provider.as_deref(),
            "compact",
        );
        let compact_total =
            self.usage
                .snapshot
                .total(page.client(), page.provider.as_deref(), None, "compact");
        let known = |t: &Totals, n| {
            if t.calls > 0 && t.unknown == t.calls {
                "unknown".into()
            } else {
                format!("{}{}", number(n), if t.unknown > 0 { " + ?" } else { "" })
            }
        };
        let mut data = vec![
            ("Calls", number(daily.calls), number(total.calls), ROUTE),
            (
                "Success",
                number(daily.success),
                number(total.success),
                CONNECTED,
            ),
            ("Failed", number(daily.failed), number(total.failed), ERROR),
            (
                "Interrupted",
                number(daily.interrupted),
                number(total.interrupted),
                WARNING,
            ),
            (
                "Pending",
                number(daily.pending),
                number(total.pending),
                MUTED,
            ),
            (
                "Input tokens",
                known(daily, daily.input),
                known(total, total.input),
                Color::White,
            ),
            (
                "Output tokens",
                known(daily, daily.output),
                known(total, total.output),
                Color::White,
            ),
            (
                "Cache read",
                number(daily.cache_read),
                number(total.cache_read),
                MUTED,
            ),
            (
                "Cache write",
                number(daily.cache_write),
                number(total.cache_write),
                MUTED,
            ),
            (
                "Missing usage",
                number(daily.unknown),
                number(total.unknown),
                WARNING,
            ),
            (
                "Compaction calls",
                number(compact_day.calls),
                number(compact_total.calls),
                MUTED,
            ),
        ];
        if let Some(since) = self
            .usage
            .snapshot
            .since
            .and_then(|v| chrono::DateTime::from_timestamp(v, 0))
        {
            data.push((
                "Since (UTC)",
                String::new(),
                since.format("%Y-%m-%d").to_string(),
                MUTED,
            ));
        }
        let rows = data
            .into_iter()
            .map(|(name, day, total, color)| {
                Row::new(vec![
                    Cell::from(name),
                    numeric(day, color),
                    numeric(total, color),
                ])
            })
            .collect();
        let table_area = area;
        draw_table(
            frame,
            table_area,
            page,
            rows,
            vec![Cell::from("Metric"), heading("Range"), heading("All time")],
            vec![
                Constraint::Min(12),
                Constraint::Length(if area.width < 60 { 11 } else { 18 }),
                Constraint::Length(if area.width < 60 { 10 } else { 18 }),
            ],
            "",
        );
    }
}

fn draw_summary(
    frame: &mut ratatui::Frame,
    area: Rect,
    daily: &Totals,
    total: &Totals,
    tracked: bool,
    range_label: &str,
) {
    if !tracked || area.is_empty() {
        return;
    }

    // On short screens the summary collapses to one line so table rows remain.
    if area.height < 4 {
        let range = match range_label {
            "1 day" => "1d",
            "1 week" => "7d",
            "1 month" => "30d",
            _ => "All",
        };
        let compact = if range_label == "All time" {
            format!("{} tok", tokens(daily))
        } else {
            format!("{} tok  │  All {} tok", tokens(daily), tokens(total))
        };
        let line = Line::from(vec![
            Span::styled(format!("{range} "), Style::default().fg(ROUTE)),
            Span::styled(
                compact,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }

    if area.width < 58 || range_label == "All time" {
        draw_stat_card(frame, area, range_label, daily, true);
    } else {
        let columns = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        draw_stat_card(frame, columns[0], range_label, daily, true);
        draw_stat_card(frame, columns[1], "All time", total, false);
    }
}

fn draw_stat_card(
    frame: &mut ratatui::Frame,
    area: Rect,
    label: &str,
    totals: &Totals,
    featured: bool,
) {
    let accent = if featured { ROUTE } else { MUTED };
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(accent));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.is_empty() {
        return;
    }

    let amount = if totals.calls > 0 && totals.unknown == totals.calls {
        "unknown".into()
    } else {
        let exact = number(totals.input.saturating_add(totals.output));
        if exact.len() + 8 <= usize::from(inner.width) {
            format!("{exact}{}", if totals.unknown > 0 { " + ?" } else { "" })
        } else {
            tokens(totals)
        }
    };
    let lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{}  ", label.to_ascii_uppercase()),
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} calls", compact(totals.calls)),
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                amount,
                Style::default()
                    .fg(if featured { ROUTE } else { Color::White })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" TOKENS", Style::default().fg(MUTED)),
        ]),
        Line::from(vec![
            Span::styled("IN ", Style::default().fg(MUTED)),
            Span::styled(compact(totals.input), Style::default().fg(Color::White)),
            Span::styled("   OUT ", Style::default().fg(MUTED)),
            Span::styled(compact(totals.output), Style::default().fg(WARNING)),
        ]),
        token_composition(totals, inner.width),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn token_composition(totals: &Totals, width: u16) -> Line<'static> {
    let width = usize::from(width).min(24);
    let total = totals.input.saturating_add(totals.output);
    if width < 3 || total <= 0 {
        return Line::default();
    }
    let input = ((totals.input as f64 / total as f64) * width as f64).round() as usize;
    let input = input.min(width);
    Line::from(vec![
        Span::styled("━".repeat(input), Style::default().fg(ROUTE)),
        Span::styled("━".repeat(width - input), Style::default().fg(WARNING)),
    ])
}

fn draw_table(
    frame: &mut ratatui::Frame,
    area: Rect,
    page: &UsagePage,
    rows: Vec<Row<'static>>,
    headers: Vec<Cell<'static>>,
    widths: Vec<Constraint>,
    empty: &str,
) {
    page.table_area.set(area);
    page.limit
        .set(rows.len().saturating_sub(1).min(u16::MAX as usize) as u16);
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(empty)
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(MUTED)),
            area,
        );
        page.offset.set(0);
        return;
    }
    let selected = page.scroll.min(page.limit.get()) as usize;
    let mut state = TableState::default()
        .with_offset(page.offset.get().min(selected))
        .with_selected(selected);
    let table = Table::new(rows, widths)
        .header(Row::new(headers).style(Style::default().fg(MUTED)))
        .column_spacing(1)
        .row_highlight_style(Style::default().bg(SELECTION));
    frame.render_stateful_widget(table, area, &mut state);
    page.offset.set(state.offset());
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn token_summary_prioritizes_total_and_shows_input_output_mix() {
        let totals = Totals {
            calls: 12,
            input: 9_000,
            output: 3_000,
            ..Default::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 5)).unwrap();
        terminal
            .draw(|frame| {
                draw_summary(
                    frame,
                    Rect::new(0, 0, 80, 4),
                    &totals,
                    &totals,
                    true,
                    "1 day",
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let row = |y| (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>();
        assert!(row(0).contains("1 DAY"));
        assert!(row(1).contains("12,000 TOKENS"));
        assert!(row(2).contains("IN 9,000   OUT 3,000"));
        assert_eq!(buffer[(1, 3)].fg, ROUTE);
        assert_eq!(buffer[(19, 3)].fg, WARNING);

        let mut narrow = Terminal::new(TestBackend::new(40, 3)).unwrap();
        narrow
            .draw(|frame| {
                draw_summary(
                    frame,
                    Rect::new(0, 0, 40, 1),
                    &totals,
                    &totals,
                    true,
                    "1 day",
                )
            })
            .unwrap();
        let text = narrow
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("12.0K tok"));
    }

    #[test]
    fn sessions_filter_sort_render_and_mouse_without_proxy_data() {
        let (_temp, mut app) = crate::tui::tests::persisted_app();
        app.open_usage();
        app.usage.sessions_updated = Some(std::time::Instant::now());
        app.usage.sessions.rows = vec![
            crate::sessions::Session {
                id: "older-codex".into(),
                client: "Codex",
                project: "/work/project".into(),
                updated: 100,
                tokens: crate::sessions::Tokens {
                    input: 1000,
                    output: 20,
                    known: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            crate::sessions::Session {
                id: "recent-claude".into(),
                client: "Claude",
                updated: 200,
                tokens: crate::sessions::Tokens {
                    input: 10,
                    output: 2,
                    known: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        ];
        app.usage.page.as_mut().unwrap().provider = Some("proxy-provider".into());
        app.usage_key(KeyEvent::new(KeyCode::Char('6'), KeyModifiers::NONE));
        app.usage_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        let page = app.usage.page.as_ref().unwrap();
        assert!(page.provider.is_none());
        assert_eq!(app.session_rows(page)[0].id, "recent-claude");
        app.usage_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        assert_eq!(
            app.session_rows(app.usage.page.as_ref().unwrap())[0].id,
            "older-codex"
        );
        // A loading or unavailable proxy ledger must never block local sessions.
        app.usage.query = Some(crate::usage::Query::All);
        app.usage.updated = None;
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
            assert!(text.contains("older-co"));
            assert!(text.contains("1,020"));
            assert!(!text.contains("Loading usage"));
            let screen = Rect::new(0, 0, width, height);
            let a = areas(page_area(screen));
            let session_button = controls(&a, app.usage.page.as_ref().unwrap())
                .into_iter()
                .find(|(_, action, _)| matches!(action, Action::Section(5)))
                .unwrap()
                .2;
            app.usage_mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: session_button.x,
                    row: session_button.y,
                    modifiers: KeyModifiers::NONE,
                },
                screen,
            );
            assert_eq!(app.usage.page.as_ref().unwrap().section, 5);
        }
        app.usage.page.as_mut().unwrap().client = 1;
        assert_eq!(app.session_rows(app.usage.page.as_ref().unwrap()).len(), 1);
        app.usage.page.as_mut().unwrap().client = 3;
        assert!(
            app.session_rows(app.usage.page.as_ref().unwrap())
                .is_empty()
        );
        app.usage.page.as_mut().unwrap().client = 0;
        app.usage.page.as_mut().unwrap().range = 0;
        app.usage.page.as_mut().unwrap().day = "2026-09-22".into();
        assert!(
            app.session_rows(app.usage.page.as_ref().unwrap())
                .is_empty()
        );
    }

    #[test]
    fn usage_drilldown_dates_and_mouse_targets_are_consistent() {
        let (_temp, mut app) = crate::tui::tests::persisted_app();
        let today = app.usage.snapshot.today();
        let yesterday = chrono::NaiveDate::parse_from_str(&today, "%Y-%m-%d")
            .unwrap()
            .pred_opt()
            .unwrap()
            .to_string();
        app.usage.snapshot.rows = vec![crate::usage::Row {
            hour: 12,
            model: "claude-sonnet-test".into(),
            day: yesterday.clone(),
            client: "Claude".into(),
            provider: "fixture".into(),
            name: "Fixture".into(),
            kind: "generation".into(),
            totals: Totals {
                calls: 17,
                success: 16,
                failed: 1,
                input: 1200,
                output: 400,
                ..Default::default()
            },
        }];
        app.open_usage();
        let key = |app: &mut App, code| {
            app.usage_key(KeyEvent::new(code, KeyModifiers::NONE));
        };
        key(&mut app, KeyCode::Right);
        assert_eq!(app.usage.page.as_ref().unwrap().day, today);
        key(&mut app, KeyCode::Left);
        assert_eq!(app.usage.page.as_ref().unwrap().day, yesterday);
        assert!(!app.usage.page.as_ref().unwrap().follow_today);
        key(&mut app, KeyCode::Char('t'));
        assert!(app.usage.page.as_ref().unwrap().follow_today);
        key(&mut app, KeyCode::Char('w'));
        key(&mut app, KeyCode::Char('2'));
        key(&mut app, KeyCode::Enter);
        let page = app.usage.page.as_ref().unwrap();
        assert_eq!((page.section, page.day.as_str()), (3, yesterday.as_str()));
        key(&mut app, KeyCode::Char('1'));
        key(&mut app, KeyCode::Left);
        key(&mut app, KeyCode::Right);
        // Select the fixture through a client filter so empty configured providers do not interfere.
        app.usage.page.as_mut().unwrap().provider = Some("fixture".into());
        let screen = Rect::new(0, 0, 80, 24);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let body = app.usage.page.as_ref().unwrap().table_area.get();
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: body.x + 1,
            row: body.y + 1,
            modifiers: KeyModifiers::NONE,
        };
        app.usage_mouse(click, screen);
        assert_eq!(
            app.usage.page.as_ref().unwrap().section,
            0,
            "first click selects only"
        );
        app.usage_mouse(click, screen);
        assert_eq!(
            app.usage.page.as_ref().unwrap().section,
            3,
            "second click opens models"
        );
        assert_eq!(
            app.usage.page.as_ref().unwrap().provider.as_deref(),
            Some("fixture")
        );
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
            assert!(!text.contains("Auto-refresh"));
            assert!(!text.contains("Proxy calls only"));
            assert!(text.contains("Models 4") || text.contains("Models4"));
            let a = areas(page_area(Rect::new(0, 0, width, height)));
            let page = app.usage.page.as_ref().unwrap();
            let controls = controls(&a, page);
            let prev = controls
                .iter()
                .find(|(_, action, _)| matches!(action, Action::Previous))
                .unwrap()
                .2;
            let date = controls
                .iter()
                .find(|(_, action, _)| matches!(action, Action::DateLabel))
                .unwrap()
                .2;
            let next = controls
                .iter()
                .find(|(_, action, _)| matches!(action, Action::Next))
                .unwrap()
                .2;
            assert!(prev.right() < date.x && date.right() < next.x);
            assert!(next.right() <= a.date.right());
            if width == 80 {
                for y in 0..height {
                    println!(
                        "{}",
                        (0..width)
                            .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                            .collect::<String>()
                    );
                }
            }
        }
        key(&mut app, KeyCode::Char('5'));
        for (width, height) in [(40, 12), (80, 24), (120, 36)] {
            let screen = Rect::new(0, 0, width, height);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| app.draw(frame)).unwrap();
            let a = areas(page_area(screen));
            let page = app.usage.page.as_ref().unwrap();
            let (_, _, range_rect) = controls(&a, page)
                .into_iter()
                .find(|(_, action, _)| matches!(action, Action::Range(1)))
                .unwrap();
            app.usage_mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: range_rect.x,
                    row: range_rect.y,
                    modifiers: KeyModifiers::NONE,
                },
                screen,
            );
            assert_eq!(app.usage.page.as_ref().unwrap().range, 1);
            key(&mut app, KeyCode::Char('v'));
            terminal.draw(|frame| app.draw(frame)).unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            if height >= 24 {
                assert!(text.contains("Tokens / day"));
                assert!(
                    text.chars().any(|ch| "█▇▆▅▄▃▂▁".contains(ch)),
                    "usage must render vertical bars"
                );
                if width == 80 {
                    for y in 0..height {
                        println!(
                            "{}",
                            (0..width)
                                .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                                .collect::<String>()
                        );
                    }
                }
            }
            assert!(text.contains("1 week"));
            assert!(app.usage.page.as_ref().unwrap().chart_tokens);
        }
        key(&mut app, KeyCode::Esc);
        assert!(app.usage.active);
        assert!(app.usage.page.as_ref().unwrap().provider.is_none());
        assert_eq!(app.usage.page.as_ref().unwrap().section, 0);
        key(&mut app, KeyCode::Esc);
        assert!(!app.usage.active);
    }

    #[test]
    fn all_usage_button_rows_share_spacing_and_fit() {
        let (_temp, mut app) = crate::tui::tests::persisted_app();
        app.open_usage();
        let page = app.usage.page.as_mut().unwrap();
        page.section = 4;
        page.provider = Some("fixture".into());
        for width in [40, 60, 72, 80, 120] {
            let a = areas(page_area(Rect::new(0, 0, width, 24)));
            let buttons = controls(&a, page);
            for row in [a.clients, a.date, a.range, a.sections, a.body] {
                let list: Vec<_> = buttons
                    .iter()
                    .filter(|(_, _, rect)| rect.y == row.y)
                    .collect();
                assert!(!list.is_empty());
                for (_, _, rect) in &list {
                    assert!(rect.x >= row.x && rect.right() <= row.right());
                }
                let gap = if row.width >= 70 { 2 } else { 1 };
                for pair in list.windows(2) {
                    assert_eq!(pair[1].2.x - pair[0].2.right(), gap);
                }
            }
            assert_eq!(
                buttons
                    .iter()
                    .filter(|(_, action, _)| matches!(action, Action::Section(_)))
                    .count(),
                6
            );
            assert_eq!(
                buttons
                    .iter()
                    .filter(|(_, action, _)| matches!(action, Action::Range(_)))
                    .count(),
                4
            );
            assert!(
                buttons
                    .iter()
                    .any(|(_, action, _)| matches!(action, Action::Refresh))
            );
            assert!(
                buttons
                    .iter()
                    .any(|(_, action, _)| matches!(action, Action::All))
            );
        }
    }
}
