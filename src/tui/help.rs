use super::*;

/// Shortcuts are grouped by the page that owns the key. Search and text fields
/// consume typed characters before page commands.
pub(super) fn help_commands(section: HelpSection) -> &'static [(&'static str, &'static str)] {
    match section {
        HelpSection::Home => &[
            (
                "?",
                "Open Help; available on provider, account and Usage pages",
            ),
            ("F2 / top tabs", "Cycle Claude / Codex / Pi / Grok"),
            (
                "F4 / F6",
                "Open Settings / Usage at the same navigation level",
            ),
            ("F5", "Test the selected model with a minimal request"),
            (
                "Esc / q",
                "Return one level; quit only from the Providers root",
            ),
            (
                "Ctrl+C",
                "Quit from Home / full-screen provider sidebar; cancel active login",
            ),
            (
                "↑↓ / j k",
                "Select All Models, a provider or an account entry",
            ),
            ("Home / End", "Full screen: first / last sidebar entry"),
            ("PgUp / PgDn", "Full screen: move five sidebar entries"),
            ("Enter / l", "Open selection; full screen: focus model list"),
            (
                "Tab / Shift+Tab",
                "Full screen: cycle provider / model / details panels",
            ),
            (
                "a",
                "Add Provider when Home or the provider sidebar is focused",
            ),
            ("e / E / x", "Edit / edit / delete selected provider"),
            (
                "Space",
                "Toggle selected provider; Codex account toggle requires confirmation",
            ),
            ("A", "Home: enable every model in selected provider"),
            (
                "p / P",
                "Sync / open Proxy page; see client-specific commands below",
            ),
            (
                "Click",
                "Top actions are clickable; Add Provider is always available",
            ),
        ],
        HelpSection::AllEnabled => &[
            (
                "/",
                "Filter model label, ID, provider name or ID; words combine with AND",
            ),
            (
                "Text / Backspace",
                "While filtering: type / remove a character",
            ),
            ("Ctrl+U / Clear", "While filtering: clear the query"),
            (
                "Esc",
                "Clear query first; leave input if empty; otherwise return to providers",
            ),
            (
                "Enter / Tab / ↑↓",
                "While filtering: leave input; arrows also select a model",
            ),
            (
                "↑↓ / j k",
                "Model list: select; configuration panel: scroll details",
            ),
            (
                "PgUp / PgDn",
                "Page through models or configuration details",
            ),
            (
                "Home / End",
                "First / last model or start / end of configuration",
            ),
            (
                "Tab / Shift+Tab",
                "Full screen: cycle provider / model / configuration panels",
            ),
            (
                "h / l",
                "h: return one panel; l: enter configuration, then provider; narrow: back / open",
            ),
            ("Enter", "Open the selected model's provider"),
            (
                "Click model",
                "Full screen: show configuration in place; narrow: click again to open",
            ),
            (
                "a",
                "Add Model for the selected result; sidebar focus adds Provider instead",
            ),
            (
                "Space / status dot",
                "Toggle the selected model (Pi entries are configured directly)",
            ),
            (
                "p / P",
                "Sync / open Proxy page; see client-specific commands below",
            ),
            (
                "? / F4 / F5 / F6",
                "Help / settings / model request test / Usage",
            ),
            ("q", "Return one level; quit from the Providers root"),
        ],
        HelpSection::Provider => &[
            (
                "Tab / Shift+Tab",
                "Cycle panels; full screen includes the provider sidebar",
            ),
            (
                "h / l / ←→",
                "h: details → models → providers; l: providers → models → details",
            ),
            (
                "↑↓ / j k",
                "Select model; provider sidebar focus selects provider",
            ),
            (
                "PgUp / PgDn",
                "Move ten models; sidebar moves five providers",
            ),
            (
                "Home / End",
                "First / last model; sidebar selects first / last provider",
            ),
            (
                "a",
                "Add Model from models/details; Add Provider from sidebar",
            ),
            ("e / E", "Edit selected model / edit provider"),
            (
                "x",
                "Models: delete model; Details or sidebar: delete provider",
            ),
            (
                "Space / d / 1",
                "Toggle model / set default / toggle 1M context",
            ),
            (
                "A / C",
                "Enable filtered models / clear non-essential enabled models",
            ),
            ("/", "Start model search"),
            ("Text / Backspace", "In search: type / remove a character"),
            ("Esc (search)", "Clear query; when empty, leave search"),
            ("Tab / Down", "Leave search input"),
            (
                "Up / Enter (search)",
                "Previous result / toggle result; Pi Enter leaves search",
            ),
            (
                "p / P",
                "Sync / open Proxy page; see client-specific commands below",
            ),
            ("Esc", "Return to providers outside search"),
            (
                "? / F4 / F5 / F6",
                "Help / settings / model request test / Usage",
            ),
            ("Esc / q", "Return one level outside search input"),
        ],
        HelpSection::Forms => &[
            (
                "Tab / ↓",
                "Next field; Shift+Tab / Up selects previous field",
            ),
            ("Enter", "Next field; submit at the last field"),
            ("Ctrl+S", "Save provider / model / preferences form"),
            ("←→", "Move text cursor; change an option or toggle"),
            ("Home / Ctrl+A", "Move cursor to start of field"),
            ("End / Ctrl+E", "Move cursor to end of field"),
            ("Backspace / Delete", "Delete previous / next character"),
            ("Ctrl+U", "Clear text field"),
            ("Space", "Change toggle or option; otherwise type a space"),
            ("Esc", "Cancel; active model search clears before closing"),
            (
                "Template ↑↓ / k j",
                "Select template; Tab / Shift+Tab also select",
            ),
            ("Template Enter / l", "Use template; Esc / h cancels"),
            (
                "Alt+F / Ctrl+R",
                "Provider: cached model picker / fetch API models; Model: fetch API models",
            ),
            (
                "Picker /",
                "Start model search; type to filter; Backspace removes text",
            ),
            ("Picker ↑↓ / j k", "Select API model; PgUp / PgDn scroll"),
            (
                "Picker Enter / l",
                "Use model; Esc / h goes back (outside search input)",
            ),
            (
                "Model Tab / Shift+Tab",
                "Switch API picker / configuration fields",
            ),
            (
                "Provider F5",
                "Test connection from Base URL field; other fields test model",
            ),
            ("Alt+1", "Toggle 1M context in provider / model forms"),
            (
                "y / Enter",
                "Confirm provider/model deletion or account confirmation",
            ),
            (
                "n / q / Esc",
                "Cancel provider/model deletion; account confirmation uses n / Esc",
            ),
            (
                "Import i / Enter",
                "Accept discovered configuration; Esc skips",
            ),
            (
                "Grok import ↑↓ / Pg",
                "Scroll import/reconnect preview; Enter accepts; Esc cancels",
            ),
            (
                "Codex input",
                "Enter submit; Esc cancel; Backspace delete; Ctrl+U clear",
            ),
        ],
        HelpSection::Usage => &[
            (
                "r / x",
                "Refresh logs and sessions / reset all dashboard filters",
            ),
            ("h / Esc / F6", "Back to providers"),
            ("q / Esc", "Return to Providers; search consumes q as text"),
            (
                "? / F4 / F2",
                "Help / settings / return to Claude provider page",
            ),
            (
                "[ / ]",
                "Previous / next agent filter; clears provider filter",
            ),
            ("← / → / t", "Previous / next day (window end) / Today"),
            ("d / w / m / y", "1 day / 7 days / 30 days / all time"),
            ("a", "Clear provider filter"),
            ("Tab / Shift+Tab", "Next / previous dashboard section"),
            (
                "1–6",
                "Providers / History / Metrics / Models / Charts / Sessions",
            ),
            ("↑↓ / j k / wheel", "Scroll the page"),
            ("PgUp / PgDn", "Scroll one page"),
            ("Home / End", "Start / end of page"),
            ("n / p / Alt+↓↑", "Next / previous row in the active table"),
            (
                "Click row",
                "Expand details in place without moving the page",
            ),
            (
                "Enter / l",
                "Provider: filter & models; History: date & chart; Session: inspect",
            ),
            ("c / v", "Calls / tokens chart"),
            (
                "/",
                "Focus Sessions and search project, session ID or model",
            ),
            (
                "f",
                "Sessions: toggle following dashboard dates (default: all dates)",
            ),
            ("s", "Sessions: toggle recent / token sort"),
            ("Search ↑↓", "Select session while typing"),
            ("Search Enter / Esc", "Leave search; query remains active"),
            (
                "Search Backspace / Ctrl+U",
                "Delete character / clear session query",
            ),
        ],
        HelpSection::Accounts => &[
            ("Codex F3", "Open / close ChatGPT Accounts"),
            ("Codex ↑↓ / j k", "Select account"),
            ("Codex PgUp / PgDn", "Scroll cached account details"),
            ("Codex b / d", "Browser login / device code login"),
            (
                "Codex i / I",
                "Import current login / import auth.json file",
            ),
            ("Codex e / x", "Rename / delete account (confirmation)"),
            (
                "Codex Space / p",
                "Select without applying / apply selected account",
            ),
            (
                "Codex r / w",
                "Refresh usage / wake account and refresh (uses some quota)",
            ),
            ("Codex s / D", "Local status / disconnect (confirmation)"),
            (
                "Codex Esc",
                "Cancel active login; otherwise back to providers",
            ),
            (
                "Codex status ↑↓ / j k",
                "Scroll status; PgUp/PgDn page; Esc/q/?/Enter closes",
            ),
            ("Grok o", "Open OAuth account page from providers"),
            (
                "Grok Tab / ↓",
                "Next control; Shift+Tab / Up previous control",
            ),
            (
                "Grok Enter",
                "Run focused action; model field advances to actions",
            ),
            (
                "Grok b / d / u",
                "Browser login / device login / use configured OAuth model",
            ),
            (
                "Grok r / s / w",
                "Refresh cached usage / refresh / wake and refresh",
            ),
            ("Grok x", "Log out (Enter/y confirms; n/Esc cancels)"),
            ("Grok PgUp / PgDn", "Scroll account details or login output"),
            ("Grok Esc / q", "Back; Esc / Ctrl+C cancels an active login"),
            ("? / F2", "Help / switch agent when navigation is available"),
        ],
        HelpSection::Settings => &[
            ("F4", "Open settings from provider, model or Usage pages"),
            (
                "Tab / p / Shift+Tab",
                "Next / previous TUI theme, Pulse theme, refresh interval",
            ),
            ("↑← / k h", "Previous theme or decrease refresh interval"),
            ("↓→ / j l", "Next theme or increase refresh interval"),
            ("Enter / s / Esc", "Save / save / cancel settings"),
            ("c", "Claude / Grok: open client preferences"),
            (
                "Preferences Ctrl+S",
                "Save client preferences; other form keys in Forms",
            ),
            ("Claude Alt+P", "Fill preset environment variables"),
            (
                "Claude Alt+N / Alt+D",
                "Add / delete custom environment entry",
            ),
            ("Claude Alt+V", "Show / mask custom environment value"),
            (
                "Claude Alt+X",
                "Disconnect when no unsaved draft or pending operation",
            ),
            (
                "Claude Esc / y",
                "Ask to discard dirty preferences / confirm discard",
            ),
            ("Claude discard other key", "Keep editing"),
            ("Grok Alt+M", "Cycle configured model keys in model fields"),
            (
                "Grok Alt+E",
                "Type a custom reasoning value in reasoning field",
            ),
            (
                "Grok discard y / Enter",
                "Discard changes; n / Esc keeps editing",
            ),
        ],
        HelpSection::Proxy => &[
            (
                "P",
                "Claude: proxy manager; Pi: selected provider's proxy API",
            ),
            ("Tab / ↓→ / j l", "Next action"),
            ("Shift+Tab / ↑← / k h", "Previous action"),
            ("Enter / Space", "Run selected action"),
            ("s / x / r", "Start / stop / refresh proxy"),
            ("i / u", "Enable / disable startup at login"),
            ("e", "Edit port when proxy is idle"),
            (
                "Port Enter / Esc",
                "Apply / cancel port edit; form cursor keys also work",
            ),
            ("Esc / P / q", "Close proxy manager"),
            (
                "Grok P",
                "Show gateway information; p connects API providers",
            ),
        ],
        HelpSection::Pulse => &[
            (
                "Pulse",
                "Standalone monitor (mux pulse), separate from Usage",
            ),
            ("q / Ctrl+C", "Quit monitor"),
            ("?", "Toggle monitor help"),
            ("Tab / 1–4", "Cycle / select agent"),
            ("e", "Open editor in a new tab"),
            ("r", "Refresh account usage and sessions"),
            ("m", "Summary: toggle model / provider breakdown"),
            ("s / c / v", "Toggle sessions / chart / visual view"),
            ("t", "Sessions: toggle sorting by tokens"),
            ("↑↓ / j k", "Scroll one line"),
            ("PgUp / PgDn", "Scroll ten lines"),
            ("Home / Esc", "Close help and return to top"),
            ("End", "Scroll to bottom"),
        ],
    }
}

