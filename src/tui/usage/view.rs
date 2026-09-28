use super::*;

pub(super) struct ProviderRow {
    pub(super) client: String,
    pub(super) id: String,
    pub(super) name: String,
    pub(super) daily: Totals,
    pub(super) total: Totals,
}

pub(super) struct ModelRow {
    pub(super) client: String,
    pub(super) provider: String,
    pub(super) model: String,
    pub(super) daily: Totals,
    pub(super) total: Totals,
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

impl App {
    fn session_rows(&self, page: &UsagePage) -> Vec<&crate::sessions::Session> {
        let zone = chrono::FixedOffset::east_opt(self.usage.snapshot.offset)
            .unwrap_or_else(|| chrono::FixedOffset::east_opt(0).unwrap());
        let terms: Vec<_> = page
            .session_search
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let mut rows: Vec<_> = self
            .usage
            .sessions
            .rows
            .iter()
            .filter(|s| {
                page.client().is_none_or(|c| c == s.client)
                    && (terms.is_empty() || {
                        let haystack = format!(
                            "{} {} {} {}",
                            s.id,
                            s.project,
                            s.client,
                            s.models.iter().cloned().collect::<Vec<_>>().join(" ")
                        )
                        .to_lowercase();
                        terms.iter().all(|word| haystack.contains(word))
                    })
                    && (!page.session_follow_range
                        || page.range == 3
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
}
#[derive(Clone, Default)]
pub(super) struct DashboardState {
    pub(super) viewport: Rect,
    anchors: [usize; 6],
    selected: [Option<String>; 6],
    rows: [Vec<(String, usize)>; 6],
    hits: Vec<(Rect, usize, usize)>,
    controls: Vec<(Rect, Action)>,
    jump: Option<usize>,
    reveal: Option<(usize, String)>,
    total_height: usize,
    session_detail_y: usize,
    inspect_session: bool,
    pub(super) effective_scroll: usize,
}
#[derive(Clone, Copy)]
enum Action {
    Client(usize),
    Range(usize),
    Previous,
    Next,
    Today,
    DateLabel,
    Sort,
    Clear,
    Refresh,
    Close,
    Quit,
    Reset,
    Search,
    SessionScope,
    SessionClear,
    SessionPrevious,
    SessionNext,
    Activate(usize),
}
impl UsagePage {
    pub(super) fn jump_to_section(&mut self) {
        self.dashboard.borrow_mut().jump = Some(self.section);
    }
    pub(super) fn select_table_row(&mut self, down: bool) {
        let mut state = self.dashboard.borrow_mut();
        let rows = &state.rows[self.section];
        if rows.is_empty() {
            return;
        }
        let index = rows
            .iter()
            .position(|(key, _)| Some(key) == state.selected[self.section].as_ref())
            .unwrap_or(0);
        let next = if down {
            (index + 1).min(rows.len() - 1)
        } else {
            index.saturating_sub(1)
        };
        let (key, y) = rows[next].clone();
        state.selected[self.section] = Some(key);
        state.reveal = Some((self.section, state.selected[self.section].clone().unwrap()));
        self.scroll = y
            .saturating_sub(usize::from(state.viewport.height.saturating_sub(1)).min(2))
            .min(self.limit.get());
    }
}
fn label(text: impl Into<String>) -> Span<'static> {
    Span::styled(text.into(), Style::default().fg(FIELD_LABEL))
}
fn value(text: impl Into<String>, color: Color) -> Span<'static> {
    Span::styled(
        text.into(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}
fn muted(text: impl Into<String>) -> Span<'static> {
    Span::styled(text.into(), Style::default().fg(MUTED))
}
fn pair(name: &str, text: impl Into<String>, color: Color) -> Line<'static> {
    Line::from(vec![label(format!("{name}  ")), value(text, color)])
}
fn fit(text: &str, width: usize, right: bool) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let size = c.width().unwrap_or(0);
        if used + size > width {
            break;
        }
        out.push(c);
        used += size;
    }
    let padding = " ".repeat(width.saturating_sub(used));
    if right {
        format!("{padding}{out}")
    } else {
        format!("{out}{padding}")
    }
}
fn wrapped(lines: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .flat_map(|line| {
            wrap_styled_segments(
                line.spans
                    .into_iter()
                    .map(|span| (span.content.into_owned(), span.style))
                    .collect(),
                width.max(1),
            )
        })
        .collect()
}
fn io_bar(input: i64, output: i64, width: u16) -> Line<'static> {
    let total = input.saturating_add(output).max(0);
    let count = usize::from(width);
    let inputs = if total > 0 {
        ((input.max(0) as f64 / total as f64) * count as f64).round() as usize
    } else {
        0
    }
    .min(count);
    Line::from(vec![
        Span::styled("━".repeat(inputs), Style::default().fg(ENABLED)),
        Span::styled(
            "━".repeat(if total > 0 { count - inputs } else { 0 }),
            Style::default().fg(DATA_SECONDARY),
        ),
        Span::styled(
            if total == 0 {
                "░".repeat(count)
            } else {
                String::new()
            },
            Style::default().fg(MUTED),
        ),
    ])
}
fn health_bar(t: &Totals, width: u16) -> Line<'static> {
    let counts = [t.success.max(0), t.failed.max(0), t.interrupted.max(0)];
    let completed: i64 = counts.iter().sum();
    if completed == 0 {
        return Line::styled("░".repeat(width.into()), Style::default().fg(MUTED));
    }
    let mut spans = vec![];
    let mut used = 0usize;
    let mut cumulative = 0i64;
    for (count, color) in counts.into_iter().zip([CONNECTED, ERROR, WARNING]) {
        cumulative += count;
        let end = ((cumulative as f64 / completed as f64) * f64::from(width)).round() as usize;
        spans.push(Span::styled(
            "━".repeat(end.saturating_sub(used)),
            Style::default().fg(color),
        ));
        used = end;
    }
    Line::from(spans)
}
fn known(t: &Totals, n: i64) -> String {
    if t.calls > 0 && t.unknown == t.calls {
        "unknown".into()
    } else {
        format!("{}{}", number(n), if t.unknown > 0 { " + ?" } else { "" })
    }
}

// Layout uses document coordinates. Only visible lines are formatted and painted;
// the same transformation creates mouse hit targets for the displayed rows.
struct Canvas<'a, 'b> {
    frame: Option<&'a mut ratatui::Frame<'b>>,
    viewport: Rect,
    scroll: usize,
    theme: theme::Theme,
    focus: usize,
    state: &'a mut DashboardState,
}
impl Canvas<'_, '_> {
    fn visible(&self, y: usize) -> Option<u16> {
        if y < self.scroll || y >= self.scroll + usize::from(self.viewport.height) {
            None
        } else {
            Some(self.viewport.y + (y - self.scroll) as u16)
        }
    }
    fn line(&mut self, x: u16, y: usize, width: u16, make: impl FnOnce() -> Line<'static>) {
        let Some(screen_y) = self.visible(y) else {
            return;
        };
        if let Some(frame) = self.frame.as_mut() {
            frame.render_widget(Paragraph::new(make()), Rect::new(x, screen_y, width, 1));
        }
    }
    fn rail(&mut self, x: u16, y: usize, width: u16) {
        self.line(x, y, 1, || {
            Line::styled("│", Style::default().fg(theme::EDGE))
        });
        self.line(x + width.saturating_sub(1), y, 1, || {
            Line::styled("│", Style::default().fg(theme::EDGE))
        });
    }
    fn content(&mut self, x: u16, y: usize, width: u16, make: impl FnOnce() -> Line<'static>) {
        self.rail(x, y, width);
        self.line(x + 1, y, width.saturating_sub(2), make);
    }
    fn detail_content(&mut self, x: u16, y: usize, width: u16, mut line: Line<'static>) {
        line.spans.insert(0, Span::raw(" "));
        self.content(x, y, width, || line);
    }
    fn header(&mut self, x: u16, y: usize, width: u16, title: &str, section: Option<usize>) {
        if let Some(section) = section {
            self.state.anchors[section] = y;
        }
        let color = if section == Some(self.focus) {
            theme::ACTIVE_EDGE
        } else {
            theme::EDGE
        };
        self.line(x, y, width, || {
            let title = fit(
                title,
                usize::from(width.saturating_sub(6)).min(title.width()),
                false,
            );
            let trailing = usize::from(width).saturating_sub(title.width() + 5);
            Line::from(vec![
                Span::styled("┌─ ", Style::default().fg(color)),
                value(title, FIELD_LABEL),
                Span::styled(
                    format!(" {}┐", "─".repeat(trailing)),
                    Style::default().fg(color),
                ),
            ])
        });
    }
    fn bottom(&mut self, x: u16, y: usize, width: u16) {
        self.line(x, y, width, || {
            Line::styled(
                format!("└{}┘", "─".repeat(usize::from(width.saturating_sub(2)))),
                Style::default().fg(theme::EDGE),
            )
        });
    }
    fn prepare(&mut self, section: usize, keys: Vec<String>) {
        if !keys
            .iter()
            .any(|key| Some(key) == self.state.selected[section].as_ref())
        {
            self.state.selected[section] = keys.first().cloned();
        }
        self.state.rows[section] = keys.into_iter().map(|key| (key, 0)).collect();
    }
    fn selected(&self, section: usize, index: usize) -> bool {
        self.state.rows[section]
            .get(index)
            .is_some_and(|(key, _)| Some(key) == self.state.selected[section].as_ref())
    }
    fn row(
        &mut self,
        x: u16,
        y: usize,
        width: u16,
        section: usize,
        index: usize,
        make: impl FnOnce(u16) -> Line<'static>,
    ) {
        self.state.rows[section][index].1 = y;
        let selected = self.selected(section, index);
        let focused = selected && self.focus == section;
        let symbol = if focused {
            self.theme.selection_symbol()
        } else {
            ""
        };
        let marker_width = u16::from(self.theme.terminal_background());
        if let Some(screen_y) = self.visible(y)
            && self.frame.is_some()
        {
            self.state.hits.push((
                Rect::new(x + 1, screen_y, width.saturating_sub(2), 1),
                section,
                index,
            ));
        }
        self.content(x, y, width, || {
            let mut line = make(width.saturating_sub(2 + marker_width));
            if marker_width > 0 {
                line.spans.insert(
                    0,
                    value(if symbol.is_empty() { " " } else { symbol }, ROUTE),
                );
            }
            if focused {
                line = line.style(Style::default().bg(theme::PROVIDER_SELECTION));
            }
            line
        });
    }
}

