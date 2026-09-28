use super::*;

// Shared action typography for provider workspaces and Usage.
pub(super) fn action_line(
    text: &str,
    color: Color,
    selected: bool,
    theme: theme::Theme,
) -> Line<'static> {
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
        spans.push(Span::styled(
            format!(" [{shortcut}"),
            Style::default().fg(MUTED),
        ));
    }
    Line::from(spans).style(if selected { style } else { Style::default() })
}
