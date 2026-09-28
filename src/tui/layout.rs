use super::*;

pub(super) fn workspace_content_area(area: Rect) -> Rect {
    Rect::new(
        area.x,
        area.y.saturating_add(1),
        area.width,
        area.height.saturating_sub(1),
    )
}

pub(super) fn settings_page_area(screen: Rect) -> Rect {
    workspace_content_area(screen)
}

pub(super) fn page_header_actions(area: Rect) -> [(Rect, Rect); 1] {
    let back_width = if area.width >= 56 { 17 } else { 11 };
    let help_width = 10;
    let back = Rect::new(
        area.right().saturating_sub(back_width + 2),
        area.y,
        back_width,
        1,
    );
    let help = Rect::new(back.x.saturating_sub(help_width + 1), area.y, help_width, 1);
    [(help, back)]
}

pub(super) fn settings_proxy_button(area: Rect) -> Rect {
    if area.width < 90 {
        return Rect::new(area.x.saturating_add(8), area.y, 7, 1);
    }
    Rect::new(
        area.right().saturating_sub(60),
        area.y,
        14.min(area.width),
        1,
    )
}

pub(super) fn settings_appearance_button(area: Rect) -> Rect {
    if area.width < 90 {
        return Rect::new(area.x.saturating_add(1), area.y, 6, 1);
    }
    Rect::new(
        area.right().saturating_sub(76),
        area.y,
        15.min(area.width),
        1,
    )
}

pub(super) fn provider_workspace(area: Rect) -> bool {
    area.width >= 120 && area.height >= 24
}

#[cfg(test)]
pub(super) fn ui_areas(area: Rect, focus: Focus, view_mode: ViewMode) -> UiAreas {
    let rows = app_rows(area);
    ui_content_areas(area, rows[1], rows[2], focus, view_mode)
}

pub(super) fn ui_content_areas(
    area: Rect,
    content: Rect,
    status: Rect,
    focus: Focus,
    view_mode: ViewMode,
) -> UiAreas {
    if provider_workspace(area) {
        let columns = Layout::horizontal([
            Constraint::Length((area.width / 4).clamp(30, 42)),
            Constraint::Length(1),
            Constraint::Min(50),
        ])
        .split(content);
        let right = Layout::horizontal([
            Constraint::Ratio(55, 100),
            Constraint::Length(1),
            Constraint::Ratio(45, 100),
        ])
        .split(columns[2]);
        return UiAreas {
            profiles: Some(columns[0]),
            models: match view_mode {
                ViewMode::Home => None,
                ViewMode::Provider => Some(right[0]),
                ViewMode::AllEnabled => Some(right[0]),
            },
            details: match view_mode {
                ViewMode::Home => Some(columns[2]),
                ViewMode::Provider => Some(right[2]),
                ViewMode::AllEnabled => Some(right[2]),
            },
            footer: status,
        };
    }
    match view_mode {
        ViewMode::Home => UiAreas {
            profiles: Some(content),
            models: None,
            details: None,
            footer: status,
        },
        ViewMode::Provider => {
            if area.width >= 100 {
                let cols = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Ratio(58, 100),
                        Constraint::Length(1),
                        Constraint::Ratio(42, 100),
                    ])
                    .split(content);
                UiAreas {
                    profiles: None,
                    models: Some(cols[0]),
                    details: Some(cols[2]),
                    footer: status,
                }
            } else {
                UiAreas {
                    profiles: None,
                    models: (focus != Focus::Details).then_some(content),
                    details: (focus == Focus::Details).then_some(content),
                    footer: status,
                }
            }
        }
        ViewMode::AllEnabled => UiAreas {
            profiles: None,
            models: Some(content),
            details: None,
            footer: status,
        },
    }
}

pub(super) struct ProviderPageLayout {
    pub(super) shell: Rect,
    pub(super) controls: Vec<(FooterControl, Rect)>,
    pub(super) content: Rect,
    pub(super) status: Rect,
}