fn gateway_lines(
    t: &Totals,
    total: &Totals,
    range: &str,
    width: u16,
    large: bool,
    loading: bool,
    unavailable: bool,
    pi: bool,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![
        value(range.to_ascii_uppercase(), FIELD_LABEL),
        muted(" · generation tokens"),
    ])];
    if loading || (unavailable && t.calls == 0) || (pi && t.calls == 0) {
        lines.push(Line::styled(
            if loading {
                "Reading gateway usage…"
            } else if pi {
                "Not tracked · direct API"
            } else {
                "Usage unavailable · r retry"
            },
            Style::default().fg(if loading { MUTED } else { WARNING }),
        ));
        return lines;
    }
    let unknown = t.calls > 0 && t.unknown == t.calls;
    let amount = if unknown {
        "—".into()
    } else {
        compact(t.input + t.output)
    };
    if large && amount.width() * 4 <= usize::from(width) {
        lines.extend(
            super::super::meters::digits(&amount)
                .into_iter()
                .map(|line| {
                    Line::styled(
                        line,
                        Style::default().fg(ROUTE).add_modifier(Modifier::BOLD),
                    )
                }),
        );
    } else {
        lines.push(Line::from(vec![value(amount, ROUTE), label(" TOKENS")]));
    }
    lines.push(pair(
        "Exact",
        known(t, t.input.saturating_add(t.output)),
        ROUTE,
    ));
    lines.push(Line::from(vec![
        label("IN "),
        value(known(t, t.input), ENABLED),
        label("  OUT "),
        value(known(t, t.output), DATA_SECONDARY),
    ]));
    lines.push(if unknown {
        Line::styled("Input/output unavailable", Style::default().fg(MUTED))
    } else {
        io_bar(t.input, t.output, width)
    });
    lines.push(Line::from(vec![
        label("Calls "),
        value(number(t.calls), Color::White),
        label("  Unknown "),
        value(
            number(t.unknown),
            if t.unknown > 0 { WARNING } else { MUTED },
        ),
    ]));
    lines.push(Line::from(vec![
        label("CACHE  read "),
        value(
            if unknown {
                "—".into()
            } else {
                number(t.cache_read)
            },
            DEFAULT_MODEL,
        ),
        label("  write "),
        value(
            if unknown {
                "—".into()
            } else {
                number(t.cache_write)
            },
            DEFAULT_MODEL,
        ),
    ]));
    let hit = (t.cache_input > 0).then(|| t.cache_hits as f64 / t.cache_input as f64);
    let mut spans = vec![label("HIT ")];
    spans.extend(super::super::meters::progress_spans(
        hit,
        usize::from(width.saturating_sub(14)).min(24),
        DEFAULT_MODEL,
        MUTED,
    ));
    spans.push(value(
        hit.map_or(" — no samples".into(), |n| {
            format!(" {n:.1}%", n = n * 100.0)
        }),
        DEFAULT_MODEL,
    ));
    lines.push(Line::from(spans));
    let speed = (t.speed_ms > 0 && t.speed_samples > 0)
        .then(|| 1000.0 * t.speed_output as f64 / t.speed_ms as f64);
    let average = if t.calls > t.unknown {
        compact((t.input + t.output) / (t.calls - t.unknown))
    } else {
        "—".into()
    };
    lines.push(pair("AVG", format!("{average} tok/call"), ROUTE));
    lines.push(pair(
        "RATE (E2E)",
        speed.map_or("—".into(), |n| format!("{n:.1} tok/s")),
        ROUTE,
    ));
    lines.push(Line::from(vec![
        muted(format!("{} measured streams", t.speed_samples)),
        muted(format!("  ·  All time {} tok", tokens(total))),
    ]));
    wrapped(lines, width)
}
fn health_lines(t: &Totals, compaction: &Totals, width: u16, loading: bool) -> Vec<Line<'static>> {
    if loading {
        return vec![Line::styled(
            "Waiting for gateway ledger…",
            Style::default().fg(MUTED),
        )];
    }
    let completed = t.success + t.failed + t.interrupted;
    let rate = if completed > 0 {
        format!("{:.1}%", 100.0 * t.success as f64 / completed as f64)
    } else {
        "— no samples".into()
    };
    wrapped(
        vec![
            pair("SUCCESS", rate, CONNECTED),
            health_bar(t, width),
            pair("Success", number(t.success), CONNECTED),
            pair(
                "Failed",
                number(t.failed),
                if t.failed > 0 { ERROR } else { MUTED },
            ),
            pair(
                "Stopped",
                number(t.interrupted),
                if t.interrupted > 0 { WARNING } else { MUTED },
            ),
            pair(
                "In flight",
                number(t.pending),
                if t.pending > 0 { ROUTE } else { MUTED },
            ),
            pair(
                "Missing usage",
                number(t.unknown),
                if t.unknown > 0 { WARNING } else { MUTED },
            ),
            pair("Compaction calls", number(compaction.calls), Color::White),
            Line::styled(
                "Pending excluded from success rate",
                Style::default().fg(MUTED),
            ),
        ],
        width,
    )
}
fn session_detail(s: &crate::sessions::Session, width: u16) -> Vec<Line<'static>> {
    let n = |v| {
        if s.tokens.known {
            number(v)
        } else {
            "unknown".into()
        }
    };
    wrapped(
        vec![
            Line::from(vec![
                value(s.client, FIELD_LABEL),
                muted(format!(
                    " · {}{}{}",
                    s.id,
                    if s.child { " · child" } else { "" },
                    if s.fork { " · fork *" } else { "" }
                )),
            ]),
            Line::from(vec![
                label("IN "),
                value(n(s.tokens.input), ENABLED),
                label("  OUT "),
                value(n(s.tokens.output), DATA_SECONDARY),
                label("  TOTAL "),
                value(n(s.tokens.total()), ROUTE),
            ]),
            if s.tokens.known {
                io_bar(s.tokens.input, s.tokens.output, width)
            } else {
                Line::styled("Input/output unavailable", Style::default().fg(MUTED))
            },
            Line::from(vec![
                label("CACHE  read "),
                value(
                    if s.tokens.cache_known {
                        n(s.tokens.read)
                    } else {
                        "—".into()
                    },
                    DEFAULT_MODEL,
                ),
                label("  write "),
                value(
                    if s.tokens.cache_known {
                        n(s.tokens.write)
                    } else {
                        "—".into()
                    },
                    DEFAULT_MODEL,
                ),
            ]),
            pair(
                "Cache reuse",
                s.tokens
                    .cache_reuse_percent()
                    .map_or("—".into(), |rate| format!("{rate:.1}%")),
                DEFAULT_MODEL,
            ),
            pair("Project", s.project.clone(), Color::White),
            pair(
                "Models",
                s.models.iter().cloned().collect::<Vec<_>>().join(", "),
                DATA_SECONDARY,
            ),
            Line::styled(
                if s.incomplete {
                    "+ ? partial token log"
                } else if s.fork {
                    "* may include inherited usage"
                } else {
                    "Lifetime totals · independent of gateway"
                },
                Style::default().fg(if s.incomplete || s.fork {
                    WARNING
                } else {
                    MUTED
                }),
            ),
        ],
        width,
    )
}

