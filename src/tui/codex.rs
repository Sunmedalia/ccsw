use super::*;
use crate::codex as service;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender},
};

enum Update {
    Select(String),
    Login(Option<String>, String),
    Done(std::result::Result<String, String>),
}
#[derive(Clone)]
enum Input {
    Import,
    ImportFile,
    Reasoning,
    Disconnect,
}
pub(super) struct CodexUi {
    pub enabled: bool,
    pub accounts: bool,
    selected: usize,
    pending_selection: Option<String>,
    chosen_account: Option<String>,
    live_id: Option<String>,
    live_message: String,
    pub busy: bool,
    pub cancel: Arc<AtomicBool>,
    sender: Sender<Update>,
    receiver: Receiver<Update>,
    input: Option<Input>,
    field: String,
    status_view: bool,
    help_scroll: u16,
    scroll_max: std::cell::Cell<u16>,
    message: String,
}
impl Default for CodexUi {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            enabled: false,
            accounts: false,
            selected: 0,
            pending_selection: None,
            chosen_account: None,
            live_id: None,
            live_message: "s inspect local login".into(),
            busy: false,
            cancel: Arc::new(AtomicBool::new(false)),
            sender,
            receiver,
            input: None,
            field: String::new(),
            status_view: false,
            help_scroll: 0,
            scroll_max: std::cell::Cell::new(200),
            message: "i import current · I import file · Space select · p apply · Esc providers"
                .into(),
        }
    }
}
impl App {
    pub(super) fn open_codex_accounts(&mut self) {
        if !self.codex_ui.accounts {
            let _ = self.handle_codex_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE));
        }
    }
    pub(super) fn chatgpt_provider_lines(&self, width: u16) -> Vec<Line<'static>> {
        let selected = matches!(
            self.config.codex.active,
            Some(service::Selection::Account { .. })
        );
        let name = self
            .chosen_codex_account()
            .and_then(|id| self.config.codex.accounts.get(&id))
            .map(|account| account.name.as_str())
            .unwrap_or("Select an account");
        let mut lines = wrap_styled_segments(
            vec![
                (
                    (if selected { " ● " } else { " ○ " }).into(),
                    Style::default().fg(if selected { CONNECTED } else { MUTED }),
                ),
                (
                    "ChatGPT Account".into(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                ("  [ChatGPT]".into(), Style::default().fg(ROUTE)),
            ],
            width,
        );
        for (text, color) in [
            (
                format!(
                    "     Account: {name}   {} saved",
                    self.config.codex.accounts.len()
                ),
                WARNING,
            ),
            ("     Provider: ChatGPT".into(), MUTED),
            ("     Credential: Saved Codex login".into(), MUTED),
        ] {
            lines.extend(wrap_styled_segments(
                vec![(text, Style::default().fg(color))],
                width,
            ));
        }
        lines.push(Line::raw(""));
        lines
    }

    fn apply_codex_account(&mut self) {
        let Some(id) = self.chosen_codex_account() else {
            self.set_error("No saved account selected · Enter Account, import a login, then press Space to select");
            return;
        };
        self.codex_job(move |paths, _, _| {
            service::accounts::activate(&paths, &id)?;
            Ok("Codex account applied · restart CLI / Codex App and open a new chat".into())
        });
    }

    fn codex_job(
        &mut self,
        work: impl FnOnce(AppPaths, Arc<AtomicBool>, Sender<Update>) -> Result<String> + Send + 'static,
    ) {
        if self.codex_ui.busy {
            self.codex_ui.message = "Another Codex operation is running · Esc cancels login".into();
            return;
        }
        self.codex_ui.busy = true;
        self.codex_ui.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.codex_ui.cancel.clone();
        let sender = self.codex_ui.sender.clone();
        let paths = self.paths.clone();
        self.codex_ui.message = "Working…".into();
        self.status = "Codex: working…".into();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                work(paths, cancel, sender.clone())
            }))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("Codex operation failed unexpectedly")))
            .map_err(|error| format!("{error:#}"));
            let (id, message) = service::accounts::live_login()
                .unwrap_or_else(|e| (None, format!("Cannot read local login: {e}")));
            let _ = sender.send(Update::Login(id, message));
            let _ = sender.send(Update::Done(result));
        });
    }
    pub(super) fn poll_codex(&mut self) -> bool {
        let mut changed = false;
        while let Ok(update) = self.codex_ui.receiver.try_recv() {
            changed = true;
            match update {
                Update::Select(id) => self.codex_ui.pending_selection = Some(id),
                Update::Login(id, message) => {
                    self.codex_ui.live_id = id;
                    self.codex_ui.live_message = message;
                }
                Update::Done(result) => {
                    self.codex_ui.busy = false;
                    self.status_error = result.is_err();
                    self.codex_ui.message = result.unwrap_or_else(|e| e);
                    self.status = self.codex_ui.message.clone();
                    if let Ok(config) =
                        config::load_client(&self.paths.config, config::Client::Codex)
                    {
                        self.config = config;
                    }
                    if let Some(id) = self.codex_ui.pending_selection.take() {
                        self.codex_ui.selected = self
                            .config
                            .codex
                            .accounts
                            .keys()
                            .position(|key| key == &id)
                            .unwrap_or(self.codex_ui.selected);
                    }
                    self.codex_ui.selected = self
                        .codex_ui
                        .selected
                        .min(self.config.codex.accounts.len().saturating_sub(1));
                }
            }
        }
        changed
    }
    fn chosen_codex_account(&self) -> Option<String> {
        self.codex_ui
            .chosen_account
            .clone()
            .or_else(|| match &self.config.codex.active {
                Some(service::Selection::Account { id }) => Some(id.clone()),
                _ => None,
            })
            .filter(|id| self.config.codex.accounts.contains_key(id))
    }

    fn selected_codex_account(&self) -> Option<String> {
        if !self.codex_ui.accounts
            && let Some(service::Selection::Account { id }) = &self.config.codex.active
            && self.config.codex.accounts.contains_key(id)
        {
            return Some(id.clone());
        }
        self.config
            .codex
            .accounts
            .keys()
            .nth(self.codex_ui.selected)
            .cloned()
    }
    pub(super) fn apply_codex(&mut self) {
        if self.view_mode == ViewMode::Home
            && (self.home_all_selected || self.config.profiles.is_empty())
        {
            self.apply_codex_account();
            return;
        }
        let Some(profile) = self.selected_profile_id() else {
            self.set_error("Select a provider and model to apply to Codex");
            return;
        };
        let model = self.selected_model().map(|m| canonical_model_id(&m.id));
        self.codex_job(move |paths, _, _| {
            service::apply(&paths, &profile, model.as_deref(), None)?;
            Ok("Codex API applied · restart CLI / ChatGPT App and open a new chat".into())
        });
    }
    fn codex_input(&mut self, input: Input, initial: String) {
        if self.codex_ui.busy {
            return;
        }
        self.codex_ui.input = Some(input);
        self.codex_ui.field = initial;
    }
    pub(super) fn codex_navigation_blocked(&self) -> bool {
        self.codex_ui.busy || self.codex_ui.status_view || self.codex_ui.input.is_some()
    }
    pub(super) fn handle_codex_key(&mut self, key: KeyEvent) -> Result<Option<bool>> {
        if self.codex_ui.status_view {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter => {
                    self.codex_ui.status_view = false;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.codex_ui.help_scroll =
                        (self.codex_ui.help_scroll + 1).min(self.codex_ui.scroll_max.get())
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.codex_ui.help_scroll = self.codex_ui.help_scroll.saturating_sub(1)
                }
                KeyCode::PageDown => {
                    self.codex_ui.help_scroll =
                        (self.codex_ui.help_scroll + 5).min(self.codex_ui.scroll_max.get())
                }
                KeyCode::PageUp => {
                    self.codex_ui.help_scroll = self.codex_ui.help_scroll.saturating_sub(5)
                }
                _ => {}
            }
            return Ok(Some(false));
        }
        if let Some(input) = self.codex_ui.input.clone() {
            match key.code {
                KeyCode::Esc => self.codex_ui.input = None,
                KeyCode::Backspace => {
                    self.codex_ui.field.pop();
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.codex_ui.field.clear()
                }
                KeyCode::Char(c) => self.codex_ui.field.push(c),
                KeyCode::Enter => {
                    let text = self.codex_ui.field.trim().to_owned();
                    self.codex_ui.input = None;
                    self.codex_job(move |paths, _, sender| match input {
                        Input::Import => {
                            let id = service::accounts::import(&paths, &text, None)?;
                            let _ = sender.send(Update::Select(id));
                            Ok("Account imported and highlighted · Space select · p apply".into())
                        }
                        Input::ImportFile => {
                            let path = std::path::PathBuf::from(&text);
                            let name = path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("Imported account");
                            let id = service::accounts::import(&paths, name, Some(&path))?;
                            let _ = sender.send(Update::Select(id));
                            Ok("Account imported and highlighted · Space select · p apply".into())
                        }
                        Input::Reasoning => {
                            service::validate_reasoning(&text)?;
                            config::update(&paths.config, |c| {
                                c.codex.reasoning_effort = Some(text);
                                Ok(())
                            })?;
                            Ok("Reasoning saved · p apply to Codex".into())
                        }
                        Input::Disconnect => {
                            if text != "disconnect" {
                                anyhow::bail!("Disconnect cancelled");
                            }
                            service::disconnect(&paths)?;
                            Ok("Previous Codex settings restored · restart clients".into())
                        }
                    });
                }
                _ => {}
            }
            return Ok(Some(false));
        }
        if !self.codex_ui.enabled {
            return Ok(None);
        }
        if key.code == KeyCode::Enter
            && !self.codex_ui.accounts
            && self.view_mode == ViewMode::Home
            && (self.home_all_selected || self.config.profiles.is_empty())
        {
            self.open_codex_accounts();
            return Ok(Some(false));
        }
        match key.code {
            KeyCode::F(3) => {
                self.codex_ui.accounts = !self.codex_ui.accounts;
                if self.codex_ui.accounts
                    && let Some(service::Selection::Account { id }) = &self.config.codex.active
                {
                    self.codex_ui.selected = self
                        .config
                        .codex
                        .accounts
                        .keys()
                        .position(|v| v == id)
                        .unwrap_or(0);
                }
                if self.codex_ui.accounts {
                    let sender = self.codex_ui.sender.clone();
                    std::thread::spawn(move || {
                        let (id, message) = service::accounts::live_login()
                            .unwrap_or_else(|e| (None, format!("Cannot read local login: {e}")));
                        let _ = sender.send(Update::Login(id, message));
                    });
                }
                return Ok(Some(false));
            }
            KeyCode::Char('?') => {
                self.open_help();
                return Ok(Some(false));
            }
            KeyCode::Char('g') if !self.codex_ui.accounts => {
                self.codex_input(
                    Input::Reasoning,
                    self.config
                        .codex
                        .reasoning_effort
                        .clone()
                        .unwrap_or("medium".into()),
                );
                return Ok(Some(false));
            }
            KeyCode::Char('s') => {
                self.codex_ui.status_view = true;
                self.codex_ui.help_scroll = 0;
                self.codex_job(|paths, _, _| service::status(&paths));
                return Ok(Some(false));
            }
            KeyCode::Char('D') => {
                self.codex_input(Input::Disconnect, String::new());
                return Ok(Some(false));
            }
            _ => {}
        }
        if !self.codex_ui.accounts {
            return Ok(None);
        }
        match key.code {
            KeyCode::Char('q') => {
                self.codex_ui.cancel.store(true, Ordering::Relaxed);
                return Ok(Some(true));
            }
            KeyCode::Esc if self.codex_ui.busy => {
                self.codex_ui.cancel.store(true, Ordering::Relaxed)
            }
            KeyCode::Esc => self.codex_ui.accounts = false,
            KeyCode::Down | KeyCode::Char('j') => {
                self.codex_ui.selected = (self.codex_ui.selected + 1)
                    .min(self.config.codex.accounts.len().saturating_sub(1))
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.codex_ui.selected = self.codex_ui.selected.saturating_sub(1)
            }
            KeyCode::Char('i') => self.codex_input(Input::Import, String::new()),
            KeyCode::Char('I') => self.codex_input(Input::ImportFile, String::new()),
            KeyCode::Char(' ') => {
                self.codex_ui.chosen_account = self.selected_codex_account();
                self.codex_ui.message = "Account selected · p / Apply Codex to apply".into();
            }
            KeyCode::Char('p') => {
                self.apply_codex_account();
            }
            _ => {}
        }
        Ok(Some(false))
    }
    pub(super) fn codex_mouse(&mut self, mouse: MouseEvent, area: Rect) -> Result<bool> {
        if !self.codex_ui.enabled || self.modal.is_some() {
            return Ok(false);
        }
        if self.codex_ui.status_view {
            if matches!(mouse.kind, MouseEventKind::ScrollDown) {
                self.codex_ui.help_scroll =
                    (self.codex_ui.help_scroll + 1).min(self.codex_ui.scroll_max.get());
            }
            if matches!(mouse.kind, MouseEventKind::ScrollUp) {
                self.codex_ui.help_scroll = self.codex_ui.help_scroll.saturating_sub(1);
            }
            return Ok(true);
        }
        if self.codex_ui.input.is_some() {
            return Ok(true);
        }
        if !self.codex_ui.accounts {
            return Ok(false);
        }
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            if mouse.row == area.y + 1 && mouse.column < area.x + 14 {
                self.codex_ui.accounts = false;
                return Ok(true);
            }
            for (key, _, rect) in account_buttons(area) {
                if mouse.column >= rect.x && mouse.column < rect.right() && mouse.row == rect.y {
                    self.handle_codex_key(KeyEvent::new(
                        if key == '\u{1b}' {
                            KeyCode::Esc
                        } else {
                            KeyCode::Char(key)
                        },
                        KeyModifiers::NONE,
                    ))?;
                    return Ok(true);
                }
            }
        }
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                self.codex_ui.selected = (self.codex_ui.selected + 1)
                    .min(self.config.codex.accounts.len().saturating_sub(1))
            }
            MouseEventKind::ScrollUp => {
                self.codex_ui.selected = self.codex_ui.selected.saturating_sub(1)
            }
            MouseEventKind::Down(MouseButton::Left) if mouse.row > area.y + 4 => {
                let rows = Layout::vertical([
                    Constraint::Length(4),
                    Constraint::Percentage(40),
                    Constraint::Min(3),
                    Constraint::Length(3),
                ])
                .split(area);
                let visible = rows[1].height.saturating_sub(2) as usize;
                let offset = self
                    .codex_ui
                    .selected
                    .saturating_sub(visible.saturating_sub(1));
                if mouse.row >= rows[1].bottom().saturating_sub(1) {
                    return Ok(true);
                }
                let index = (mouse.row - rows[1].y - 1) as usize + offset;
                if index < self.config.codex.accounts.len() {
                    self.codex_ui.selected = index;
                }
            }
            _ => {}
        }
        Ok(true)
    }
    pub(super) fn draw_codex_accounts(&self, frame: &mut ratatui::Frame, area: Rect) {
        let rows = Layout::vertical([
            Constraint::Length(4),
            Constraint::Percentage(40),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(area);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " ‹ Back (Esc) ",
                    self.footer_control_style(FooterControl::Back, false, false)
                        .1,
                ),
                Span::styled(
                    "  ChatGPT Account",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ])),
            Rect::new(rows[0].x, rows[0].y + 1, rows[0].width, 1),
        );
        self.draw_client_tabs(frame, rows[0]);
        frame.render_widget(
            Paragraph::new(self.codex_ui.live_message.as_str()).wrap(Wrap { trim: false }),
            Rect::new(
                rows[0].x,
                rows[0].y + 2,
                rows[0].width,
                rows[0].height.saturating_sub(2),
            ),
        );
        let items = self
            .config
            .codex
            .accounts
            .iter()
            .map(|(id, account)| {
                let active = self.config.codex.active
                    == Some(service::Selection::Account { id: id.clone() });
                ListItem::new(format!(
                    "{}{} {} · {}",
                    if self.chosen_codex_account().as_ref() == Some(id) {
                        "[●] "
                    } else {
                        "[○] "
                    },
                    if active {
                        "[Applied]"
                    } else if self.codex_ui.live_id.as_ref() == Some(id) {
                        "[Local login]"
                    } else {
                        "○"
                    },
                    account.name,
                    account.email
                ))
            })
            .collect::<Vec<_>>();
        let mut state = ListState::default().with_selected(Some(self.codex_ui.selected));
        frame.render_stateful_widget(
            List::new(items)
                .block(panel(" ChatGPT accounts ", true))
                .highlight_style(
                    Style::default()
                        .fg(Color::White)
                        .bg(SELECTION)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol(" "),
            rows[1],
            &mut state,
        );
        let details = self.selected_codex_account()
            .and_then(|id| self.config.codex.accounts.get(&id))
            .map(|account| format!("{}\n{}\n\nSpace selects an account; p / Apply Codex applies it.\nAPI providers use their own endpoint and model.\nNo account or quota requests are made.", account.name, account.email))
            .unwrap_or("Import your current Codex login, or import an auth.json file. Then select an account and press Space, then p.".into());
        let details = format!("{}\n\n{details}", self.codex_ui.message);
        frame.render_widget(
            Paragraph::new(details)
                .block(panel(" Switch account ", false))
                .wrap(Wrap { trim: false }),
            rows[2],
        );
        for (_, label, rect) in account_buttons(area) {
            frame.render_widget(
                Paragraph::new(label).alignment(Alignment::Center).style(
                    self.footer_control_style(
                        if label.contains("Import") || label.contains("File") {
                            FooterControl::AddProfile
                        } else if label.contains("Back") {
                            FooterControl::Back
                        } else {
                            FooterControl::Sync
                        },
                        false,
                        false,
                    )
                    .1,
                ),
                rect,
            );
        }
        self.draw_codex_overlay(frame, area);
    }
    pub(super) fn draw_codex_overlay(&self, frame: &mut ratatui::Frame, area: Rect) {
        if !self.codex_ui.enabled {
            return;
        }
        let title = self.codex_ui.input.as_ref().map(|input| match input {
            Input::Import => "Import current login · account name",
            Input::ImportFile => "Import auth.json · absolute file path",
            Input::Reasoning => "Reasoning: none/minimal/low/medium/high/xhigh",
            Input::Disconnect => "Type disconnect to restore previous configuration",
        });
        if let Some(title) = title {
            let popup = Rect::new(
                area.x + 2,
                area.y + area.height / 3,
                area.width.saturating_sub(4),
                5.min(area.height),
            );
            frame.render_widget(Clear, popup);
            frame.render_widget(
                Paragraph::new(format!(
                    "{}\nEnter save · Esc cancel · Ctrl+U clear",
                    self.codex_ui.field
                ))
                .block(panel(title, true))
                .wrap(Wrap { trim: false }),
                popup,
            );
        } else if self.codex_ui.status_view {
            let popup = Rect::new(
                area.x + 1,
                area.y + 1,
                area.width.saturating_sub(2),
                area.height.saturating_sub(2),
            );
            frame.render_widget(Clear, popup);
            let text = self.codex_ui.message.as_str();
            let title = " Codex Status · ↑↓ scroll · Esc close ";
            let max_scroll = text
                .lines()
                .map(|line| {
                    use unicode_width::UnicodeWidthStr;
                    line.width()
                        .max(1)
                        .div_ceil(popup.width.saturating_sub(2).max(1) as usize)
                })
                .sum::<usize>()
                .saturating_sub(popup.height.saturating_sub(2) as usize)
                .min(u16::MAX as usize) as u16;
            self.codex_ui.scroll_max.set(max_scroll);
            frame.render_widget(
                Paragraph::new(text)
                    .block(panel(title, true))
                    .wrap(Wrap { trim: false })
                    .scroll((self.codex_ui.help_scroll.min(max_scroll), 0)),
                popup,
            );
        }
    }
}

// Shared hit regions and rendering keep mouse actions aligned at every width.
fn account_buttons(area: Rect) -> Vec<(char, &'static str, Rect)> {
    let labels = [
        ('i', "Import (i)"),
        ('I', "File (I)"),
        ('p', "Apply Codex"),
        ('\u{1b}', "‹ Back"),
    ];
    let button_width = area.width.saturating_sub(3).saturating_div(4).min(14);
    let slots = Layout::horizontal([Constraint::Length(button_width); 4])
        .spacing(1)
        .split(Rect::new(
            area.x,
            area.bottom().saturating_sub(3),
            area.width,
            1,
        ));
    labels
        .into_iter()
        .zip(slots.iter())
        .map(|((key, label), rect)| (key, label, *rect))
        .collect()
}
