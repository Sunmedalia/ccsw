use super::*;
use crate::codex as service;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender},
};

enum Update {
    Message(String),
    Done(std::result::Result<String, String>),
}
#[derive(Clone)]
enum Input {
    Login,
    Device,
    Import,
    ImportFile,
    Rename(String),
    Reasoning,
    Delete(String),
    Disconnect,
}
pub(super) struct CodexUi {
    pub enabled: bool,
    pub accounts: bool,
    selected: usize,
    pub busy: bool,
    pub cancel: Arc<AtomicBool>,
    sender: Sender<Update>,
    receiver: Receiver<Update>,
    input: Option<Input>,
    field: String,
    pub help: bool,
    help_scroll: u16,
    message: String,
}
impl Default for CodexUi {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            enabled: false,
            accounts: false,
            selected: 0,
            busy: false,
            cancel: Arc::new(AtomicBool::new(false)),
            sender,
            receiver,
            input: None,
            field: String::new(),
            help: false,
            help_scroll: 0,
            message: "n login · i import current · p use · r refresh · F3 API Providers".into(),
        }
    }
}
impl App {
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
            let _ = sender.send(Update::Done(result));
        });
    }
    pub(super) fn poll_codex(&mut self) -> bool {
        let mut changed = false;
        while let Ok(update) = self.codex_ui.receiver.try_recv() {
            changed = true;
            match update {
                Update::Message(message) => {
                    self.codex_ui.message = message;
                }
                Update::Done(result) => {
                    self.codex_ui.busy = false;
                    self.status_error = result.is_err();
                    self.codex_ui.message = result.unwrap_or_else(|e| e);
                    self.status = self.codex_ui.message.clone();
                    if let Ok(config) = config::load(&self.paths.config) {
                        self.config = config;
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
    fn selected_codex_account(&self) -> Option<String> {
        self.config
            .codex
            .accounts
            .keys()
            .nth(self.codex_ui.selected)
            .cloned()
    }
    fn refresh_codex_account(&mut self) {
        if let Some(id) = self.selected_codex_account() {
            self.codex_job(move |paths, _, _| {
                service::accounts::refresh(&paths, &id)?;
                Ok("Account limits refreshed".into())
            });
        }
    }
    pub(super) fn apply_codex(&mut self) {
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
        self.codex_ui.input = Some(input);
        self.codex_ui.field = initial;
    }
    pub(super) fn codex_navigation_blocked(&self) -> bool {
        self.codex_ui.busy || self.codex_ui.help || self.codex_ui.input.is_some()
    }
    pub(super) fn handle_codex_key(&mut self, key: KeyEvent) -> Result<Option<bool>> {
        if self.codex_ui.help {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter => {
                    self.codex_ui.help = false
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.codex_ui.help_scroll = (self.codex_ui.help_scroll + 1).min(18)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.codex_ui.help_scroll = self.codex_ui.help_scroll.saturating_sub(1)
                }
                KeyCode::PageDown => {
                    self.codex_ui.help_scroll = (self.codex_ui.help_scroll + 5).min(18)
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
                    self.codex_job(move |paths, cancel, sender| match input {
                        Input::Login | Input::Device => {
                            let device = matches!(input, Input::Device);
                            let id =
                                service::accounts::login(&paths, &text, device, &cancel, |m| {
                                    let _ = sender.send(Update::Message(m));
                                })?;
                            Ok(format!(
                                "Account {id} saved · select it and press p to switch"
                            ))
                        }
                        Input::Import => {
                            let id = service::accounts::import(&paths, &text, None)?;
                            Ok(format!("Imported {id}"))
                        }
                        Input::ImportFile => {
                            let path = std::path::PathBuf::from(&text);
                            let name = path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("Imported account");
                            let id = service::accounts::import(&paths, name, Some(&path))?;
                            Ok(format!("Imported {id} · e rename"))
                        }
                        Input::Rename(id) => {
                            service::accounts::rename(&paths, &id, &text)?;
                            Ok("Account renamed".into())
                        }
                        Input::Reasoning => {
                            service::validate_reasoning(&text)?;
                            config::update(&paths.config, |c| {
                                c.codex.reasoning_effort = Some(text);
                                Ok(())
                            })?;
                            Ok("Reasoning saved · p apply to Codex".into())
                        }
                        Input::Delete(id) => {
                            if text != "delete" {
                                anyhow::bail!("Deletion cancelled");
                            }
                            service::accounts::remove(&paths, &id)?;
                            Ok("Saved account removed".into())
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
                    self.refresh_codex_account();
                }
                return Ok(Some(false));
            }
            KeyCode::Char('?') => {
                self.codex_ui.help = true;
                self.codex_ui.help_scroll = 0;
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
                self.codex_ui.message =
                    service::status(&self.paths).unwrap_or_else(|e| e.to_string());
                self.status = self.codex_ui.message.clone();
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
            KeyCode::Char('n') => self.codex_input(Input::Login, String::new()),
            KeyCode::Char('N') => self.codex_input(Input::Device, String::new()),
            KeyCode::Char('i') => self.codex_input(Input::Import, String::new()),
            KeyCode::Char('I') => self.codex_input(Input::ImportFile, String::new()),
            KeyCode::Char('e') => {
                if let Some(id) = self.selected_codex_account() {
                    self.codex_input(
                        Input::Rename(id.clone()),
                        self.config.codex.accounts[&id].name.clone(),
                    );
                }
            }
            KeyCode::Char('x') => {
                if let Some(id) = self.selected_codex_account() {
                    self.codex_input(Input::Delete(id), String::new());
                }
            }
            KeyCode::Char('r') => self.refresh_codex_account(),
            KeyCode::Char('p') | KeyCode::Enter => {
                if let Some(id) = self.selected_codex_account() {
                    self.codex_job(move |paths, _, _| {
                        service::accounts::activate(&paths, &id)?;
                        Ok(
                            "Account selected · restart CLI / ChatGPT App · s checks on-disk state"
                                .into(),
                        )
                    });
                }
            }
            _ => {}
        }
        Ok(Some(false))
    }
    pub(super) fn codex_mouse(&mut self, mouse: MouseEvent, area: Rect) -> Result<bool> {
        if !self.codex_ui.enabled {
            return Ok(false);
        }
        if self.codex_ui.help {
            if matches!(mouse.kind, MouseEventKind::ScrollDown) {
                self.codex_ui.help_scroll = (self.codex_ui.help_scroll + 1).min(18);
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
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                self.codex_ui.selected = (self.codex_ui.selected + 1)
                    .min(self.config.codex.accounts.len().saturating_sub(1))
            }
            MouseEventKind::ScrollUp => {
                self.codex_ui.selected = self.codex_ui.selected.saturating_sub(1)
            }
            MouseEventKind::Down(MouseButton::Left)
                if mouse.row > area.y + 2 && mouse.row < area.y + area.height / 2 =>
            {
                let rows = Layout::vertical([
                    Constraint::Length(2),
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
                let index = (mouse.row - area.y - 3) as usize + offset;
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
            Constraint::Length(2),
            Constraint::Percentage(40),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(area);
        frame.render_widget(
            Paragraph::new("CCSW · Codex Accounts\nF2 Pi · F3 API Providers · ? Help")
                .style(Style::default().fg(ROUTE)),
            rows[0],
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
                    "{} {} · {} · {}",
                    if active { "●" } else { "○" },
                    account.name,
                    account.email,
                    account.plan.as_deref().unwrap_or("unknown")
                ))
            })
            .collect::<Vec<_>>();
        let mut state = ListState::default().with_selected(Some(self.codex_ui.selected));
        frame.render_stateful_widget(
            List::new(items)
                .block(panel(" Accounts · n add · e edit · p use ", true))
                .highlight_style(Style::default().bg(SELECTION)),
            rows[1],
            &mut state,
        );
        let details = self
            .selected_codex_account()
            .and_then(|id| service::accounts::summary(&self.paths, &id).ok())
            .unwrap_or(
                "No subscription accounts.\nn: browser login · i: import current login".into(),
            );
        frame.render_widget(
            Paragraph::new(details)
                .block(panel(" Subscription & limits · r refresh ", false))
                .wrap(Wrap { trim: false }),
            rows[2],
        );
        frame.render_widget(
            Paragraph::new(self.codex_ui.message.as_str())
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(WARNING)),
            rows[3],
        );
        self.draw_codex_overlay(frame, area);
    }
    pub(super) fn draw_codex_overlay(&self, frame: &mut ratatui::Frame, area: Rect) {
        if !self.codex_ui.enabled {
            return;
        }
        let title = self.codex_ui.input.as_ref().map(|input| match input {
            Input::Login => "Browser login · account name",
            Input::Device => "Device login · account name",
            Input::Import => "Import current login · account name",
            Input::ImportFile => "Import auth.json · absolute file path",
            Input::Rename(_) => "Rename account",
            Input::Reasoning => "Reasoning: none/minimal/low/medium/high/xhigh",
            Input::Delete(_) => "Type delete to remove saved account",
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
        } else if self.codex_ui.help {
            let popup = Rect::new(
                area.x + 1,
                area.y + 1,
                area.width.saturating_sub(2),
                area.height.saturating_sub(2),
            );
            frame.render_widget(Clear, popup);
            let text = "F2  Claude / Codex / Pi\nF3  API Providers / Accounts\nAPI: n new provider · Enter models\ne edit provider on Home; model on model page\nE edit provider · a add model · p apply\ng reasoning · s status · D disconnect\nAccounts: n browser login · N device login\ni import current · I import auth.json\ne rename · x delete · p/Enter switch\nr refresh limits · s status\nEsc cancel login / back · ? close Help\n\nSwitching writes shared configuration.\nRestart CLI / ChatGPT App and open a new chat.\nLimits are cached; refresh failures keep old data.";
            frame.render_widget(
                Paragraph::new(text)
                    .block(panel(" Codex Help · ↑↓ scroll ", true))
                    .wrap(Wrap { trim: false })
                    .scroll((self.codex_ui.help_scroll, 0)),
                popup,
            );
        }
    }
}