impl App {
    pub(super) fn provider_page_layout(&self, area: Rect) -> ProviderPageLayout {
        let shell = Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(1),
        );
        let mut inner = panel_inner(shell);
        if inner.width >= 60 {
            inner.x += 1;
            inner.width = inner.width.saturating_sub(2);
        }
        let actions = [
            if self.focus == Focus::Profiles || self.view_mode == ViewMode::Home {
                FooterControl::AddProfile
            } else {
                FooterControl::AddModel
            },
            FooterControl::Sync,
            FooterControl::Proxy,
            FooterControl::Disconnect,
        ]
        .into_iter()
        .filter(|control| {
            (*control != FooterControl::Disconnect || !self.pi_enabled)
                && (*control != FooterControl::Proxy || self.pi_enabled)
        });
        let mut rows: Vec<Vec<(FooterControl, u16)>> = vec![vec![]];
        let mut used = 0;
        let reserve = (UnicodeWidthStr::width("Help [?]")
            + UnicodeWidthStr::width("Back [Esc/q]")
            + 7) as u16;
        for control in actions {
            let width =
                (UnicodeWidthStr::width(self.provider_action_text(control).as_str()) as u16 + 2)
                    .min(inner.width);
            if used > 0 && used + width + 1 + reserve > inner.width {
                rows.push(vec![]);
                used = 0;
            }
            if used > 0 {
                used += 1;
            }
            rows.last_mut().unwrap().push((control, width));
            used += width;
        }
        if used > 0 && used + reserve > inner.width {
            rows.push(vec![]);
            used = 0;
        }
        for control in [FooterControl::Help, FooterControl::Back] {
            let width =
                (UnicodeWidthStr::width(self.provider_action_text(control).as_str()) as u16 + 2)
                    .min(inner.width);
            if used > 0 && used + width + 1 > inner.width {
                rows.push(vec![]);
                used = 0;
            }
            if used > 0 {
                used += 1;
            }
            rows.last_mut().unwrap().push((control, width));
            used += width;
        }
        let header_height = rows.len() as u16;
        let mut controls = vec![];
        for (index, row) in rows.into_iter().enumerate() {
            let width = row.iter().map(|(_, width)| *width).sum::<u16>()
                + row.len().saturating_sub(1) as u16;
            let mut x = inner.right().saturating_sub(width);
            for (control, width) in row {
                controls.push((control, Rect::new(x, inner.y + index as u16, width, 1)));
                x += width + 1;
            }
        }
        let content = Rect::new(
            inner.x,
            inner.y + header_height,
            inner.width,
            inner.height.saturating_sub(header_height + 1),
        );
        let status = Rect::new(
            inner.x,
            inner.bottom().saturating_sub(1),
            inner.width,
            u16::from(inner.height > 0),
        );
        ProviderPageLayout {
            shell,
            controls,
            content,
            status,
        }
    }

    pub(super) fn provider_ui_areas(&self, area: Rect) -> UiAreas {
        let page = self.provider_page_layout(area);
        ui_content_areas(area, page.content, page.status, self.focus, self.view_mode)
    }
}

#[cfg(test)]
pub(super) fn app_rows(area: Rect) -> [Rect; 3] {
    let edge_height = if area.height >= 14 {
        3
    } else if area.height >= 8 {
        2
    } else {
        1
    }
    .min(area.height / 2);
    let content_height = area.height.saturating_sub(edge_height.saturating_mul(2));
    [
        Rect::new(area.x, area.y, area.width, edge_height),
        Rect::new(
            area.x,
            area.y.saturating_add(edge_height),
            area.width,
            content_height,
        ),
        Rect::new(
            area.x,
            area.y
                .saturating_add(edge_height)
                .saturating_add(content_height),
            area.width,
            edge_height,
        ),
    ]
}

