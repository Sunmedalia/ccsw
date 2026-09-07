use super::*;

pub(super) fn ui_areas(area: Rect, focus: Focus, view_mode: ViewMode) -> UiAreas {
    let rows = app_rows(area);
    match view_mode {
        ViewMode::Home => UiAreas {
            profiles: Some(rows[1]),
            models: None,
            details: None,
            footer: rows[2],
        },
        ViewMode::Provider => {
            if area.width >= 100 {
                let cols = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
                    .split(rows[1]);
                UiAreas {
                    profiles: None,
                    models: Some(cols[0]),
                    details: Some(cols[1]),
                    footer: rows[2],
                }
            } else {
                UiAreas {
                    profiles: None,
                    models: (focus != Focus::Details).then_some(rows[1]),
                    details: (focus == Focus::Details).then_some(rows[1]),
                    footer: rows[2],
                }
            }
        }
        ViewMode::AllEnabled => UiAreas {
            profiles: None,
            models: Some(rows[1]),
            details: None,
            footer: rows[2],
        },
    }
}

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
        (FooterControl::Help, 8),
        (FooterControl::Quit, 8),
    ];
    pub(super) const HOME_COMPACT: &[(FooterControl, u16)] = &[
        (FooterControl::AddProfile, 9),
        (FooterControl::Sync, 8),
        (FooterControl::Proxy, 7),
        (FooterControl::Help, 5),
        (FooterControl::Quit, 5),
    ];
    pub(super) const PROVIDER_WIDE: &[(FooterControl, u16)] = &[
        (FooterControl::Back, 14),
        (FooterControl::Models, 11),
        (FooterControl::Details, 11),
        (FooterControl::Sync, 14),
        (FooterControl::Proxy, 10),
        (FooterControl::Help, 8),
        (FooterControl::Quit, 8),
    ];
    pub(super) const PROVIDER_COMPACT: &[(FooterControl, u16)] = &[
        (FooterControl::Back, 8),
        (FooterControl::Models, 8),
        (FooterControl::Details, 8),
        (FooterControl::Sync, 8),
        (FooterControl::Proxy, 7),
        (FooterControl::Help, 5),
        (FooterControl::Quit, 5),
    ];
    pub(super) const HOME_TINY: &[(FooterControl, u16)] = &[
        (FooterControl::AddProfile, 7),
        (FooterControl::Sync, 6),
        (FooterControl::Proxy, 5),
        (FooterControl::Help, 3),
        (FooterControl::Quit, 3),
    ];
    pub(super) const PROVIDER_TINY: &[(FooterControl, u16)] = &[
        (FooterControl::Back, 6),
        (FooterControl::Models, 5),
        (FooterControl::Details, 5),
        (FooterControl::Sync, 5),
        (FooterControl::Proxy, 5),
        (FooterControl::Help, 3),
        (FooterControl::Quit, 3),
    ];
    pub(super) const ALL_WIDE: &[(FooterControl, u16)] = &[
        (FooterControl::Back, 14),
        (FooterControl::Sync, 14),
        (FooterControl::Proxy, 10),
        (FooterControl::Help, 8),
        (FooterControl::Quit, 8),
    ];
    pub(super) const ALL_COMPACT: &[(FooterControl, u16)] = &[
        (FooterControl::Back, 8),
        (FooterControl::Sync, 8),
        (FooterControl::Proxy, 7),
        (FooterControl::Help, 5),
        (FooterControl::Quit, 5),
    ];
    pub(super) const ALL_TINY: &[(FooterControl, u16)] = &[
        (FooterControl::Back, 6),
        (FooterControl::Sync, 5),
        (FooterControl::Proxy, 5),
        (FooterControl::Help, 3),
        (FooterControl::Quit, 3),
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
            x = x.saturating_add(*width).saturating_add(1);
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

pub(super) fn showcase_controls(area: Rect) -> Vec<(ShowcaseControl, Rect)> {
    let inner = panel_inner(area);
    if inner.height < 2 || inner.width < 8 {
        return vec![];
    }
    let controls = [
        ShowcaseControl::Toggle,
        ShowcaseControl::Default,
        ShowcaseControl::OneM,
        ShowcaseControl::Delete,
    ];
    if inner.width >= 38 && inner.height >= 2 {
        let width = inner.width / 2;
        let start_y = inner.y + inner.height.saturating_sub(2);
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
        let visible = usize::from(inner.height.min(4));
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

pub(super) fn catalog_add_button_rect(search_area: Rect) -> Option<Rect> {
    let inner = panel_inner(search_area);
    if inner.width >= 10 {
        let btn_w = if inner.width >= 35 { 17 } else { 6 };
        let btn_x = inner.x + inner.width.saturating_sub(btn_w);
        Some(Rect::new(btn_x, inner.y, btn_w, 1))
    } else {
        None
    }
}

pub(super) fn detail_controls(area: Rect) -> Vec<(DetailControl, Rect)> {
    let inner = panel_inner(area);
    if inner.height == 0 || inner.width < 8 {
        return vec![];
    }
    let y = inner.y + inner.height.saturating_sub(1);
    if inner.width >= 28 {
        let first_width = inner.width / 2;
        vec![
            (
                DetailControl::FetchModels,
                Rect::new(inner.x, y, first_width, 1),
            ),
            (
                DetailControl::Edit,
                Rect::new(
                    inner.x.saturating_add(first_width),
                    y,
                    inner.width.saturating_sub(first_width),
                    1,
                ),
            ),
        ]
    } else {
        vec![(DetailControl::Edit, Rect::new(inner.x, y, inner.width, 1))]
    }
}

pub(super) fn draw_detail_controls(frame: &mut ratatui::Frame, area: Rect) {
    for (control, rect) in detail_controls(area) {
        let (label, style) = match control {
            DetailControl::FetchModels => (
                "[Fetch models (r)]",
                Style::default()
                    .fg(Color::Black)
                    .bg(ROUTE)
                    .add_modifier(Modifier::BOLD),
            ),
            DetailControl::Edit => ("[Edit provider (E)]", Style::default().fg(WARNING)),
        };
        frame.render_widget(
            Paragraph::new(label)
                .alignment(Alignment::Center)
                .style(style),
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
        Modal::Help(_) => centered_rect(
            82.min(screen.width.saturating_sub(2)),
            22.min(screen.height.saturating_sub(2)),
            screen,
        ),
        Modal::Proxy(_) => centered_rect(
            82.min(screen.width.saturating_sub(2)),
            22.min(screen.height.saturating_sub(2)),
            screen,
        ),
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
        let style = if index == 0 {
            Style::default()
                .fg(Color::Black)
                .bg(ROUTE)
                .add_modifier(Modifier::BOLD)
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
        .title(title)
        .border_style(Style::default().fg(if active { ROUTE } else { MUTED }))
}

pub(super) fn detail(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<13} "), Style::default().fg(MUTED)),
        Span::raw(value.to_owned()),
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
) -> Vec<Line<'static>> {
    let active = model_count > 0;
    let mut lines = wrap_styled_segments(
        vec![
            (
                if active { " ● " } else { " ○ " }.into(),
                Style::default().fg(if active { CONNECTED } else { MUTED }),
            ),
            (
                "All Models".into(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            (
                format!(
                    "  {total_count} models · {model_count} enabled · {provider_count} providers"
                ),
                Style::default().fg(CONNECTED),
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
) -> Vec<Line<'static>> {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let signal = if profile.enabled { " ● " } else { " ○ " };
    let signal_color = if profile.enabled { CONNECTED } else { MUTED };
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
        (
            format!("     Default: {}", profile.default_model),
            Style::default().fg(WARNING),
        ),
        (
            if profile.enabled {
                format!("   {enabled_count} enabled")
            } else {
                "   provider disabled".into()
            },
            Style::default().fg(if profile.enabled { CONNECTED } else { WARNING }),
        ),
    ];
    if width >= 96
        && UnicodeWidthStr::width(
            format!(
                " {} {}  [{}]  {}     Default: {}   {} enabled",
                if profile.enabled { "●" } else { "○" },
                profile.name,
                profile.api_format.label(),
                id,
                profile.default_model,
                enabled_count
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
            ("     Endpoint: ".into(), Style::default().fg(MUTED)),
            (profile.base_url.clone(), Style::default()),
        ],
        width,
    ));
    lines.extend(wrap_styled_segments(
        vec![
            ("     Credential: ".into(), Style::default().fg(MUTED)),
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