fn help_rows(help: &HelpModal) -> Vec<(&'static str, &'static str)> {
    let mut rows = help_commands(help.section).to_vec();
    if !help.pi
        && matches!(
            help.section,
            HelpSection::Home | HelpSection::AllEnabled | HelpSection::Provider
        )
    {
        rows.push(("D (Shift+d)", "Disconnect current client; Codex asks for confirmation. Claude preferences also use Alt+X."));
    }
    if help.pi
        && matches!(
            help.section,
            HelpSection::Home | HelpSection::Provider | HelpSection::AllEnabled
        )
    {
        rows.retain(|(key, _)| {
            !matches!(
                *key,
                "Space" | "A" | "Space / d / 1" | "A / C" | "Space / status dot"
            )
        });
        rows.extend([
            ("Pi d / 1", "Set default / change context window"),
            (
                "Pi p / P",
                "Set startup provider/model / toggle selected provider's proxy API",
            ),
            (
                "Pi i / s",
                "Reload Pi files / show local configuration status",
            ),
            (
                "Pi Space / A / C / D",
                "Show configuration information; entries have no enable switch",
            ),
        ]);
    }
    if help.codex
        && matches!(
            help.section,
            HelpSection::Home | HelpSection::Provider | HelpSection::AllEnabled
        )
    {
        rows.extend([
            ("Codex F3", "ChatGPT accounts; shortcuts in Accounts"),
            ("Codex g", "Set reasoning effort"),
            (
                "Codex s / D",
                "Local status / disconnect and restore managed settings",
            ),
            (
                "Codex p",
                "Sync API models and startup default, or apply chosen ChatGPT account",
            ),
        ]);
    }
    if help.grok
        && matches!(
            help.section,
            HelpSection::Home | HelpSection::Provider | HelpSection::AllEnabled
        )
    {
        rows.extend([
            ("Grok o / i", "OAuth accounts / import native configuration"),
            ("Grok p / s / D", "Connect / show local status / disconnect"),
            ("Grok P", "Show gateway information"),
        ]);
    }
    rows
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum HelpAction {
    Providers,
    AllModels,
    Provider,
    AddProvider,
    AddModel,
    Usage,
    Refresh,
    Reset,
    Accounts,
    Settings,
    Proxy,
    Sync,
    Disconnect,
}

fn section_action(section: HelpSection) -> Option<HelpAction> {
    Some(match section {
        HelpSection::Home => HelpAction::Providers,
        HelpSection::AllEnabled => HelpAction::AllModels,
        HelpSection::Provider => HelpAction::Provider,
        HelpSection::Forms => HelpAction::AddModel,
        HelpSection::Usage => HelpAction::Usage,
        HelpSection::Accounts => HelpAction::Accounts,
        HelpSection::Settings => HelpAction::Settings,
        HelpSection::Proxy => HelpAction::Proxy,
        HelpSection::Pulse => return None,
    })
}

fn help_actions(help: &HelpModal) -> Vec<(HelpAction, &'static str)> {
    let mut actions = match help.section {
        HelpSection::Home => vec![
            (HelpAction::Providers, "Providers [l]"),
            (HelpAction::AddProvider, "Add Provider [a]"),
        ],
        HelpSection::AllEnabled => vec![
            (HelpAction::AllModels, "All Models [l]"),
            (HelpAction::AddModel, "Add Model [a]"),
        ],
        HelpSection::Provider => vec![
            (HelpAction::Provider, "Provider [l]"),
            (HelpAction::AddModel, "Add Model [a]"),
        ],
        HelpSection::Forms => vec![
            (HelpAction::AddModel, "Add Model [l]"),
            (HelpAction::AddProvider, "Add Provider [a]"),
        ],
        HelpSection::Usage => vec![
            (HelpAction::Usage, "Usage [l]"),
            (HelpAction::Refresh, "Refresh [r]"),
            (HelpAction::Reset, "Reset [x]"),
        ],
        HelpSection::Accounts if help.codex || help.grok => {
            vec![(HelpAction::Accounts, "Accounts [l]")]
        }
        HelpSection::Settings => vec![(HelpAction::Settings, "Settings [l]")],
        HelpSection::Proxy if !help.grok => vec![(HelpAction::Proxy, "Proxy [l]")],
        _ => vec![],
    };
    if matches!(
        help.section,
        HelpSection::Home | HelpSection::AllEnabled | HelpSection::Provider
    ) {
        actions.push((HelpAction::Sync, "Sync / Apply [p]"));
    }
    if !help.pi && help.section != HelpSection::Pulse {
        actions.push((HelpAction::Disconnect, "Disconnect [D]"));
    }
    if help.section != HelpSection::Settings {
        actions.push((HelpAction::Settings, "Settings [F4]"));
    }
    if help.section != HelpSection::Usage {
        actions.push((HelpAction::Usage, "Usage [F6]"));
    }
    actions
}

pub(super) fn help_key_action(help: &HelpModal, key: KeyEvent) -> Option<HelpAction> {
    if !key.modifiers.is_empty() {
        return None;
    }
    let action = match key.code {
        KeyCode::Char('l') | KeyCode::Enter => section_action(help.section)?,
        KeyCode::F(4) => HelpAction::Settings,
        KeyCode::F(6) => HelpAction::Usage,
        KeyCode::Char('D') => HelpAction::Disconnect,
        KeyCode::Char('p') => HelpAction::Sync,
        KeyCode::Char('a') if matches!(help.section, HelpSection::Home | HelpSection::Forms) => {
            HelpAction::AddProvider
        }
        KeyCode::Char('a') => HelpAction::AddModel,
        KeyCode::Char('r') => HelpAction::Refresh,
        KeyCode::Char('x') => HelpAction::Reset,
        _ => return None,
    };
    help_actions(help)
        .iter()
        .any(|(candidate, _)| *candidate == action)
        .then_some(action)
}

impl App {
    pub(super) fn run_help_action(&mut self, action: HelpAction) -> Result<()> {
        if self.codex_navigation_blocked() || self.grok_auth.busy {
            self.set_error(
                "Finish or cancel the current account operation before opening another action",
            );
            return Ok(());
        }
        self.modal = None;
        self.help_return = None;
        if action == HelpAction::Settings {
            self.open_appearance();
            return Ok(());
        }
        self.usage.active = false;
        self.codex_ui.accounts = false;
        self.grok_auth.page = None;
        self.all_models_filter.active = false;
        if let Some(editor) = &mut self.provider_editor {
            editor.search_active = false;
        }
        match action {
            HelpAction::Providers => self.return_home(),
            HelpAction::AllModels => self.enter_all_enabled_view(),
            HelpAction::Provider => {
                if self.view_mode == ViewMode::AllEnabled {
                    self.open_selected_global_model();
                } else if self.selected_profile().is_some() {
                    self.enter_provider_view();
                } else {
                    self.set_error("Select a provider before opening its models");
                }
            }
            HelpAction::AddProvider => self.new_profile(),
            HelpAction::AddModel => self.open_add_model_modal(),
            HelpAction::Usage | HelpAction::Refresh | HelpAction::Reset => {
                self.open_usage();
                if action != HelpAction::Usage {
                    let code = if action == HelpAction::Refresh {
                        'r'
                    } else {
                        'x'
                    };
                    self.usage_key(KeyEvent::new(KeyCode::Char(code), KeyModifiers::NONE));
                }
            }
            HelpAction::Accounts => {
                if self.codex_ui.enabled {
                    self.open_codex_accounts();
                } else if self.grok_enabled {
                    self.open_grok_auth();
                } else {
                    self.set_error("Accounts are available for Codex and Grok");
                }
            }
            HelpAction::Proxy => {
                self.open_appearance();
                self.open_proxy_manager();
            }
            HelpAction::Sync | HelpAction::Disconnect => {
                let code = if action == HelpAction::Sync { 'p' } else { 'D' };
                self.handle_key_inner(KeyEvent::new(KeyCode::Char(code), KeyModifiers::NONE))?;
            }
            HelpAction::Settings => unreachable!(),
        }
        Ok(())
    }
}

fn help_tab_lines(active: HelpSection, width: u16) -> Vec<Line<'static>> {
    let mut rows = vec![];
    let mut spans = vec![];
    let mut used = 0;
    for (index, section) in HelpSection::ALL.iter().enumerate() {
        let label = format!(" {} {} ", index + 1, section.label());
        let len = UnicodeWidthStr::width(label.as_str()) as u16;
        if used > 0 && used + len > width {
            rows.push(Line::from(std::mem::take(&mut spans)));
            used = 0;
        }
        spans.push(Span::styled(
            label,
            if *section == active {
                Style::default()
                    .fg(ROUTE)
                    .bg(theme::PROVIDER_SELECTION)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(MUTED)
            },
        ));
        used += len;
    }
    rows.push(Line::from(spans));
    rows
}