#[cfg(test)]
pub(super) fn footer_controls(
    area: Rect,
    compact: bool,
    view_mode: ViewMode,
) -> Vec<(FooterControl, Rect)> {
    if area.width == 0 || area.height == 0 {
        return vec![];
    }
    pub(super) const HOME_WIDE: &[(FooterControl, u16)] = &[
        (FooterControl::AddProfile, 13),
        (FooterControl::Sync, 14),
        (FooterControl::Proxy, 10),
        (FooterControl::Settings, 8),
        (FooterControl::Help, 8),
        (FooterControl::DeleteProfile, 12),
        (FooterControl::Quit, 8),
    ];
    pub(super) const HOME_COMPACT: &[(FooterControl, u16)] = &[
        (FooterControl::AddProfile, 9),
        (FooterControl::Sync, 8),
        (FooterControl::Proxy, 7),
        (FooterControl::Settings, 8),
        (FooterControl::Help, 5),
        (FooterControl::DeleteProfile, 7),
        (FooterControl::Quit, 5),
    ];
    pub(super) const PROVIDER_WIDE: &[(FooterControl, u16)] = &[
        (FooterControl::Back, 14),
        (FooterControl::Models, 11),
        (FooterControl::Details, 11),
        (FooterControl::Sync, 14),
        (FooterControl::Proxy, 10),
        (FooterControl::Settings, 8),
        (FooterControl::Help, 8),
    ];
    pub(super) const PROVIDER_COMPACT: &[(FooterControl, u16)] = &[
        (FooterControl::Back, 8),
        (FooterControl::Models, 8),
        (FooterControl::Details, 8),
        (FooterControl::Sync, 8),
        (FooterControl::Proxy, 7),
        (FooterControl::Settings, 8),
        (FooterControl::Help, 5),
    ];
    pub(super) const HOME_TINY: &[(FooterControl, u16)] = &[
        (FooterControl::AddProfile, 7),
        (FooterControl::Sync, 6),
        (FooterControl::Proxy, 5),
        (FooterControl::Settings, 4),
        (FooterControl::Help, 3),
        (FooterControl::DeleteProfile, 7),
        (FooterControl::Quit, 3),
    ];
    pub(super) const PROVIDER_TINY: &[(FooterControl, u16)] = &[
        (FooterControl::Back, 6),
        (FooterControl::Models, 5),
        (FooterControl::Details, 5),
        (FooterControl::Sync, 5),
        (FooterControl::Proxy, 5),
        (FooterControl::Settings, 4),
        (FooterControl::Help, 3),
    ];
    pub(super) const ALL_WIDE: &[(FooterControl, u16)] = &[
        (FooterControl::Back, 14),
        (FooterControl::Sync, 14),
        (FooterControl::Proxy, 10),
        (FooterControl::Settings, 8),
        (FooterControl::Help, 8),
    ];
    pub(super) const ALL_COMPACT: &[(FooterControl, u16)] = &[
        (FooterControl::Back, 8),
        (FooterControl::Sync, 8),
        (FooterControl::Proxy, 7),
        (FooterControl::Settings, 8),
        (FooterControl::Help, 5),
    ];
    pub(super) const ALL_TINY: &[(FooterControl, u16)] = &[
        (FooterControl::Back, 6),
        (FooterControl::Sync, 5),
        (FooterControl::Proxy, 5),
        (FooterControl::Settings, 4),
        (FooterControl::Help, 3),
    ];
    let specs = match (view_mode, compact, area.width < 55) {
        (ViewMode::Home, _, true) => HOME_TINY,
        (ViewMode::Provider, _, true) => PROVIDER_TINY,
        (ViewMode::AllEnabled, _, true) => ALL_TINY,
        (ViewMode::Home, false, false) => HOME_WIDE,
        (ViewMode::Home, true, false) => HOME_COMPACT,
        (ViewMode::Provider, false, false) => PROVIDER_WIDE,
        (ViewMode::Provider, true, false) => PROVIDER_COMPACT,
        (ViewMode::AllEnabled, false, false) => ALL_WIDE,
        (ViewMode::AllEnabled, true, false) => ALL_COMPACT,
    };
    let right = area.x.saturating_add(area.width);
    let mut x = area.x;
    specs
        .iter()
        .filter_map(|(control, width)| {
            if x.saturating_add(*width) > right {
                return None;
            }
            let rect = Rect {
                x,
                y: area.y,
                width: *width,
                height: 1,
            };
            x = x
                .saturating_add(*width)
                .saturating_add(u16::from(area.width >= 55));
            Some((*control, rect))
        })
        .collect()
}

pub(super) fn provider_detail_cards(area: Rect) -> (Rect, Rect) {
    let showcase_height = if area.width < 38 && area.height >= 19 {
        13
    } else if area.height >= 24 {
        12
    } else if area.height >= 16 {
        9
    } else {
        area.height / 2
    };
    let cards = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(showcase_height), Constraint::Min(6)])
        .split(area);
    (cards[0], cards[1])
}

