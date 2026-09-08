use super::*;

pub(super) fn help_commands(section: HelpSection) -> &'static [(&'static str, &'static str)] {
    match section {
        HelpSection::Home => &[
            ("↑↓ / j k", "Select All Models or a provider"),
            ("Enter / Click", "Open selection"),
            ("Space", "Toggle provider; auto-sync after connection"),
            ("n / x", "New / delete provider"),
            ("e", "Edit selected provider"),
            ("r / t", "Test connection and fetch model catalog"),
            ("A", "Enable all models in the selected provider"),
            ("p / P", "Connect or sync all models / manage proxy"),
            ("q", "Quit CCSW"),
        ],
        HelpSection::AllEnabled => &[
            ("↑↓ / j k", "Select a model across providers"),
            (
                "PgUp / PgDn",
                "Page through models; Home / End jump to edges",
            ),
            ("Space", "Toggle model; auto-sync after connection"),
            ("Enter / click again", "Open the selected model’s provider"),
            ("p / P", "Connect or sync all models / manage proxy"),
            ("Esc", "Return to providers"),
        ],
        HelpSection::Provider => &[
            (
                "↑↓ / j k",
                "Browse models; Tab switches panels in narrow windows",
            ),
            ("/ / Esc", "Search / clear search or return home"),
            ("Space / d / 1", "Toggle / set default / toggle 1M"),
            (
                "A / C",
                "Enable filtered models / clear non-essential enabled models",
            ),
            ("a / x", "Add model / delete custom model"),
            ("e", "Edit selected model from either panel"),
            ("E (Shift+e)", "Edit provider configuration"),
            ("r / p / P", "Fetch models / sync / proxy"),
        ],
        HelpSection::Forms => &[
            (
                "↑↓ / Tab",
                "Next field; Shift+Tab returns to previous field",
            ),
            ("Enter", "Next field; save at the last field"),
            ("←→ / Home End", "Move text cursor"),
            (
                "Backspace / Del",
                "Delete characters; Ctrl+U clears the field",
            ),
            ("Space / ←→", "Change toggle or option"),
            (
                "Ctrl+F / Ctrl+R",
                "Model form: fetch available models from the provider API",
            ),
            (
                "Alt+1",
                "Model form: toggle 1M context from any field or search",
            ),
            ("Ctrl+S", "Save changes"),
            ("Esc", "Cancel; clear active model search first"),
        ],
    }
}

pub(super) fn help_tabs(active: HelpSection) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, section) in HelpSection::ALL.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        let label = format!(" {} {} ", index + 1, section.label());
        let style = if *section == active {
            Style::default()
                .fg(Color::Black)
                .bg(ROUTE)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(MUTED)
        };
        spans.push(Span::styled(label, style));
    }
    Line::from(spans)
}

pub(super) fn help_content(section: HelpSection, wide: bool) -> Vec<Line<'static>> {
    let heading = match section {
        HelpSection::Home => "Home · Providers",
        HelpSection::AllEnabled => "All Models · Across providers",
        HelpSection::Provider => "Provider · Models and connection",
        HelpSection::Forms => "Forms · Editing",
    };
    let mut lines = vec![
        Line::styled(
            heading,
            Style::default().fg(ROUTE).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
    ];
    for (key, action) in help_commands(section) {
        let key = if wide {
            format!("{key:<18}")
        } else {
            format!("{key}  ")
        };
        lines.push(Line::from(vec![
            Span::styled(key, Style::default().fg(WARNING)),
            Span::raw(*action),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("●", Style::default().fg(CONNECTED)),
        Span::styled(" enabled  ", Style::default().fg(MUTED)),
        Span::styled("○", Style::default().fg(MUTED)),
        Span::styled(" disabled  ", Style::default().fg(MUTED)),
        Span::styled("◆", Style::default().fg(ROUTE)),
        Span::styled(" default  ", Style::default().fg(MUTED)),
        Span::styled("◈", Style::default().fg(WARNING)),
        Span::styled(" role dependency", Style::default().fg(MUTED)),
    ]));
    lines
}

pub(super) fn draw_help(frame: &mut ratatui::Frame, area: Rect, help: &HelpModal) {
    let compact = area.width < 58 || area.height < 14;
    let title = if compact {
        format!(" Help · {} · Esc ", help.section.label())
    } else {
        format!(" Help · {} ", help.section.label())
    };
    frame.render_widget(panel(&title, true), area);

    let inner = panel_inner(area);
    let show_button = area.height >= 12;
    let usable = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(if show_button { 2 } else { 0 }),
    );
    if usable.height == 0 {
        return;
    }

    let tab_height = if compact { 2 } else { 1 }.min(usable.height);
    let tabs = Rect::new(usable.x, usable.y, usable.width, tab_height);
    frame.render_widget(
        Paragraph::new(help_tabs(help.section)).wrap(Wrap { trim: true }),
        tabs,
    );

    let show_navigation = !compact && usable.height > tab_height;
    let navigation_height = u16::from(show_navigation);
    if show_navigation {
        let navigation = Rect::new(
            usable.x,
            usable.y.saturating_add(tab_height),
            usable.width,
            1,
        );
        frame.render_widget(
            Paragraph::new("←→ / Tab sections · ↑↓ scroll · 1–4 jump")
                .style(Style::default().fg(MUTED)),
            navigation,
        );
    }

    let content_y = usable
        .y
        .saturating_add(tab_height)
        .saturating_add(navigation_height);
    let content = Rect::new(
        usable.x,
        content_y,
        usable.width,
        usable
            .height
            .saturating_sub(tab_height.saturating_add(navigation_height)),
    );
    if content.height > 0 {
        frame.render_widget(
            Paragraph::new(help_content(help.section, area.width >= 58))
                .wrap(Wrap { trim: false })
                .scroll((help.scroll, 0)),
            content,
        );
    }

    if show_button {
        draw_modal_buttons(frame, area, &["Close  Esc / q / ? / Enter"]);
    }
}