fn format_help_rows(section: HelpSection, rows: &[(&str, &str)], wide: bool) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::styled(
            section.label(),
            Style::default().fg(ROUTE).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
    ];
    for (key, action) in rows {
        if wide {
            lines.push(Line::from(vec![
                Span::styled(format!("{key:<28}"), Style::default().fg(ROUTE)),
                Span::raw(action.to_string()),
            ]));
        } else {
            lines.push(Line::styled(
                key.to_string(),
                Style::default().fg(ROUTE).add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::raw(format!("  {action}")));
        }
    }
    lines
}

pub(super) fn help_tab_at(area: Rect, column: u16, row: u16) -> Option<usize> {
    let inner = panel_inner(area);
    let tabs = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        help_tab_lines(HelpSection::Home, inner.width).len() as u16,
    );
    let mut x = tabs.x;
    let mut y = tabs.y;
    for (index, section) in HelpSection::ALL.iter().enumerate() {
        let width =
            UnicodeWidthStr::width(format!(" {} {} ", index + 1, section.label()).as_str()) as u16;
        if x > tabs.x && x + width > tabs.x + tabs.width {
            x = tabs.x;
            y += 1;
        }
        if y < tabs.y + tabs.height && row == y && column >= x && column < x + width {
            return Some(index);
        }
        x += width;
    }
    None
}