pub(super) fn showcase_controls(area: Rect, pi: bool) -> Vec<(ShowcaseControl, Rect)> {
    let inner = panel_inner(area);
    if inner.height < 2 || inner.width < 8 {
        return vec![];
    }
    let controls = [
        ShowcaseControl::Toggle,
        ShowcaseControl::Test,
        ShowcaseControl::Default,
        ShowcaseControl::OneM,
        ShowcaseControl::Delete,
    ]
    .into_iter()
    .filter(|control| !pi || *control != ShowcaseControl::Toggle)
    .collect::<Vec<_>>();
    if inner.width >= 38 && inner.height >= 3 {
        let width = inner.width / 2;
        let start_y = inner.y + inner.height.saturating_sub(3);
        controls
            .into_iter()
            .enumerate()
            .map(|(index, control)| {
                let column = u16::try_from(index % 2).unwrap_or(0);
                let row = u16::try_from(index / 2).unwrap_or(0);
                let x = inner.x + column.saturating_mul(width);
                let cell_width = if column == 0 {
                    width
                } else {
                    inner.width.saturating_sub(width)
                };
                (control, Rect::new(x, start_y + row, cell_width, 1))
            })
            .collect()
    } else {
        let visible = usize::from(inner.height.min(5));
        let start_y = inner.y + inner.height.saturating_sub(visible as u16);
        controls
            .into_iter()
            .take(visible)
            .enumerate()
            .map(|(index, control)| {
                (
                    control,
                    Rect::new(inner.x, start_y + index as u16, inner.width, 1),
                )
            })
            .collect()
    }
}

pub(super) fn header_add_button_rect(area: Rect) -> Option<Rect> {
    (area.width >= 10 && area.height > 0).then(|| Rect::new(area.x + area.width - 6, area.y, 5, 1))
}

pub(super) fn model_add_button_rect(area: Rect, view: ViewMode) -> Option<Rect> {
    let list = if view == ViewMode::AllEnabled {
        all_models::areas(area).1
    } else if view == ViewMode::Provider {
        Rect::new(
            area.x,
            area.y + 3,
            area.width,
            area.height.saturating_sub(3),
        )
    } else {
        area
    };
    header_add_button_rect(list)
}

pub(super) fn draw_header_add_button(frame: &mut ratatui::Frame, area: Rect) {
    if let Some(rect) = header_add_button_rect(area) {
        frame.render_widget(
            Paragraph::new(" [+] ").style(
                Style::default()
                    .fg(ROUTE)
                    .bg(theme::SURFACE)
                    .add_modifier(Modifier::BOLD),
            ),
            rect,
        );
    }
}

pub(super) fn detail_controls(area: Rect) -> Vec<(DetailControl, Rect)> {
    let inner = panel_inner(area);
    if inner.height == 0 || inner.width < 8 {
        return vec![];
    }
    let (edit_width, delete_width, gap) = if inner.width >= 23 {
        (10, 12, 1)
    } else {
        (inner.width / 2, inner.width - inner.width / 2, 0)
    };
    let x = inner.right() - edit_width - delete_width - gap;
    vec![
        (DetailControl::Edit, Rect::new(x, inner.y, edit_width, 1)),
        (
            DetailControl::Delete,
            Rect::new(x + edit_width + gap, inner.y, delete_width, 1),
        ),
    ]
}

pub(super) fn draw_detail_controls(frame: &mut ratatui::Frame, area: Rect, theme: theme::Theme) {
    for (control, rect) in detail_controls(area) {
        let (text, color) = match control {
            DetailControl::Edit => (
                if rect.width >= 8 { "Edit [E]" } else { "[E]" },
                DATA_SECONDARY,
            ),
            DetailControl::Delete => (
                if rect.width >= 10 {
                    "Delete [x]"
                } else {
                    "[x]"
                },
                ERROR,
            ),
        };
        frame.render_widget(
            Paragraph::new(toolbar::action_line(text, color, false, theme))
                .alignment(Alignment::Center),
            rect,
        );
    }
}