fn cells(name: &str, t: &Totals, all: Option<&Totals>, width: u16) -> Line<'static> {
    let mut columns = vec![
        (tokens(t), 12usize, ROUTE),
        (compact(t.calls), 8, Color::White),
    ];
    if width >= 66
        && let Some(all) = all
    {
        columns.push((tokens(all), 12, MUTED));
    }
    let numeric_width: usize = columns.iter().map(|(_, width, _)| width + 1).sum();
    let name_width = usize::from(width).saturating_sub(numeric_width);
    let mut spans = vec![Span::styled(
        fit(name, name_width, false),
        Style::default().fg(Color::White),
    )];
    for (text, length, color) in columns {
        spans.push(Span::raw(" "));
        spans.push(value(fit(&text, length, true), color));
    }
    Line::from(spans)
}
fn table_header(name: &str, width: u16, all: bool) -> Line<'static> {
    let numeric_width = 13 + 9 + if all && width >= 66 { 13 } else { 0 };
    let mut spans = vec![
        label(fit(
            name,
            usize::from(width).saturating_sub(numeric_width),
            false,
        )),
        value(format!(" {}", fit("Tokens", 12, true)), ROUTE),
        label(format!(" {}", fit("Calls", 8, true))),
    ];
    if all && width >= 66 {
        spans.push(muted(format!(" {}", fit("All tokens", 12, true))));
    }
    Line::from(spans)
}
struct Data<'a> {
    providers: Vec<ProviderRow>,
    models: Vec<ModelRow>,
    history: Vec<(String, Totals)>,
    sessions: Vec<&'a crate::sessions::Session>,
    daily: Totals,
    total: Totals,
    compact: Totals,
}
impl App {
    fn gateway_waiting(&self) -> bool {
        self.usage.snapshot.rows.is_empty()
            && (self.usage.updated.is_none() || self.usage.error.is_some())
    }
    fn dashboard_content(&self, c: &mut Canvas<'_, '_>, page: &UsagePage, d: &Data<'_>) -> usize {
        c.state.hits.clear();
        c.prepare(
            0,
            d.providers
                .iter()
                .map(|p| format!("{}\u{1f}{}", p.client, p.id))
                .collect(),
        );
        c.prepare(1, d.history.iter().map(|(day, _)| day.clone()).collect());
        c.prepare(
            3,
            d.models
                .iter()
                .map(|m| format!("{}\u{1f}{}\u{1f}{}", m.client, m.provider, m.model))
                .collect(),
        );
        c.prepare(
            5,
            d.sessions
                .iter()
                .map(|s| format!("{}\u{1f}{}", s.client, s.id))
                .collect(),
        );
        let x = c.viewport.x;
        let width = c.viewport.width;
        let gap = 2;
        let mut y = 0;
        let gateway_width = if self.screen.width >= 80 {
            width * 3 / 5
        } else {
            width
        };
        let gateway = gateway_lines(
            &d.daily,
            &d.total,
            page.range_label(),
            gateway_width.saturating_sub(2),
            self.screen.width >= 80,
            self.usage.updated.is_none() && self.usage.snapshot.rows.is_empty(),
            self.usage.error.is_some() && self.usage.snapshot.rows.is_empty(),
            page.client() == Some("Pi"),
        );
        let health_width = if self.screen.width >= 80 {
            width - gateway_width - gap
        } else {
            width
        };
        let health = health_lines(
            &d.daily,
            &d.compact,
            health_width.saturating_sub(2),
            (self.usage.updated.is_none() || self.usage.error.is_some())
                && self.usage.snapshot.rows.is_empty(),
        );
        let paint_card =
            |c: &mut Canvas<'_, '_>, x, y, width, title: &str, lines: Vec<Line<'static>>| {
                c.header(x, y, width, title, None);
                let height = lines.len();
                for (index, line) in lines.into_iter().enumerate() {
                    c.content(x, y + 1 + index, width, || line);
                }
                c.bottom(x, y + height + 1, width);
                height + 2
            };
        let gateway_height = paint_card(c, x, y, gateway_width, "GATEWAY / TOKENS", gateway);
        if self.screen.width >= 80 {
            let health_height = paint_card(
                c,
                x + gateway_width + gap,
                y,
                health_width,
                "CALL HEALTH",
                health,
            );
            y += gateway_height.max(health_height) + 1;
        } else {
            y += gateway_height + 1;
            y += paint_card(c, x, y, width, "CALL HEALTH", health) + 1;
        }
        c.state.anchors[4] = y;
        let chart_width = if self.screen.width >= 120 {
            (width - gap) / 2
        } else {
            width
        };
        let calls = chart::lines(
            &self.usage.snapshot,
            page,
            false,
            chart_width.saturating_sub(2),
        );
        let token_chart_width = if self.screen.width >= 120 {
            width - chart_width - gap
        } else {
            width
        };
        let token_chart = chart::lines(
            &self.usage.snapshot,
            page,
            true,
            token_chart_width.saturating_sub(2),
        );
        let unavailable = (self.usage.updated.is_none() || self.usage.error.is_some())
            && self.usage.snapshot.rows.is_empty();
        let calls = if unavailable {
            vec![Line::styled(
                "Trend unavailable · waiting for ledger",
                Style::default().fg(MUTED),
            )]
        } else {
            calls
        };
        let token_chart = if unavailable {
            vec![Line::styled(
                "Trend unavailable · waiting for ledger",
                Style::default().fg(MUTED),
            )]
        } else {
            token_chart
        };
        let h = paint_card(c, x, y, chart_width, "TREND / CALLS · 5", calls);
        if self.screen.width >= 120 {
            let other = paint_card(
                c,
                x + chart_width + gap,
                y,
                token_chart_width,
                "TREND / TOKENS",
                token_chart,
            );
            y += h.max(other) + 1;
        } else {
            y += h + 1;
            y += paint_card(c, x, y, width, "TREND / TOKENS", token_chart) + 1;
        }
        let left_width = if self.screen.width >= 120 {
            (width - gap) / 2
        } else {
            width
        };
        let right_width = if self.screen.width >= 120 {
            width - left_width - gap
        } else {
            width
        };
        let left_end = self.dashboard_providers(c, x, y, left_width, d);
        if self.screen.width >= 120 {
            let right_end = self.dashboard_models(c, x + left_width + gap, y, right_width, d);
            y = left_end.max(right_end) + 1;
        } else {
            y = self.dashboard_models(c, x, left_end + 1, width, d) + 1;
        }
        let left_end = self.dashboard_history(c, x, y, left_width, d);
        if self.screen.width >= 120 {
            let right_end =
                self.dashboard_metrics(c, x + left_width + gap, y, right_width, page, d);
            y = left_end.max(right_end) + 1;
        } else {
            y = self.dashboard_metrics(c, x, left_end + 1, width, page, d) + 1;
        }
        self.dashboard_sessions(c, x, y, width, page, d)
    }
    fn dashboard_providers(
        &self,
        c: &mut Canvas<'_, '_>,
        x: u16,
        mut y: usize,
        width: u16,
        d: &Data<'_>,
    ) -> usize {
        c.header(x, y, width, "PROVIDERS · 1", Some(0));
        y += 1;
        c.content(x, y, width, || {
            table_header("Provider", width.saturating_sub(2), true)
        });
        y += 1;
        if d.providers.is_empty() {
            c.content(x, y, width, || {
                Line::styled(
                    "No tracked providers in this scope",
                    Style::default().fg(MUTED),
                )
            });
            y += 1;
        }
        for (index, p) in d.providers.iter().enumerate() {
            c.row(x, y, width, 0, index, |w| {
                if (self.usage.updated.is_none() || self.usage.error.is_some())
                    && self.usage.snapshot.rows.is_empty()
                {
                    Line::from(vec![
                        Span::raw(fit(&p.name, usize::from(w).saturating_sub(12), false)),
                        muted(" — waiting"),
                    ])
                } else {
                    cells(&p.name, &p.daily, Some(&p.total), w)
                }
            });
            y += 1;
            if c.selected(0, index) {
                let mut lines = wrapped(
                    vec![
                        Line::from(vec![
                            label("Provider "),
                            value(p.name.clone(), Color::White),
                            muted(format!(" · {} · {}", p.client, p.id)),
                        ]),
                        pair(
                            "Range",
                            format!(
                                "{} tokens · {} calls",
                                known(&p.daily, p.daily.input.saturating_add(p.daily.output)),
                                number(p.daily.calls)
                            ),
                            ROUTE,
                        ),
                        pair(
                            "All time",
                            format!(
                                "{} tokens · {} calls",
                                known(&p.total, p.total.input.saturating_add(p.total.output)),
                                number(p.total.calls)
                            ),
                            MUTED,
                        ),
                    ],
                    width.saturating_sub(3),
                );
                if self.gateway_waiting() {
                    lines = wrapped(
                        vec![
                            Line::from(vec![
                                label("Provider "),
                                value(p.name.clone(), Color::White),
                                muted(format!(" · {} · {}", p.client, p.id)),
                            ]),
                            Line::styled(
                                "Gateway usage unavailable · r refresh",
                                Style::default().fg(MUTED),
                            ),
                        ],
                        width.saturating_sub(3),
                    );
                }
                for line in lines {
                    c.detail_content(x, y, width, line);
                    y += 1;
                }
            }
        }
        c.bottom(x, y, width);
        y + 1
    }
    fn dashboard_models(
        &self,
        c: &mut Canvas<'_, '_>,
        x: u16,
        mut y: usize,
        width: u16,
        d: &Data<'_>,
    ) -> usize {
        c.header(x, y, width, "MODELS · 4", Some(3));
        y += 1;
        c.content(x, y, width, || {
            table_header("Model", width.saturating_sub(2), true)
        });
        y += 1;
        if d.models.is_empty() {
            c.content(x, y, width, || {
                Line::styled("No model calls in this scope", Style::default().fg(MUTED))
            });
            y += 1;
        }
        for (index, m) in d.models.iter().enumerate() {
            c.row(x, y, width, 3, index, |w| {
                cells(
                    if m.model.is_empty() {
                        "(unspecified)"
                    } else {
                        &m.model
                    },
                    &m.daily,
                    Some(&m.total),
                    w,
                )
            });
            y += 1;
            if c.selected(3, index) {
                let lines = wrapped(
                    vec![
                        pair("Model", m.model.clone(), DATA_SECONDARY),
                        Line::from(vec![
                            label("Provider "),
                            value(m.provider.clone(), FIELD_LABEL),
                            muted(format!(" · {}", m.client)),
                        ]),
                        pair(
                            "Range",
                            format!(
                                "{} tokens · {} calls",
                                known(&m.daily, m.daily.input.saturating_add(m.daily.output)),
                                number(m.daily.calls)
                            ),
                            ROUTE,
                        ),
                        pair(
                            "All time",
                            format!(
                                "{} tokens · {} calls",
                                known(&m.total, m.total.input.saturating_add(m.total.output)),
                                number(m.total.calls)
                            ),
                            MUTED,
                        ),
                        pair(
                            "Failed / stopped",
                            format!("{} / {}", m.daily.failed, m.daily.interrupted),
                            if m.daily.failed > 0 { ERROR } else { MUTED },
                        ),
                    ],
                    width.saturating_sub(3),
                );
                for line in lines {
                    c.detail_content(x, y, width, line);
                    y += 1;
                }
            }
        }
        c.bottom(x, y, width);
        y + 1
    }
    fn dashboard_history(
        &self,
        c: &mut Canvas<'_, '_>,
        x: u16,
        mut y: usize,
        width: u16,
        d: &Data<'_>,
    ) -> usize {
        c.header(x, y, width, "HISTORY · 2", Some(1));
        y += 1;
        c.content(x, y, width, || {
            table_header("Date", width.saturating_sub(2), false)
        });
        y += 1;
        if d.history.is_empty() {
            c.content(x, y, width, || {
                Line::styled(
                    if self.gateway_waiting() {
                        "Waiting for history…"
                    } else {
                        "No history in this range"
                    },
                    Style::default().fg(MUTED),
                )
            });
            y += 1;
        }
        for (index, (day, t)) in d.history.iter().enumerate() {
            c.row(x, y, width, 1, index, |w| cells(day, t, None, w));
            y += 1;
            if c.selected(1, index) {
                let lines = wrapped(
                    vec![
                        Line::from(vec![
                            label("Success "),
                            value(number(t.success), CONNECTED),
                            label("  Failed "),
                            value(number(t.failed), if t.failed > 0 { ERROR } else { MUTED }),
                            label("  Stopped "),
                            value(
                                number(t.interrupted),
                                if t.interrupted > 0 { WARNING } else { MUTED },
                            ),
                            label("  Pending "),
                            value(number(t.pending), MUTED),
                        ]),
                        Line::styled(
                            "View this day [Enter]",
                            Style::default().fg(ROUTE).add_modifier(Modifier::BOLD),
                        ),
                    ],
                    width.saturating_sub(3),
                );
                for line in lines {
                    if line.to_string().contains("View this day") {
                        if let Some(screen_y) = c.visible(y)
                            && c.frame.is_some()
                        {
                            c.state.controls.push((
                                Rect::new(x + 2, screen_y, width.saturating_sub(3), 1),
                                Action::Activate(1),
                            ));
                        }
                    }
                    c.detail_content(x, y, width, line);
                    y += 1;
                }
            }
        }
        c.bottom(x, y, width);
        y + 1
    }
    fn dashboard_metrics(
        &self,
        c: &mut Canvas<'_, '_>,
        x: u16,
        mut y: usize,
        width: u16,
        page: &UsagePage,
        d: &Data<'_>,
    ) -> usize {
        c.header(x, y, width, "METRICS · 3", Some(2));
        y += 1;
        if (self.usage.updated.is_none() || self.usage.error.is_some())
            && self.usage.snapshot.rows.is_empty()
        {
            c.content(x, y, width, || {
                Line::styled(
                    "Gateway metrics unavailable · r refresh",
                    Style::default().fg(MUTED),
                )
            });
            y += 1;
            c.bottom(x, y, width);
            return y + 1;
        }
        let compaction_all =
            self.usage
                .snapshot
                .total(page.client(), page.provider.as_deref(), None, "compact");
        let data = [
            ("Calls", number(d.daily.calls), number(d.total.calls), ROUTE),
            (
                "Success",
                number(d.daily.success),
                number(d.total.success),
                CONNECTED,
            ),
            (
                "Failed",
                number(d.daily.failed),
                number(d.total.failed),
                if d.daily.failed > 0 { ERROR } else { MUTED },
            ),
            (
                "Stopped",
                number(d.daily.interrupted),
                number(d.total.interrupted),
                if d.daily.interrupted > 0 {
                    WARNING
                } else {
                    MUTED
                },
            ),
            (
                "Pending",
                number(d.daily.pending),
                number(d.total.pending),
                MUTED,
            ),
            (
                "Input",
                known(&d.daily, d.daily.input),
                known(&d.total, d.total.input),
                ENABLED,
            ),
            (
                "Output",
                known(&d.daily, d.daily.output),
                known(&d.total, d.total.output),
                DATA_SECONDARY,
            ),
            (
                "Cache read",
                known(&d.daily, d.daily.cache_read),
                known(&d.total, d.total.cache_read),
                DEFAULT_MODEL,
            ),
            (
                "Cache write",
                known(&d.daily, d.daily.cache_write),
                known(&d.total, d.total.cache_write),
                DEFAULT_MODEL,
            ),
            (
                "Missing usage",
                number(d.daily.unknown),
                number(d.total.unknown),
                if d.daily.unknown > 0 { WARNING } else { MUTED },
            ),
            (
                "Compaction",
                number(d.compact.calls),
                number(compaction_all.calls),
                Color::White,
            ),
        ];
        let name_width = usize::from(width.saturating_sub(2)).saturating_sub(26);
        c.content(x, y, width, || {
            Line::from(vec![
                label(fit("Metric", name_width, false)),
                value(format!(" {}", fit("Range", 12, true)), ROUTE),
                muted(format!(" {}", fit("All time", 12, true))),
            ])
        });
        y += 1;
        for (name, day, all, color) in data {
            if name.width() > name_width || day.width() > 12 || all.width() > 12 {
                for line in wrapped(
                    vec![Line::from(vec![
                        label(format!("{name}  ")),
                        value(day, color),
                        muted(" / "),
                        muted(all),
                    ])],
                    width.saturating_sub(2),
                ) {
                    c.content(x, y, width, || line);
                    y += 1;
                }
                continue;
            }
            c.content(x, y, width, || {
                Line::from(vec![
                    label(fit(name, name_width, false)),
                    value(format!(" {}", fit(&day, 12, true)), color),
                    muted(format!(" {}", fit(&all, 12, true))),
                ])
            });
            y += 1;
        }
        if let Some(since) = self
            .usage
            .snapshot
            .since
            .and_then(|v| chrono::DateTime::from_timestamp(v, 0))
        {
            c.content(x, y, width, || {
                pair("Since (UTC)", since.format("%Y-%m-%d").to_string(), MUTED)
            });
            y += 1;
        }
        c.bottom(x, y, width);
        y + 1
    }
    fn dashboard_sessions(
        &self,
        c: &mut Canvas<'_, '_>,
        x: u16,
        mut y: usize,
        width: u16,
        page: &UsagePage,
        d: &Data<'_>,
    ) -> usize {
        c.header(x, y, width, "LOCAL SESSIONS · 6", Some(5));
        y += 1;
        let scope = if page.session_follow_range && page.range != 3 {
            format!(
                "Last active: {} – {}",
                page.start_day().unwrap_or_else(|| "beginning".into()),
                page.day
            )
        } else {
            "All dates".into()
        };
        for line in wrapped(
            vec![
                Line::from(vec![
                    value(format!("{} sessions", d.sessions.len()), ROUTE),
                    muted(format!(
                        " · {} · {scope}",
                        page.client().unwrap_or("All agents")
                    )),
                ]),
                Line::from(vec![
                    label("Lifetime tokens"),
                    muted(" · separate from gateway"),
                ]),
            ],
            width.saturating_sub(2),
        ) {
            c.content(x, y, width, || line);
            y += 1;
        }
        let available = usize::from(width.saturating_sub(15));
        let mut tail = String::new();
        let mut used = 0;
        for ch in page.session_search.chars().rev() {
            let size = ch.width().unwrap_or(0);
            if used + size > available {
                break;
            }
            tail.push(ch);
            used += size;
        }
        let query: String = tail.chars().rev().collect();
        if let Some(screen_y) = c.visible(y)
            && c.frame.is_some()
        {
            c.state.controls.push((
                Rect::new(x + 1, screen_y, width.saturating_sub(2), 1),
                Action::Search,
            ));
        }
        c.content(x, y, width, || {
            Line::from(vec![
                label("Search [/]  "),
                if page.session_search.is_empty() && !page.session_searching {
                    muted("project / ID / model")
                } else {
                    value(
                        format!("{query}{}", if page.session_searching { "▏" } else { "" }),
                        ROUTE,
                    )
                },
            ])
        });
        y += 1;
        let theme = c.theme;
        let buttons = vec![
            (
                format!(
                    "{} [s]",
                    if page.session_sort_tokens {
                        "Most tokens"
                    } else {
                        "Recent first"
                    }
                ),
                Action::Sort,
                false,
            ),
            (
                format!(
                    "{} [f]",
                    if page.session_follow_range {
                        "Follow period"
                    } else {
                        "All dates"
                    }
                ),
                Action::SessionScope,
                page.session_follow_range,
            ),
            ("Clear filters".into(), Action::SessionClear, false),
            ("Details [Enter]".into(), Action::Activate(5), false),
        ];
        let mut bx = x + 1;
        for (text, action, selected) in buttons {
            let text = fit(&text, usize::from(width.saturating_sub(4)).min(38), false)
                .trim_end()
                .to_owned();
            let bw = (text.width() as u16 + 2).min(width.saturating_sub(2));
            if bx > x + 1 && bx + bw > x + width - 1 {
                c.rail(x, y, width);
                y += 1;
                bx = x + 1;
            }
            if let Some(screen_y) = c.visible(y)
                && c.frame.is_some()
            {
                c.state
                    .controls
                    .push((Rect::new(bx, screen_y, bw, 1), action));
            }
            c.line(bx, y, bw, || {
                control_line(&text, action, selected, false, theme)
            });
            bx += bw;
        }
        c.rail(x, y, width);
        y += 1;
        let hint = if page.session_searching {
            "Type project, ID or model · Enter done · Esc done · Ctrl+U clear"
        } else {
            "Click / n,p select · Enter details · / search ID, project, model"
        };
        for line in wrapped(
            vec![Line::styled(hint, Style::default().fg(MUTED))],
            width.saturating_sub(2),
        ) {
            c.content(x, y, width, || line);
            y += 1;
        }
        let selected = d
            .sessions
            .iter()
            .enumerate()
            .find(|(index, _)| c.selected(5, *index));
        let selected_index = selected.map_or(0, |(index, _)| index);
        let start = selected_index / 8 * 8;
        let end = (start + 8).min(d.sessions.len());
        let split = width >= 100;
        let list_width = if split { width * 3 / 5 } else { width };
        let detail_x = x + list_width;
        let detail_width = width.saturating_sub(list_width);
        let detail_y = y;
        c.content(x, y, list_width, || {
            Line::from(vec![
                label(fit(
                    "Project / session",
                    usize::from(list_width.saturating_sub(2)).saturating_sub(20),
                    false,
                )),
                label(format!(" {}", fit("Tokens", 10, true))),
                label("  Agent"),
            ])
        });
        y += 1;
        if d.sessions.is_empty() {
            for line in wrapped(
                vec![Line::styled(
                    if self.usage.sessions_updated.is_none() && self.usage.sessions.rows.is_empty()
                    {
                        "Reading local sessions…"
                    } else if !page.session_search.is_empty() || page.session_follow_range {
                        "No matches · clear session filters or select All agents"
                    } else if page.client() == Some("Pi") {
                        "Pi local sessions are not supported"
                    } else {
                        "No sessions for this agent"
                    },
                    Style::default().fg(MUTED),
                )],
                list_width.saturating_sub(2),
            ) {
                c.content(x, y, list_width, || line);
                y += 1;
            }
        }
        let zone = chrono::FixedOffset::east_opt(self.usage.snapshot.offset)
            .unwrap_or_else(|| chrono::FixedOffset::east_opt(0).unwrap());
        for (index, s) in d.sessions.iter().enumerate().take(end).skip(start) {
            c.row(x, y, list_width, 5, index, |w| {
                let project = std::path::Path::new(&s.project)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                let title = if project.is_empty() {
                    s.id.chars().take(8).collect::<String>()
                } else {
                    project.into_owned()
                };
                let amount = if s.tokens.known {
                    format!(
                        "{}{}",
                        compact(s.tokens.total()),
                        if s.incomplete { " + ?" } else { "" }
                    )
                } else {
                    "unknown".into()
                };
                Line::from(vec![
                    Span::raw(fit(&title, usize::from(w).saturating_sub(20), false)),
                    value(format!(" {}", fit(&amount, 10, true)), ROUTE),
                    label(format!(" {}", fit(s.client, 8, true))),
                ])
            });
            y += 1;
            let date = chrono::DateTime::from_timestamp(s.updated, 0).map_or("—".into(), |t| {
                t.with_timezone(&zone).format("%m-%d %H:%M").to_string()
            });
            c.content(x, y, list_width, || {
                Line::from(vec![muted(format!(
                    "{} · {}{}{}",
                    s.id.chars().take(8).collect::<String>(),
                    date,
                    if s.child { " · child" } else { "" },
                    if s.fork { " · fork" } else { "" }
                ))])
            });
            // Both identity and timestamp lines select the same session.
            if let Some(screen_y) = c.visible(y)
                && c.frame.is_some()
            {
                c.state.hits.push((
                    Rect::new(x + 1, screen_y, list_width.saturating_sub(2), 1),
                    5,
                    index,
                ));
            }
            y += 1;
        }
        let paging = format!(
            "{}–{} / {}",
            if end == 0 { 0 } else { start + 1 },
            end,
            d.sessions.len()
        );
        c.content(x, y, list_width, || Line::from(vec![muted(paging)]));
        let mut px = x + list_width.saturating_sub(19);
        for (text, action, disabled) in [
            ("Prev", Action::SessionPrevious, start == 0),
            ("Next", Action::SessionNext, end == d.sessions.len()),
        ] {
            if !disabled
                && let Some(screen_y) = c.visible(y)
                && c.frame.is_some()
            {
                c.state
                    .controls
                    .push((Rect::new(px, screen_y, 8, 1), action));
            }
            c.line(px, y, 8, || {
                control_line(text, action, false, disabled, theme)
            });
            px += 8;
        }
        y += 1;
        let mut inspector_y = if split { detail_y } else { y + 1 };
        let (ix, iw) = if split {
            (detail_x, detail_width)
        } else {
            (x, width)
        };
        c.state.session_detail_y = inspector_y;
        c.detail_content(
            ix,
            inspector_y,
            iw,
            value("SELECTED SESSION", FIELD_LABEL).into(),
        );
        inspector_y += 1;
        if let Some((_, session)) = selected {
            for line in session_detail(session, iw.saturating_sub(3)) {
                c.detail_content(ix, inspector_y, iw, line);
                inspector_y += 1;
            }
        }
        let bottom = y.max(inspector_y);
        // Complete both rails even when the list and inspector have different heights.
        for row in y..bottom {
            c.rail(x, row, list_width);
        }
        if split {
            for row in inspector_y..bottom {
                c.rail(detail_x, row, detail_width);
            }
        }
        y = bottom;
        if self.usage.sessions.warnings > 0 {
            for line in wrapped(
                vec![Line::styled(
                    "Some logs unavailable · cached/partial data · r retry",
                    Style::default().fg(WARNING),
                )],
                width.saturating_sub(2),
            ) {
                c.content(x, y, width, || line);
                y += 1;
            }
        }
        c.bottom(x, y, width);
        y + 1
    }
}