fn help_geometry(
    help: &HelpModal,
    area: Rect,
) -> (Rect, Vec<(HelpAction, &'static str, Rect)>, Rect) {
    let inner = panel_inner(area);
    let tabs_height = help_tab_lines(help.section, inner.width).len() as u16;
    let tabs = Rect::new(inner.x, inner.y, inner.width, tabs_height.min(inner.height));
    let bottom = inner.y + inner.height.saturating_sub(2);
    let mut x = inner.x;
    let mut y = tabs.y + tabs.height + 1;
    let mut actions = vec![];
    for (action, label) in help_actions(help) {
        let width = (UnicodeWidthStr::width(label) as u16 + 2).min(inner.width);
        if x > inner.x && x + width > inner.x + inner.width {
            x = inner.x;
            y += 1;
        }
        if y < bottom {
            actions.push((action, label, Rect::new(x, y, width, 1)));
        }
        x += width + 1;
    }
    let content_y = (y + 2).min(bottom);
    (
        tabs,
        actions,
        Rect::new(
            inner.x,
            content_y,
            inner.width,
            bottom.saturating_sub(content_y),
        ),
    )
}

pub(super) fn help_action_at(
    help: &HelpModal,
    area: Rect,
    column: u16,
    row: u16,
) -> Option<HelpAction> {
    help_geometry(help, area)
        .1
        .into_iter()
        .find(|(_, _, rect)| contains(*rect, column, row))
        .map(|(action, _, _)| action)
}

pub(super) fn help_navigation_at(area: Rect, column: u16, row: u16) -> Option<usize> {
    modal_button_rects(area, 3)
        .iter()
        .position(|rect| contains(*rect, column, row))
}

fn wrapped_help(help: &HelpModal, width: u16) -> Vec<Line<'static>> {
    let source = format_help_rows(help.section, &help_rows(help), width >= 80);
    let mut lines = vec![
        Line::styled(
            "Click an action · l / Enter opens this section · h / Esc returns",
            Style::default().fg(MUTED),
        ),
        Line::styled(
            "1–9 / click: categories · Tab / Shift+Tab: switch · j k / Pg / Home End: scroll",
            Style::default().fg(MUTED),
        ),
        Line::raw(""),
    ];
    lines.extend(source);
    lines
        .into_iter()
        .flat_map(|line| {
            wrap_styled_segments(
                line.spans
                    .into_iter()
                    .map(|s| (s.content.into_owned(), s.style))
                    .collect(),
                width.max(1),
            )
        })
        .collect()
}