pub(super) fn panel_inner(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

pub(super) fn contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

pub(super) fn clicked_list_index(
    area: Rect,
    column: u16,
    row: u16,
    offset: usize,
    item_height: u16,
) -> Option<usize> {
    let inner = panel_inner(area);
    contains(inner, column, row)
        .then(|| offset + usize::from(row.saturating_sub(inner.y) / item_height.max(1)))
}

pub(super) fn scrollbar_index(
    area: Rect,
    column: u16,
    row: u16,
    length: usize,
    visible: usize,
) -> Option<usize> {
    if length <= visible.max(1) || area.width == 0 || area.height <= 2 {
        return None;
    }
    let scrollbar_x = area.x.saturating_add(area.width.saturating_sub(1));
    let track_y = area.y.saturating_add(1);
    let track_height = area.height.saturating_sub(2);
    if column != scrollbar_x || row < track_y || row >= track_y.saturating_add(track_height) {
        return None;
    }
    if track_height <= 1 {
        return Some(0);
    }
    let relative = usize::from(row.saturating_sub(track_y));
    let denominator = usize::from(track_height.saturating_sub(1));
    Some(
        relative
            .saturating_mul(length.saturating_sub(1))
            .saturating_add(denominator / 2)
            / denominator,
    )
}

pub(super) fn modal_area(screen: Rect) -> Rect {
    centered_rect(
        72.min(screen.width.saturating_sub(4)),
        24.min(screen.height.saturating_sub(2)),
        screen,
    )
}

pub(super) fn modal_area_for(modal: &Modal, screen: Rect) -> Rect {
    match modal {
        Modal::Appearance(_) => settings_page_area(screen),
        Modal::Help(_) => centered_rect(
            104.min(screen.width.saturating_sub(2)),
            32.min(screen.height.saturating_sub(2)),
            screen,
        ),
        Modal::Proxy(_) => settings_page_area(screen),
        Modal::Model(_) => centered_rect(
            86.min(screen.width.saturating_sub(2)),
            20.min(screen.height.saturating_sub(2)),
            screen,
        ),
        _ => modal_area(screen),
    }
}

pub(super) fn route_editor_offset(editor: &RouteEditor, viewport_height: u16) -> usize {
    let visible = usize::from(viewport_height.max(1));
    editor.selected.saturating_add(1).saturating_sub(visible)
}

pub(super) fn modal_button_rects(area: Rect, count: usize) -> Vec<Rect> {
    if count == 0 {
        return vec![];
    }
    let count = u16::try_from(count).unwrap_or(u16::MAX);
    let gap = if count >= 5 { 1_u16 } else { 2_u16 };
    let available = area.width.saturating_sub(4);
    let width = 22_u16
        .min(available.saturating_sub(gap.saturating_mul(count.saturating_sub(1))) / count.max(1));
    let total = width
        .saturating_mul(count)
        .saturating_add(gap.saturating_mul(count.saturating_sub(1)));
    let start = area.x.saturating_add(area.width.saturating_sub(total) / 2);
    (0..count)
        .map(|index| Rect {
            x: start.saturating_add(index.saturating_mul(width.saturating_add(gap))),
            y: area.y.saturating_add(area.height.saturating_sub(2)),
            width,
            height: 1,
        })
        .collect()
}

pub(super) fn draw_modal_buttons(frame: &mut ratatui::Frame, area: Rect, labels: &[&str]) {
    for (index, (rect, label)) in modal_button_rects(area, labels.len())
        .into_iter()
        .zip(labels.iter())
        .enumerate()
    {
        let destructive = label.starts_with("Delete") || label.starts_with("Discard");
        let style = button_style(index == 0, false, destructive);
        frame.render_widget(
            Paragraph::new(format!("[{label}]"))
                .alignment(Alignment::Center)
                .style(style),
            rect,
        );
    }
}

pub(super) fn draw_scrollbar(
    frame: &mut ratatui::Frame,
    area: Rect,
    length: usize,
    position: usize,
    visible: usize,
) {
    if length <= visible.max(1) || area.height <= 2 {
        return;
    }
    let mut state = ScrollbarState::new(length).position(position);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .thumb_style(Style::default().fg(ROUTE))
            .track_style(Style::default().fg(Color::DarkGray)),
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut state,
    );
}

pub(super) fn panel(title: &str, active: bool) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .title(Line::styled(
            title,
            Style::default()
                .fg(if active { ROUTE } else { MUTED })
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(theme::SURFACE))
        .border_style(Style::default().fg(if active {
            theme::ACTIVE_EDGE
        } else {
            theme::EDGE
        }))
}

pub(super) fn detail(label: &str, value: &str) -> Line<'static> {
    detail_value(label, value, Color::Reset)
}

