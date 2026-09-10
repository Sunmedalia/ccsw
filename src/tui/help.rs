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
            (
                "Click top tabs / F2",
                "Switch independent Claude Code / Codex / Pi configurations",
            ),
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
    let label = if help.codex {
        ["Providers", "Accounts", "Models", "Forms"][help.section.index()]
    } else {
        help.section.label()
    };
    let title = if compact {
        format!(" Help · {} · Esc ", label)
    } else {
        format!(" Help · {} ", label)
    };
    let title = if help.codex {
        title.replacen("Help", "Codex Help", 1)
    } else if help.pi {
        title.replacen("Help", "Pi Help", 1)
    } else {
        title
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
        Paragraph::new(if help.codex {
            Line::from(
                HelpSection::ALL
                    .iter()
                    .enumerate()
                    .map(|(i, section)| {
                        let label = ["Providers", "Accounts", "Models", "Forms"][i];
                        Span::styled(
                            format!(" {} {} ", i + 1, label),
                            if *section == help.section {
                                Style::default().fg(Color::Black).bg(ROUTE)
                            } else {
                                Style::default().fg(MUTED)
                            },
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        } else {
            help_tabs(help.section)
        })
        .wrap(Wrap { trim: true }),
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
            Paragraph::new(if help.codex {
                codex_help_content(help.section)
            } else if help.pi {
                pi_help_content(help.section)
            } else {
                help_content(help.section, area.width >= 58)
            })
            .wrap(Wrap { trim: false })
            .scroll((help.scroll, 0)),
            content,
        );
    }

    if show_button {
        draw_modal_buttons(frame, area, &["Close  Esc / q / ? / Enter"]);
    }
}

fn pi_help_content(section: HelpSection) -> Vec<Line<'static>> {
    if section == HelpSection::Forms {
        return help_content(section, false);
    }
    let mut lines = vec![
        Line::styled(
            "Pi · Independent API providers and models",
            Style::default().fg(ROUTE),
        ),
        Line::raw("p  Sync enabled models directly to Pi; select the default model"),
        Line::raw("i  Import Pi API configuration"),
        Line::raw("r  Test connection and fetch provider models"),
        Line::raw("s  Inspect Pi configuration status"),
        Line::raw("D  Disconnect and restore managed settings"),
        Line::raw(""),
    ];
    for (key, action) in help_commands(section) {
        if key.contains('p') && (action.contains("proxy") || action.contains("sync")) {
            continue;
        }
        lines.push(Line::raw(format!("{key}  {action}")));
    }
    lines.extend([
        Line::raw(""),
        Line::raw("Pi uses its own provider list, model catalog and configuration."),
        Line::raw(
            "After syncing, open /model in Pi. Connected edits auto-sync; p retries failures.",
        ),
        Line::raw("Pi connects directly to the API; no CCSW proxy or Codex subscription accounts."),
    ]);
    lines
}

fn codex_help_content(section: HelpSection) -> Vec<Line<'static>> {
    let rows: &[(&str, &str)] = match section {
        HelpSection::Home => &[
            (
                "F3",
                "Open ChatGPT accounts; also available as the first provider",
            ),
            (
                "Enter / click again",
                "Open selected provider or ChatGPT Account",
            ),
            ("n / e / x", "Add / edit / remove an API provider"),
            ("p", "Apply selected provider or saved ChatGPT account"),
            (
                "Top tabs / F2",
                "Switch independent Claude / Codex / Pi configurations",
            ),
            (
                "",
                "Exactly one provider is selected: ChatGPT account or API provider.",
            ),
        ],
        HelpSection::AllEnabled => &[
            ("i", "Import current Codex login; enter a name"),
            ("I", "Import an auth.json file by absolute path"),
            ("Up/Down", "Move the account cursor"),
            ("Space", "Select the highlighted account without applying"),
            ("p / Apply", "Activate the account and ChatGPT provider"),
            ("Esc / Back", "Return to API providers"),
            (
                "",
                "Import highlights; Space selects; Apply activates. No quota queries.",
            ),
            (
                "",
                "Revoked token: sign in with Codex, then reimport the fresh login.",
            ),
            (
                "",
                "Restart Codex CLI / App and open a new chat after switching.",
            ),
        ],
        HelpSection::Provider => &[
            ("a / e / E", "Add model / edit model / edit provider"),
            ("r", "Fetch API models"),
            (
                "Space / d / 1",
                "Enable model / set default / toggle 1M context",
            ),
            ("p / g", "Use API model / set reasoning"),
            (
                "s / D",
                "Local status / disconnect and restore managed settings",
            ),
            (
                "",
                "API selection uses its endpoint and model; ChatGPT uses saved login.",
            ),
        ],
        HelpSection::Forms => return help_content(section, false),
    };
    rows.iter()
        .map(|(key, action)| {
            Line::from(vec![
                Span::styled(format!("{key}  "), Style::default().fg(WARNING)),
                Span::raw(*action),
            ])
        })
        .collect()
}