fn control_rows(inner: Rect, page: &UsagePage) -> (Vec<(String, Action, Rect)>, u16) {
    // Actions have their own row; labels never masquerade as buttons.
    let mut groups: Vec<Vec<(String, Action)>> = vec![
        vec![
            ("Refresh [r]".into(), Action::Refresh),
            ("Reset [x]".into(), Action::Reset),
            ("Back [Esc]".into(), Action::Close),
        ],
        std::iter::once(("Agent".into(), Action::DateLabel))
            .chain(
                ["All", "Claude", "Codex", "Pi", "Grok"]
                    .into_iter()
                    .enumerate()
                    .map(|(i, name)| (name.into(), Action::Client(i))),
            )
            .collect(),
        vec![
            ("Period".into(), Action::DateLabel),
            ("Day [d]".into(), Action::Range(0)),
            ("7d [w]".into(), Action::Range(1)),
            ("30d [m]".into(), Action::Range(2)),
            ("All [y]".into(), Action::Range(3)),
        ],
    ];
    if inner.width >= 116 {
        groups[0].push(("Quit [q]".into(), Action::Quit));
    }
    let window = match page.start_day() {
        None => "All dates".into(),
        Some(start) if start == page.day => page.day.clone(),
        Some(start) if inner.width < 60 && start.get(..4) == page.day.get(..4) => {
            format!("{} {}–{}", &page.day[..4], &start[5..], &page.day[5..])
        }
        Some(start) => format!("{start} – {}", page.day),
    };
    let mut dates = vec![];
    if inner.width >= 60 {
        dates.push(("Window".into(), Action::DateLabel));
    }
    dates.push((window, Action::DateLabel));
    if page.range != 3 {
        dates.extend([
            (
                if inner.width >= 60 { "‹ day" } else { "‹" }.into(),
                Action::Previous,
            ),
            (
                if inner.width >= 60 { "day ›" } else { "›" }.into(),
                Action::Next,
            ),
            ("Today [t]".into(), Action::Today),
        ]);
    }
    groups.push(dates);
    if let Some(provider) = &page.provider {
        groups.push(vec![
            ("Clear [a]".into(), Action::Clear),
            (
                format!("Gateway: {} / {provider}", page.client().unwrap_or("All")),
                Action::DateLabel,
            ),
        ]);
    } else if inner.width >= 100 {
        groups
            .last_mut()
            .unwrap()
            .push(("All providers".into(), Action::DateLabel));
    }
    if inner.width < 60 {
        for group in &mut groups {
            for (text, _) in group {
                *text = text.replace(" [", "·").replace(']', "");
            }
        }
    }
    let mut out = vec![];
    let mut y = inner.y;
    for (index, group) in groups.into_iter().enumerate() {
        let mut x = if index == 0 && inner.width >= 60 {
            inner.right().saturating_sub(
                group
                    .iter()
                    .map(|(text, _)| text.width() as u16 + 2)
                    .sum::<u16>(),
            )
        } else {
            inner.x
        };
        for (text, action) in group {
            let width = (text.width() as u16 + 2).min(inner.width);
            if x > inner.x && x + width > inner.right() {
                y += 1;
                x = inner.x;
            }
            out.push((text, action, Rect::new(x, y, width, 1)));
            x += width;
        }
        y += 1;
    }
    (out, y.saturating_sub(inner.y))
}