pub(super) fn detail_value(label: &str, value: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<13} "), Style::default().fg(FIELD_LABEL)),
        Span::styled(value.to_owned(), Style::default().fg(color)),
    ])
}

pub(super) fn wrap_styled_segments(
    segments: Vec<(String, Style)>,
    max_width: u16,
) -> Vec<Line<'static>> {
    let max_width = usize::from(max_width.max(1));
    let mut lines = Vec::new();
    let mut spans = Vec::new();
    let mut used = 0_usize;

    for (text, style) in segments {
        // Keep short semantic segments (badges, counts and labels) together.
        let segment_width = UnicodeWidthStr::width(text.as_str());
        if used > 0 && segment_width <= max_width && used + segment_width > max_width {
            lines.push(Line::from(std::mem::take(&mut spans)));
            used = 0;
        }
        let mut chunk = String::new();
        for ch in text.chars() {
            let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if used > 0 && used.saturating_add(char_width) > max_width {
                if !chunk.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut chunk), style));
                }
                lines.push(Line::from(std::mem::take(&mut spans)));
                used = 0;
            }
            chunk.push(ch);
            used = used.saturating_add(char_width);
        }
        if !chunk.is_empty() {
            spans.push(Span::styled(chunk, style));
        }
    }
    if !spans.is_empty() || lines.is_empty() {
        lines.push(Line::from(spans));
    }
    lines
}

pub(super) fn all_enabled_lines(
    provider_count: usize,
    model_count: usize,
    total_count: usize,
    width: u16,
    pi: bool,
) -> Vec<Line<'static>> {
    let active = model_count > 0;
    let mut lines = wrap_styled_segments(
        vec![
            (
                if active { " ● " } else { " ○ " }.into(),
                Style::default().fg(if active { ENABLED } else { MUTED }),
            ),
            (
                "All Models".into(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            (
                if pi {
                    format!("  {total_count} configured models · {provider_count} providers")
                } else {
                    format!(
                        "  {total_count} models · {model_count} enabled · {provider_count} providers"
                    )
                },
                Style::default().fg(ENABLED),
            ),
        ],
        width,
    );
    lines.push(Line::raw(""));
    lines
}

pub(super) fn home_profile_lines(
    id: &str,
    profile: &Profile,
    enabled_count: usize,
    width: u16,
    pi: bool,
) -> Vec<Line<'static>> {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let signal = if profile.enabled { " ● " } else { " ○ " };
    let signal_color = if profile.enabled { ENABLED } else { MUTED };
    let mut lines = wrap_styled_segments(
        vec![
            (signal.into(), Style::default().fg(signal_color)),
            (profile.name.clone(), bold),
            (
                format!("  [{}]", profile.api_format.label()),
                Style::default().fg(ROUTE),
            ),
            (format!("  {id}"), Style::default().fg(MUTED)),
        ],
        width,
    );

    let summary = vec![
        ("     Default: ".into(), Style::default().fg(DEFAULT_LABEL)),
        (
            profile.default_model.clone(),
            Style::default()
                .fg(DEFAULT_MODEL)
                .add_modifier(Modifier::BOLD),
        ),
        (
            if pi {
                format!("   {enabled_count} configured")
            } else if profile.enabled {
                format!("   {enabled_count} enabled")
            } else {
                "   provider disabled".into()
            },
            Style::default().fg(if profile.enabled { ENABLED } else { MUTED }),
        ),
    ];
    if width >= 96
        && UnicodeWidthStr::width(
            format!(
                " {} {}  [{}]  {}     Default: {}   {} {}",
                if profile.enabled { "●" } else { "○" },
                profile.name,
                profile.api_format.label(),
                id,
                profile.default_model,
                enabled_count,
                if pi { "configured" } else { "enabled" }
            )
            .as_str(),
        ) <= usize::from(width)
    {
        lines.clear();
        lines.extend(wrap_styled_segments(
            vec![
                (signal.into(), Style::default().fg(signal_color)),
                (profile.name.clone(), bold),
                (
                    format!("  [{}]", profile.api_format.label()),
                    Style::default().fg(ROUTE),
                ),
                (format!("  {id}"), Style::default().fg(MUTED)),
            ]
            .into_iter()
            .chain(summary.clone())
            .collect(),
            width,
        ));
    } else {
        lines.extend(wrap_styled_segments(summary, width));
    }
    lines.extend(wrap_styled_segments(
        vec![
            ("     Endpoint: ".into(), Style::default().fg(FIELD_LABEL)),
            (profile.base_url.clone(), Style::default()),
        ],
        width,
    ));
    lines.extend(wrap_styled_segments(
        vec![
            ("     Credential: ".into(), Style::default().fg(FIELD_LABEL)),
            (profile.credential.masked(), Style::default().fg(MUTED)),
        ],
        width,
    ));
    lines.push(Line::raw(""));
    lines
}