pub(super) fn help_scroll_limit(help: &HelpModal, area: Rect) -> u16 {
    let (_, _, content) = help_geometry(help, area);
    wrapped_help(help, content.width)
        .len()
        .saturating_sub(content.height as usize)
        .min(u16::MAX as usize) as u16
}

pub(super) fn draw_help(
    frame: &mut ratatui::Frame,
    area: Rect,
    help: &HelpModal,
    theme: theme::Theme,
) {
    frame.render_widget(
        panel(&format!(" Help · {} ", help.section.label()), true),
        area,
    );
    let (tabs, actions, content) = help_geometry(help, area);
    frame.render_widget(
        Paragraph::new(help_tab_lines(help.section, tabs.width)),
        tabs,
    );
    for (action, label, rect) in actions {
        let color = if action == HelpAction::Disconnect {
            ERROR
        } else {
            ROUTE
        };
        frame.render_widget(
            Paragraph::new(toolbar::action_line(label, color, false, theme)),
            rect,
        );
    }
    let lines = wrapped_help(help, content.width);
    let limit = lines
        .len()
        .saturating_sub(content.height as usize)
        .min(u16::MAX as usize) as u16;
    frame.render_widget(
        Paragraph::new(lines).scroll((help.scroll.min(limit), 0)),
        content,
    );
    if area.height >= 12 {
        draw_modal_buttons(
            frame,
            area,
            &["Previous [Shift+Tab]", "Next [Tab]", "Back [h / Esc]"],
        );
    }
}