fn control_line(
    text: &str,
    action: Action,
    selected: bool,
    disabled: bool,
    theme: theme::Theme,
) -> Line<'static> {
    if matches!(action, Action::DateLabel) {
        return Line::from(vec![if text.starts_with(|ch: char| ch.is_ascii_digit())
            || text == "All dates"
        {
            value(text.to_owned(), DATA_SECONDARY)
        } else {
            label(text.to_owned())
        }]);
    }
    let color = if disabled {
        MUTED
    } else if selected {
        ROUTE
    } else {
        match action {
            Action::Client(_) | Action::Previous | Action::Next => Color::White,
            Action::Refresh => CONNECTED,
            Action::Reset | Action::SessionClear => MUTED,
            Action::Range(_) | Action::Close => FIELD_LABEL,
            _ => DATA_SECONDARY,
        }
    };
    let mut style = Style::default().fg(color);
    if selected {
        style = style
            .add_modifier(Modifier::BOLD)
            .bg(theme::PROVIDER_SELECTION);
    }
    let (name, shortcut) = text
        .split_once(" [")
        .map_or((text, ""), |(name, key)| (name, key));
    let marker = if selected {
        theme.selection_symbol()
    } else {
        ""
    };
    let mut spans = vec![Span::styled(format!("{marker}{name}"), style)];
    if !shortcut.is_empty() {
        spans.push(muted(format!(" [{}", shortcut)));
    }
    Line::from(spans).style(if selected { style } else { Style::default() })
}