pub(super) fn visible_variable_items(
    heights: &[usize],
    offset: usize,
    viewport_height: usize,
) -> usize {
    let mut used = 0_usize;
    heights
        .iter()
        .skip(offset)
        .take_while(|height| {
            let fits = used == 0 || used.saturating_add(**height) <= viewport_height;
            if fits {
                used = used.saturating_add(**height);
            }
            fits
        })
        .count()
}

pub(super) fn clicked_variable_item(
    area: Rect,
    row: u16,
    offset: usize,
    heights: &[usize],
) -> Option<usize> {
    let inner = panel_inner(area);
    if row < inner.y || row >= inner.y.saturating_add(inner.height) {
        return None;
    }
    let target = usize::from(row.saturating_sub(inner.y));
    let mut top = 0_usize;
    for (index, height) in heights.iter().enumerate().skip(offset) {
        if target < top.saturating_add(*height) {
            return Some(index);
        }
        top = top.saturating_add(*height);
        if top >= usize::from(inner.height) {
            break;
        }
    }
    None
}

pub(super) fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

pub(super) fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(super) fn unique_profile_id(base: &str, profiles: &BTreeMap<String, Profile>) -> String {
    if !profiles.contains_key(base) {
        return base.into();
    }
    (2..)
        .map(|index| format!("{base}-{index}"))
        .find(|id| !profiles.contains_key(id))
        .unwrap()
}

/// Full-width account pages place the list beside the selected account details.
pub(super) fn account_page_rows(area: Rect, login_busy: bool) -> [Rect; 4] {
    let header_height = if area.height < 16 {
        2
    } else if login_busy {
        3
    } else {
        4
    };
    let footer_height = 3;
    let content_y = area.y + header_height;
    let content_height = area.height.saturating_sub(header_height + footer_height);
    let header = Rect::new(area.x, area.y, area.width, header_height);
    let footer = Rect::new(
        area.x,
        area.bottom().saturating_sub(footer_height),
        area.width,
        footer_height,
    );
    if area.width >= 120 && area.height >= 24 && !login_busy {
        let left_width = (area.width / 3).clamp(34, 46);
        let list = Rect::new(area.x, content_y, left_width, content_height);
        let detail = Rect::new(
            area.x + left_width + 1,
            content_y,
            area.width.saturating_sub(left_width + 1),
            content_height,
        );
        [header, list, detail, footer]
    } else {
        let list_height = if login_busy {
            0
        } else if area.height < 16 {
            3
        } else {
            content_height * 2 / 5
        };
        let list = Rect::new(area.x, content_y, area.width, list_height);
        let detail = Rect::new(
            area.x,
            content_y + list_height,
            area.width,
            content_height.saturating_sub(list_height),
        );
        [header, list, detail, footer]
    }
}

pub(super) fn embedded_account_rows(area: Rect, login_busy: bool) -> [Rect; 4] {
    let header_height = 3.min(area.height);
    let footer_height = 3.min(area.height.saturating_sub(header_height));
    let content_y = area.y.saturating_add(header_height);
    let content_height = area.height.saturating_sub(header_height + footer_height);
    let list_width = if login_busy {
        0
    } else {
        (area.width / 3)
            .clamp(26, 42)
            .min(area.width.saturating_sub(28))
    };
    [
        Rect::new(area.x, area.y, area.width, header_height),
        Rect::new(area.x, content_y, list_width, content_height),
        Rect::new(
            area.x
                .saturating_add(list_width + u16::from(list_width > 0)),
            content_y,
            area.width
                .saturating_sub(list_width + u16::from(list_width > 0)),
            content_height,
        ),
        Rect::new(
            area.x,
            area.bottom().saturating_sub(footer_height),
            area.width,
            footer_height,
        ),
    ]
}