impl App {
    pub(in crate::tui) fn draw_usage(
        &self,
        frame: &mut ratatui::Frame,
        area: Rect,
        page: &UsagePage,
    ) {
        let title = page
            .provider
            .as_ref()
            .map_or(" Usage ".into(), |provider| format!(" Usage / {provider} "));
        frame.render_widget(panel(&title, true), area);
        let mut inner = panel_inner(area);
        if inner.width >= 60 {
            inner.x += 1;
            inner.width = inner.width.saturating_sub(2);
        }
        let (controls, height) = control_rows(inner, page);
        if inner.width >= 60 {
            let action_x = controls
                .iter()
                .filter(|(_, _, rect)| rect.y == inner.y)
                .map(|(_, _, rect)| rect.x)
                .min()
                .unwrap_or(inner.right());
            frame.render_widget(
                Paragraph::new(Line::from(vec![value("GATEWAY FILTERS", FIELD_LABEL)])),
                Rect::new(inner.x, inner.y, action_x.saturating_sub(inner.x + 2), 1),
            );
        }
        for (text, action, rect) in &controls {
            let selected = match action {
                Action::Client(client) => page.client == *client,
                Action::Range(range) => page.range == *range,
                _ => false,
            };
            let disabled = (page.range == 3 && matches!(action, Action::Previous | Action::Next))
                || (page.day >= self.usage.snapshot.today() && matches!(action, Action::Next));
            frame.render_widget(
                Paragraph::new(control_line(text, *action, selected, disabled, self.theme))
                    .alignment(Alignment::Center),
                *rect,
            );
        }
        let footer = Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1);
        let viewport = Rect::new(
            inner.x,
            inner.y + height,
            inner.width.saturating_sub(1),
            inner.height.saturating_sub(height + 1),
        );
        let d = Data {
            providers: self.usage_providers(page),
            models: self.usage_models(page),
            history: self.usage_history(page),
            sessions: self.session_rows(page),
            daily: page.range_total(
                &self.usage.snapshot,
                page.client(),
                page.provider.as_deref(),
                "generation",
            ),
            total: self.usage.snapshot.total(
                page.client(),
                page.provider.as_deref(),
                None,
                "generation",
            ),
            compact: page.range_total(
                &self.usage.snapshot,
                page.client(),
                page.provider.as_deref(),
                "compact",
            ),
        };
        let mut state = page.dashboard.borrow_mut();
        state.viewport = viewport;
        state.controls = controls.into_iter().map(|(_, a, r)| (r, a)).collect();
        let mut c = Canvas {
            frame: None,
            viewport,
            scroll: page.scroll,
            theme: self.theme,
            focus: page.section,
            state: &mut state,
        };
        let content_height = self.dashboard_content(&mut c, page, &d);
        c.state.total_height = content_height;
        let limit = content_height.saturating_sub(usize::from(viewport.height));
        page.limit.set(limit);
        let mut scroll = page.scroll.min(limit);
        if let Some(section) = c.state.jump.take() {
            scroll = c.state.anchors[section].min(limit);
        }
        if let Some((section, key)) = c.state.reveal.take()
            && let Some((_, y)) = c.state.rows[section].iter().find(|(id, _)| *id == key)
        {
            scroll = y
                .saturating_sub(usize::from(viewport.height.saturating_sub(1)).min(2))
                .min(limit);
        }
        if c.state.inspect_session {
            scroll = c.state.session_detail_y.min(limit);
            c.state.inspect_session = false;
        }
        // Save the effective offset through interior state; key/mouse events read it before scrolling.
        c.state.effective_scroll = scroll;
        c.scroll = scroll;
        c.frame = Some(frame);
        self.dashboard_content(&mut c, page, &d);
        drop(c);
        let status = if self.usage.error.is_some() {
            "STALE · cached gateway data · r retry".into()
        } else if self.usage.updated.is_none() {
            if self.usage.snapshot.rows.is_empty() {
                "Reading gateway · local sessions independent".into()
            } else {
                "Refreshing gateway · showing cached data".into()
            }
        } else {
            let refreshed = self.usage.updated.map_or("—".into(), |updated| {
                let zone = chrono::FixedOffset::east_opt(self.usage.snapshot.offset)
                    .unwrap_or_else(|| chrono::FixedOffset::east_opt(0).unwrap());
                (chrono::Utc::now()
                    - chrono::Duration::from_std(updated.elapsed()).unwrap_or_default())
                .with_timezone(&zone)
                .format("%H:%M:%S UTC%:z")
                .to_string()
            });
            if inner.width >= 100 {
                format!(
                    "↑↓ scroll · 1–6 / Tab sections · n/p select · Enter filter · / sessions · {refreshed}",
                )
            } else if inner.width >= 60 {
                format!(
                    "↑↓ scroll · Tab sections · n/p select · / search · Esc back / {}s",
                    self.config.usage_refresh_secs
                )
            } else {
                "↑↓ scroll · Tab jump · Esc back".into()
            }
        };
        frame.render_widget(
            Paragraph::new(status).style(Style::default().fg(if self.usage.error.is_some() {
                WARNING
            } else {
                MUTED
            })),
            footer,
        );
        if limit > 0 && viewport.height > 0 {
            let mut scrollbar_state = ScrollbarState::new(content_height)
                .position(scroll)
                .viewport_content_length(usize::from(viewport.height));
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None)
                    .style(Style::default().fg(MUTED)),
                Rect::new(viewport.right(), viewport.y, 1, viewport.height),
                &mut scrollbar_state,
            );
        }
    }
    pub(in crate::tui) fn usage_enter(&self, page: &mut UsagePage, key: KeyEvent) {
        if page.session_searching || key.code != KeyCode::Enter {
            return;
        }
        let index = {
            let state = page.dashboard.borrow();
            state.rows[page.section]
                .iter()
                .position(|(id, _)| Some(id) == state.selected[page.section].as_ref())
                .unwrap_or(0)
        };
        if page.section == 5 {
            if !self.session_rows(page).is_empty() {
                page.dashboard.borrow_mut().inspect_session = true;
            }
        } else if page.section == 0 {
            if let Some(p) = self.usage_providers(page).get(index) {
                page.provider = Some(p.id.clone());
                page.client = match p.client.as_str() {
                    "Claude" => 1,
                    "Codex" => 2,
                    "Pi" => 3,
                    "Grok" => 4,
                    _ => 0,
                };
                page.section = 3;
                page.jump_to_section();
            }
        } else if page.section == 1
            && let Some((day, _)) = self.usage_history(page).get(index)
        {
            page.day = day.clone();
            page.range = 0;
            page.follow_today = page.day == self.usage.snapshot.today();
            page.section = 4;
            page.jump_to_section();
        }
    }
    pub(in crate::tui) fn usage_mouse(&mut self, mouse: MouseEvent, _screen: Rect) -> MouseAction {
        let Some(mut page) = self.usage.page.take() else {
            return MouseAction::None;
        };
        page.scroll = page.dashboard.borrow().effective_scroll;
        let mut close = false;
        let mut quit = false;
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                page.scroll = if mouse.kind == MouseEventKind::ScrollUp {
                    page.scroll.saturating_sub(1)
                } else {
                    page.scroll.saturating_add(1).min(page.limit.get())
                };
            }
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) => {
                let state = page.dashboard.borrow();
                let action = state
                    .controls
                    .iter()
                    .find(|(r, _)| contains(*r, mouse.column, mouse.row))
                    .map(|(_, a)| *a);
                let hit = state
                    .hits
                    .iter()
                    .find(|(r, _, _)| contains(*r, mouse.column, mouse.row))
                    .map(|(_, s, i)| (*s, *i));
                let viewport = state.viewport;
                drop(state);
                if let Some(action) = action {
                    if mouse.kind == MouseEventKind::Drag(MouseButton::Left) {
                        self.usage.page = Some(page);
                        return MouseAction::None;
                    }
                    if !matches!(action, Action::Search | Action::DateLabel) {
                        page.session_searching = false;
                    }
                    match action {
                        Action::Activate(section) => {
                            page.section = section;
                            self.usage_enter(
                                &mut page,
                                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                            );
                        }
                        Action::DateLabel => {}
                        Action::Reset => {
                            page.key(
                                KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
                                &self.usage.snapshot,
                            );
                        }
                        Action::Search => {
                            page.session_searching = true;
                            page.section = 5;
                            page.jump_to_section();
                        }
                        Action::SessionScope => {
                            page.session_follow_range = !page.session_follow_range;
                            page.section = 5;
                            page.jump_to_section();
                        }
                        Action::SessionClear => {
                            page.session_search.clear();
                            page.session_searching = false;
                            page.session_follow_range = false;
                            page.section = 5;
                            page.jump_to_section();
                        }
                        Action::SessionPrevious | Action::SessionNext => {
                            page.section = 5;
                            for _ in 0..8 {
                                page.select_table_row(matches!(action, Action::SessionNext));
                            }
                            page.jump_to_section();
                        }
                        Action::Sort => page.session_sort_tokens = !page.session_sort_tokens,
                        Action::Client(client) => {
                            page.client = client;
                            page.provider = None;
                            page.scroll = 0;
                        }
                        Action::Range(range) => {
                            page.range = range;
                            page.scroll = 0;
                        }
                        Action::Clear => {
                            page.provider = None;
                            page.scroll = 0;
                        }
                        Action::Refresh => {
                            self.usage.updated = None;
                            self.usage.sessions_updated = None;
                        }
                        Action::Close => close = true,
                        Action::Quit => quit = true,
                        Action::Previous | Action::Next | Action::Today => {
                            page.key(
                                KeyEvent::new(
                                    match action {
                                        Action::Previous => KeyCode::Left,
                                        Action::Next => KeyCode::Right,
                                        _ => KeyCode::Char('t'),
                                    },
                                    KeyModifiers::NONE,
                                ),
                                &self.usage.snapshot,
                            );
                        }
                    }
                } else if mouse.column == viewport.right()
                    && mouse.row >= viewport.y
                    && mouse.row < viewport.bottom()
                {
                    page.scroll = usize::from(mouse.row - viewport.y) * page.limit.get()
                        / usize::from(viewport.height.saturating_sub(1).max(1));
                } else if let Some((section, index)) = hit {
                    let key = page.dashboard.borrow().rows[section][index].0.clone();
                    let mut state = page.dashboard.borrow_mut();
                    state.selected[section] = Some(key);
                    // Clicking a visible row expands details in place. Only explicit
                    // section navigation and keyboard actions request scrolling.
                    state.reveal = None;
                    state.jump = None;
                    state.inspect_session = false;
                    drop(state);
                    page.section = section;
                }
            }
            _ => {}
        }
        self.usage.active = !close;
        self.usage.page = Some(page);
        if quit {
            MouseAction::Quit
        } else {
            MouseAction::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    fn fixture() -> (tempfile::TempDir, App) {
        let (temp, mut app) = crate::tui::tests::persisted_app();
        app.open_usage();
        app.usage.updated = Some(std::time::Instant::now());
        app.usage.sessions_updated = Some(std::time::Instant::now());
        app.usage.snapshot.rows = vec![crate::usage::Row {
            hour: 1,
            model: "deepseek/deepseek-v4.1-flash[1m]".into(),
            day: app.usage.snapshot.today(),
            client: "Claude".into(),
            provider: "fixture".into(),
            name: "Fixture gateway".into(),
            kind: "generation".into(),
            totals: Totals {
                calls: 12,
                success: 10,
                failed: 1,
                interrupted: 1,
                input: 9000,
                output: 3000,
                cache_read: 6000,
                cache_write: 1000,
                cache_input: 8000,
                cache_hits: 4000,
                speed_output: 1000,
                speed_ms: 2000,
                speed_samples: 2,
                ..Default::default()
            },
        }];
        app.usage.sessions.rows = vec![
            crate::sessions::Session {
                client: "Claude",
                id: "local-session-A".into(),
                project: "/work/project-one".into(),
                updated: chrono::Utc::now().timestamp(),
                tokens: crate::sessions::Tokens {
                    input: 1000,
                    output: 20,
                    read: 500,
                    known: true,
                    cache_known: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            crate::sessions::Session {
                client: "Codex",
                id: "local-session-B".into(),
                updated: chrono::Utc::now().timestamp() - 10,
                tokens: crate::sessions::Tokens {
                    input: 10000,
                    output: 20,
                    known: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        ];
        (temp, app)
    }
    fn key(app: &mut App, code: KeyCode) {
        app.usage_key(KeyEvent::new(code, KeyModifiers::NONE));
    }
    fn render(app: &mut App, width: u16, height: u16, suffix: &str) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        if let Ok(dir) = std::env::var("CCSW_UI_PREVIEW_DIR") {
            std::fs::create_dir_all(&dir).unwrap();
            let cells: Vec<_> = buffer.content.iter().map(|c| serde_json::json!({"text": c.symbol(), "fg": format!("{:?}", c.fg), "bg": format!("{:?}", c.bg), "bold": c.modifier.contains(Modifier::BOLD), "underline": c.modifier.contains(Modifier::UNDERLINED)})).collect();
            std::fs::write(
                std::path::Path::new(&dir).join(format!(
                    "Dashboard-{:?}-{width}-{height}-{suffix}.json",
                    app.theme
                )),
                serde_json::to_vec(
                    &serde_json::json!({"width":width,"height":height,"cells":cells}),
                )
                .unwrap(),
            )
            .unwrap();
        }
        buffer
    }
    fn text(buffer: &ratatui::buffer::Buffer) -> String {
        buffer.content.iter().map(|c| c.symbol()).collect()
    }
    #[test]
    fn token_summary_prioritizes_total_and_shows_input_output_mix() {
        let totals = Totals {
            calls: 12,
            input: 9000,
            output: 3000,
            cache_input: 8000,
            cache_hits: 4000,
            speed_output: 1000,
            speed_ms: 2000,
            speed_samples: 2,
            ..Default::default()
        };
        let lines = gateway_lines(&totals, &totals, "1 day", 60, true, false, false, false);
        let text: String = lines.iter().map(|l| l.to_string()).collect();
        assert!(text.contains("12,000"));
        assert!(text.contains("9,000"));
        assert!(text.contains("3,000"));
        assert!(text.contains("50.0%"));
        assert!(text.contains("500.0 tok/s"));
        assert_eq!(io_bar(9000, 3000, 40).spans[0].content.chars().count(), 30);
        assert_eq!(io_bar(9000, 3000, 40).spans[1].content.chars().count(), 10);
        let unknown = Totals {
            calls: 2,
            unknown: 2,
            ..Default::default()
        };
        let lines = gateway_lines(&unknown, &unknown, "1 day", 30, false, false, false, false);
        let text: String = lines.iter().map(|l| l.to_string()).collect();
        assert!(text.contains("unknown"));
        assert!(!text.contains("0 TOKENS"));
        assert!(
            health_lines(&Totals::default(), &Totals::default(), 40, false)[0]
                .to_string()
                .contains("no samples")
        );
    }
    #[test]
    fn sessions_filter_sort_render_and_mouse_without_proxy_data() {
        let (_temp, mut app) = fixture();
        app.usage.page.as_mut().unwrap().provider = Some("unrelated-gateway".into());
        app.usage.updated = None;
        render(&mut app, 80, 24, "loading");
        key(&mut app, KeyCode::Char('6'));
        assert!(app.usage.page.as_ref().unwrap().provider.is_some());
        let before = render(&mut app, 80, 24, "sessions");
        assert!(text(&before).contains("local-se"));
        assert!(text(&before).contains("1,020"));
        let selected = app.usage.page.as_ref().unwrap().dashboard.borrow().selected[5].clone();
        key(&mut app, KeyCode::Char('s'));
        render(&mut app, 80, 24, "sessions-sorted");
        assert_eq!(
            app.usage.page.as_ref().unwrap().dashboard.borrow().selected[5],
            selected
        );
        assert_eq!(
            app.session_rows(app.usage.page.as_ref().unwrap())[0].id,
            "local-session-B"
        );
        app.usage.page.as_mut().unwrap().client = 4;
        assert!(
            app.session_rows(app.usage.page.as_ref().unwrap())
                .is_empty()
        );
        app.usage.page.as_mut().unwrap().client = 1;
        assert_eq!(app.session_rows(app.usage.page.as_ref().unwrap()).len(), 1);
    }
    #[test]
    fn usage_drilldown_dates_and_mouse_targets_are_consistent() {
        let (_temp, mut app) = fixture();
        render(&mut app, 80, 24, "top");
        key(&mut app, KeyCode::Char('1'));
        render(&mut app, 80, 24, "providers");
        let hit = app
            .usage
            .page
            .as_ref()
            .unwrap()
            .dashboard
            .borrow()
            .hits
            .iter()
            .find(|(_, section, index)| *section == 0 && *index == 0)
            .unwrap()
            .0;
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.x + 2,
            row: hit.y,
            modifiers: KeyModifiers::NONE,
        };
        app.usage_mouse(click, Rect::new(0, 0, 80, 24));
        assert!(app.usage.page.as_ref().unwrap().provider.is_none());
        render(&mut app, 80, 24, "selected");
        key(&mut app, KeyCode::Enter);
        assert_eq!(
            app.usage.page.as_ref().unwrap().provider.as_deref(),
            Some("fixture")
        );
        let model = render(&mut app, 80, 24, "models");
        assert!(text(&model).contains("MODELS"));
        key(&mut app, KeyCode::Char('a'));
        assert!(app.usage.active);
        key(&mut app, KeyCode::Char('5'));
        render(&mut app, 120, 36, "chart");
        let day = app.usage.page.as_ref().unwrap().day.clone();
        key(&mut app, KeyCode::Right);
        assert_eq!(app.usage.page.as_ref().unwrap().day, day);
        key(&mut app, KeyCode::Char(']'));
        assert_eq!(app.usage.page.as_ref().unwrap().client, 2);
        key(&mut app, KeyCode::Esc);
        assert!(!app.usage.active);
    }
    #[test]
    fn all_usage_button_rows_share_spacing_and_fit() {
        for theme in theme::Theme::ALL {
            let (_temp, mut app) = fixture();
            app.theme = theme;
            for (width, height) in [(40, 12), (80, 24), (120, 36), (160, 48)] {
                key(&mut app, KeyCode::Home);
                render(&mut app, width, height, "top");
                {
                    let page = app.usage.page.as_ref().unwrap();
                    let state = page.dashboard.borrow();
                    for (rect, _) in &state.controls {
                        assert!(rect.right() <= width && rect.bottom() <= height);
                    }
                    assert_eq!(
                        state
                            .controls
                            .iter()
                            .filter(|(_, a)| matches!(a, Action::Client(_)))
                            .count(),
                        5
                    );
                    assert_eq!(
                        state
                            .controls
                            .iter()
                            .filter(|(_, a)| matches!(a, Action::Range(_)))
                            .count(),
                        4
                    );
                    assert!(state.viewport.height > 0);
                }
                for section in ['1', '2', '3', '4', '5', '6'] {
                    key(&mut app, KeyCode::Char(section));
                    let buffer = render(&mut app, width, height, &section.to_string());
                    assert!(!buffer.content.iter().any(|c| matches!(
                        c.fg,
                        DEFAULT_MODEL
                            | ENABLED
                            | DATA_SECONDARY
                            | FIELD_LABEL
                            | theme::EDGE
                            | theme::ACTIVE_EDGE
                    )));
                    if theme.terminal_background() {
                        assert!(buffer.content.iter().all(|c| c.bg == Color::Reset));
                        assert!(
                            buffer
                                .content
                                .iter()
                                .filter(|c| c.symbol() == "▶")
                                .all(|c| !c.modifier.contains(Modifier::UNDERLINED))
                        );
                    } else {
                        assert!(!buffer.content.iter().any(|c| c.symbol() == "▶"));
                    }
                    if section == '6' {
                        app.usage_key(KeyEvent::new(KeyCode::Down, KeyModifiers::ALT));
                        render(&mut app, width, height, "session-row");
                        assert!(
                            app.usage
                                .page
                                .as_ref()
                                .unwrap()
                                .dashboard
                                .borrow()
                                .hits
                                .iter()
                                .any(|(_, section, index)| *section == 5 && *index == 1)
                        );
                    }
                }
            }
        }
    }
}
