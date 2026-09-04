use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    io,
};

use anyhow::{Context, Result};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
        MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Wrap,
    },
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    claude_config,
    config::{self, ApiFormat, AppPaths, Config, Credential, ModelEntry, Profile, RoleModels},
    discovery::{self, CachedModels, ModelCache},
    import::ImportCandidate,
    proxy,
    runner::{self, LaunchRequest, SessionCapture, SessionMode},
    state::{self, ProjectState, SessionRecord},
};

const ROUTE: Color = Color::Rgb(95, 215, 215);
const CONNECTED: Color = Color::Rgb(135, 215, 135);
const WARNING: Color = Color::Rgb(255, 215, 95);
const ERROR: Color = Color::Rgb(255, 107, 107);
const MUTED: Color = Color::Rgb(128, 138, 148);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Home,
    Provider,
    AllEnabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Profiles,
    Models,
    Details,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchMode {
    Resume,
    New,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MouseAction {
    None,
    Launch,
    Quit,
}

#[derive(Debug, Clone, Copy)]
struct UiAreas {
    profiles: Option<Rect>,
    models: Option<Rect>,
    details: Option<Rect>,
    footer: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FooterControl {
    Back,
    Models,
    Details,
    Launch,
    AddProfile,
    Resume,
    New,
    Sync,
    Proxy,
    Help,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetailControl {
    FetchModels,
    Edit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShowcaseControl {
    Toggle,
    Default,
    OneM,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProxyControl {
    Start,
    Stop,
    Refresh,
    EnableAtLogin,
    DisableAtLogin,
    Close,
}

enum Modal {
    Import(Box<ImportCandidate>),
    Profile(Box<ProfileForm>),
    Model(ModelForm),
    DeleteProfile,
    DeleteModel,
    Proxy(ProxyManager),
    Help(HelpModal),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HelpSection {
    Home,
    AllEnabled,
    Provider,
    Forms,
}

impl HelpSection {
    const ALL: [Self; 4] = [Self::Home, Self::AllEnabled, Self::Provider, Self::Forms];

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|section| *section == self)
            .unwrap_or_default()
    }

    fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::AllEnabled => "All Enabled",
            Self::Provider => "Provider",
            Self::Forms => "Forms",
        }
    }
}

struct HelpModal {
    section: HelpSection,
    scroll: u16,
}

impl HelpModal {
    fn for_view(view_mode: ViewMode) -> Self {
        let section = match view_mode {
            ViewMode::Home => HelpSection::Home,
            ViewMode::AllEnabled => HelpSection::AllEnabled,
            ViewMode::Provider => HelpSection::Provider,
        };
        Self { section, scroll: 0 }
    }

    fn move_section(&mut self, forward: bool) {
        let len = HelpSection::ALL.len();
        let current = self.section.index();
        let next = if forward {
            (current + 1) % len
        } else {
            (current + len - 1) % len
        };
        self.section = HelpSection::ALL[next];
        self.scroll = 0;
    }

    fn select(&mut self, index: usize) {
        if let Some(section) = HelpSection::ALL.get(index) {
            self.section = *section;
            self.scroll = 0;
        }
    }

    fn scroll(&mut self, down: bool) {
        if down {
            self.scroll = self.scroll.saturating_add(1).min(32);
        } else {
            self.scroll = self.scroll.saturating_sub(1);
        }
    }
}

struct ProxyManager {
    runtime: Option<proxy::ProxyStatus>,
    service: Option<proxy::ProxyServiceStatus>,
    selected: usize,
    message: String,
    error: bool,
}

struct ProfileForm {
    original_id: Option<String>,
    original_profile: Option<Profile>,
    provider_enabled: bool,
    models: Vec<ModelEntry>,
    enabled_models: Vec<String>,
    disabled_models: Vec<String>,
    fields: Vec<FormField>,
    selected: usize,
}

struct ModelForm {
    fields: Vec<FormField>,
    selected: usize,
    api_models: Vec<ModelEntry>,
    api_query: String,
    api_query_cursor: usize,
    api_scroll: usize,
    api_selected: usize,
    focus_api_search: bool,
    api_status: String,
}

struct RouteEditor {
    profile_id: String,
    original_profile: Profile,
    provider_enabled: bool,
    catalog: Vec<ModelEntry>,
    enabled: BTreeSet<String>,
    disabled: BTreeSet<String>,
    locked: BTreeSet<String>,
    default_model: String,
    one_m: BTreeSet<String>,
    query: String,
    selected: usize,
    search_active: bool,
    status: String,
}

#[derive(Debug, Clone)]
struct GlobalModelRef {
    profile_id: String,
    profile_name: String,
    model: ModelEntry,
    enabled: bool,
}

struct FormField {
    label: &'static str,
    value: String,
    cursor: usize,
    secret: bool,
    toggle: bool,
    choices: &'static [&'static str],
}

pub struct App {
    paths: AppPaths,
    config: Config,
    cache: ModelCache,
    cwd: String,
    view_mode: ViewMode,
    home_all_selected: bool,
    profile_idx: usize,
    model_idx: usize,
    profile_offset: usize,
    model_offset: usize,
    focus: Focus,
    launch_mode: LaunchMode,
    status: String,
    status_error: bool,
    modal: Option<Modal>,
    current_session: Option<SessionCapture>,
    proxy_status: Option<proxy::ProxyStatus>,
    forwarded_args: Vec<OsString>,
    provider_editor: Option<RouteEditor>,
}

pub fn run(
    paths: AppPaths,
    config: Config,
    forwarded_args: Vec<OsString>,
    import: Option<ImportCandidate>,
) -> Result<()> {
    let cwd = state::canonical_project(&std::env::current_dir()?);
    let saved = state::load(&paths.state).unwrap_or_default();
    let project = saved.projects.get(&cwd);
    let profile_ids: Vec<_> = config.profiles.keys().cloned().collect();
    let profile_idx = project
        .and_then(|project| profile_ids.iter().position(|id| id == &project.profile_id))
        .unwrap_or(0);
    let selected_profile_id = profile_ids
        .get(profile_idx)
        .map(String::as_str)
        .unwrap_or("");
    let current_session = saved
        .latest_session(&cwd, selected_profile_id)
        .map(|session| SessionCapture {
            session_id: session.session_id.clone(),
            model_id: session.model_id.clone(),
            cwd: Some(session.cwd.clone()),
            event: Some("saved".into()),
        });
    let proxy_status = proxy::status(&paths).ok();
    let mut app = App {
        cache: discovery::load_cache(&paths.cache),
        paths,
        config,
        cwd,
        view_mode: ViewMode::Home,
        home_all_selected: false,
        profile_idx,
        model_idx: 0,
        profile_offset: 0,
        model_offset: 0,
        focus: Focus::Profiles,
        launch_mode: if current_session.is_some() {
            LaunchMode::Resume
        } else {
            LaunchMode::New
        },
        status: "↑↓ 选择 · Space 启用/禁用厂商 · Enter 查看或管理".into(),
        status_error: false,
        modal: None,
        current_session,
        proxy_status,
        forwarded_args,
        provider_editor: None,
    };
    app.select_saved_model(project);
    if app.config.profiles.is_empty()
        && let Some(candidate) = import
    {
        app.modal = Some(Modal::Import(Box::new(candidate)));
    }

    let mut terminal = setup_terminal()?;
    let result = app.event_loop(&mut terminal);
    restore_terminal(&mut terminal)?;
    result
}

type TuiTerminal = Terminal<CrosstermBackend<io::Stdout>>;

fn setup_terminal() -> Result<TuiTerminal> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    terminal.clear()?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut TuiTerminal) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

impl App {
    fn event_loop(&mut self, terminal: &mut TuiTerminal) -> Result<()> {
        loop {
            terminal.draw(|frame| self.draw(frame))?;
            match event::read()? {
                Event::Key(key) if key.kind == event::KeyEventKind::Press => {
                    if self.modal.is_some() {
                        self.handle_modal(key)?;
                        continue;
                    }
                    match self.view_mode {
                        ViewMode::Home => match key.code {
                            KeyCode::Char('q') => return Ok(()),
                            KeyCode::Char('?') => self.open_help(),
                            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
                            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
                            KeyCode::Enter if self.home_all_selected => {
                                self.enter_all_enabled_view();
                            }
                            KeyCode::Enter if self.selected_profile().is_some() => {
                                self.enter_provider_view();
                            }
                            KeyCode::Char('n') => self.new_profile(),
                            KeyCode::Char('e') | KeyCode::Char('E') => self.edit_profile(),
                            KeyCode::Char('x') if self.selected_profile().is_some() => {
                                self.modal = Some(Modal::DeleteProfile);
                            }
                            KeyCode::Char('r') | KeyCode::Char('t') => self.refresh_models(),
                            KeyCode::Char(' ') if self.selected_profile().is_some() => {
                                self.toggle_selected_provider()?;
                            }
                            KeyCode::Char('m') => self.toggle_launch_mode(),
                            KeyCode::Char('R') => {
                                if self.current_session.is_some() {
                                    self.launch_selected(terminal, false)?;
                                } else {
                                    self.set_error(
                                        "No saved session to resume; press Enter or N for a new session",
                                    );
                                }
                            }
                            KeyCode::Char('N') => self.launch_selected(terminal, true)?,
                            KeyCode::Char('A') => self.enable_all_models(),
                            KeyCode::Char('p') => self.sync_all_to_claude(),
                            KeyCode::Char('P') => self.open_proxy_manager(),
                            _ => {}
                        },
                        ViewMode::AllEnabled => match key.code {
                            KeyCode::Char('q') => return Ok(()),
                            KeyCode::Char('?') => self.open_help(),
                            KeyCode::Esc => self.return_home(),
                            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
                            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
                            KeyCode::PageUp => {
                                self.model_idx = self.model_idx.saturating_sub(10);
                            }
                            KeyCode::PageDown => {
                                self.model_idx = (self.model_idx + 10)
                                    .min(self.all_managed_models().len().saturating_sub(1));
                            }
                            KeyCode::Home => self.model_idx = 0,
                            KeyCode::End => {
                                self.model_idx = self.all_managed_models().len().saturating_sub(1);
                            }
                            KeyCode::Char(' ') => {
                                self.toggle_selected_global_model()?;
                            }
                            KeyCode::Enter => self.open_selected_global_model(),
                            KeyCode::Char('p') => self.sync_all_to_claude(),
                            KeyCode::Char('P') => self.open_proxy_manager(),
                            _ => {}
                        },
                        ViewMode::Provider => {
                            let mut search_active = false;
                            if let Some(editor) = &self.provider_editor {
                                search_active = editor.search_active;
                            }
                            if search_active {
                                let editor = self.ensure_provider_editor().unwrap();
                                match key.code {
                                    KeyCode::Esc => {
                                        if !editor.query.is_empty() {
                                            editor.query.clear();
                                            editor.selected = 0;
                                        } else {
                                            editor.search_active = false;
                                        }
                                    }
                                    KeyCode::Char(c) => {
                                        editor.query.push(c);
                                        editor.selected = 0;
                                    }
                                    KeyCode::Backspace => {
                                        editor.query.pop();
                                        editor.selected = 0;
                                    }
                                    KeyCode::Tab | KeyCode::Down => {
                                        editor.search_active = false;
                                    }
                                    KeyCode::Enter => {
                                        editor.toggle_selected();
                                        self.commit_provider_editor()?;
                                    }
                                    KeyCode::Up if editor.selected > 0 => {
                                        editor.selected -= 1;
                                    }
                                    _ => {}
                                }
                            } else {
                                match key.code {
                                    KeyCode::Char('q') => return Ok(()),
                                    KeyCode::Char('?') => self.open_help(),
                                    KeyCode::Esc => {
                                        self.return_home();
                                    }
                                    KeyCode::Tab | KeyCode::BackTab => self.toggle_focus(),
                                    KeyCode::Left | KeyCode::Char('h') => {
                                        self.focus = Focus::Models;
                                    }
                                    KeyCode::Right | KeyCode::Char('l') => {
                                        self.focus = Focus::Details;
                                    }
                                    KeyCode::Char('/') => {
                                        if let Some(editor) = self.ensure_provider_editor() {
                                            editor.search_active = true;
                                        }
                                    }
                                    KeyCode::Up | KeyCode::Char('k') => {
                                        let name =
                                            if let Some(editor) = self.ensure_provider_editor() {
                                                let filtered = editor.filtered_indices();
                                                if !filtered.is_empty() {
                                                    if editor.selected > 0 {
                                                        editor.selected -= 1;
                                                    } else {
                                                        editor.selected =
                                                            filtered.len().saturating_sub(1);
                                                    }
                                                    filtered.get(editor.selected).map(|&idx| {
                                                        editor.catalog[idx].label().to_string()
                                                    })
                                                } else {
                                                    None
                                                }
                                            } else {
                                                None
                                            };
                                        if let Some(name) = name {
                                            self.status_error = false;
                                            self.status = format!(
                                                "Selected {name} · Enter run · Space toggle · d default · 1 1M"
                                            );
                                        }
                                    }
                                    KeyCode::Down | KeyCode::Char('j') => {
                                        let name =
                                            if let Some(editor) = self.ensure_provider_editor() {
                                                let filtered = editor.filtered_indices();
                                                if !filtered.is_empty() {
                                                    if editor.selected + 1 < filtered.len() {
                                                        editor.selected += 1;
                                                    } else {
                                                        editor.selected = 0;
                                                    }
                                                    filtered.get(editor.selected).map(|&idx| {
                                                        editor.catalog[idx].label().to_string()
                                                    })
                                                } else {
                                                    None
                                                }
                                            } else {
                                                None
                                            };
                                        if let Some(name) = name {
                                            self.status_error = false;
                                            self.status = format!(
                                                "Selected {name} · Enter run · Space toggle · d default · 1 1M"
                                            );
                                        }
                                    }
                                    KeyCode::PageUp => {
                                        if let Some(editor) = self.ensure_provider_editor() {
                                            editor.selected = editor.selected.saturating_sub(10);
                                        }
                                    }
                                    KeyCode::PageDown => {
                                        if let Some(editor) = self.ensure_provider_editor() {
                                            let filtered = editor.filtered_indices();
                                            editor.selected = (editor.selected + 10)
                                                .min(filtered.len().saturating_sub(1));
                                        }
                                    }
                                    KeyCode::Home => {
                                        if let Some(editor) = self.ensure_provider_editor() {
                                            editor.selected = 0;
                                        }
                                    }
                                    KeyCode::End => {
                                        if let Some(editor) = self.ensure_provider_editor() {
                                            let filtered = editor.filtered_indices();
                                            editor.selected = filtered.len().saturating_sub(1);
                                        }
                                    }
                                    KeyCode::Char(' ') => {
                                        if let Some(editor) = self.ensure_provider_editor() {
                                            editor.toggle_selected();
                                        }
                                        self.commit_provider_editor()?;
                                        if let Some(editor) = &self.provider_editor {
                                            let filtered = editor.filtered_indices();
                                            if let Some(&idx) = filtered.get(editor.selected) {
                                                let model = &editor.catalog[idx];
                                                let enabled = editor.is_enabled(&model.id);
                                                self.status = format!(
                                                    "Model {} is now {}",
                                                    model.label(),
                                                    if enabled { "enabled" } else { "disabled" }
                                                );
                                            }
                                        }
                                    }
                                    KeyCode::Char('1') => {
                                        self.toggle_selected_model_1m();
                                    }
                                    KeyCode::Char('d') => {
                                        self.set_selected_as_default();
                                    }
                                    KeyCode::Enter => {
                                        if let Some(editor) = self.ensure_provider_editor() {
                                            let filtered = editor.filtered_indices();
                                            if let Some(&idx) = filtered.get(editor.selected) {
                                                let model_id = editor.catalog[idx].id.clone();
                                                if !editor.is_enabled(&model_id) {
                                                    editor.disabled.remove(&model_id);
                                                    editor.enabled.insert(model_id);
                                                }
                                            }
                                        }
                                        self.commit_provider_editor()?;
                                        self.launch_selected(
                                            terminal,
                                            self.launch_mode == LaunchMode::New,
                                        )?;
                                    }
                                    KeyCode::Char('m') => self.toggle_launch_mode(),
                                    KeyCode::Char('R') => {
                                        if let Some(editor) = self.ensure_provider_editor() {
                                            let filtered = editor.filtered_indices();
                                            if let Some(&idx) = filtered.get(editor.selected) {
                                                let model_id = editor.catalog[idx].id.clone();
                                                if !editor.is_enabled(&model_id) {
                                                    editor.disabled.remove(&model_id);
                                                    editor.enabled.insert(model_id);
                                                }
                                            }
                                        }
                                        self.commit_provider_editor()?;
                                        if self.current_session.is_some() {
                                            self.launch_selected(terminal, false)?;
                                        } else {
                                            self.set_error(
                                                "No saved session to resume; press Enter or N for a new session",
                                            );
                                        }
                                    }
                                    KeyCode::Char('N') => {
                                        if let Some(editor) = self.ensure_provider_editor() {
                                            let filtered = editor.filtered_indices();
                                            if let Some(&idx) = filtered.get(editor.selected) {
                                                let model_id = editor.catalog[idx].id.clone();
                                                if !editor.is_enabled(&model_id) {
                                                    editor.disabled.remove(&model_id);
                                                    editor.enabled.insert(model_id);
                                                }
                                            }
                                        }
                                        self.commit_provider_editor()?;
                                        self.launch_selected(terminal, true)?;
                                    }
                                    KeyCode::Char('A') => {
                                        if let Some(editor) = self.ensure_provider_editor() {
                                            editor.enable_all_filtered();
                                        }
                                        self.commit_provider_editor()?;
                                        self.status = "Enabled all filtered models".into();
                                    }
                                    KeyCode::Char('C') => {
                                        if let Some(editor) = self.ensure_provider_editor() {
                                            editor.disable_all_filtered();
                                        }
                                        self.commit_provider_editor()?;
                                        self.status = "Cleared non-essential enabled models".into();
                                    }
                                    KeyCode::Char('a') if self.selected_profile().is_some() => {
                                        self.open_add_model_modal();
                                    }
                                    KeyCode::Char('x') => {
                                        self.delete_selected_model();
                                    }
                                    KeyCode::Char('e') | KeyCode::Char('E') => self.edit_profile(),
                                    KeyCode::Char('r') | KeyCode::Char('t') => {
                                        self.refresh_models();
                                        self.init_provider_editor();
                                    }
                                    KeyCode::Char('p') => self.sync_all_to_claude(),
                                    KeyCode::Char('P') => self.open_proxy_manager(),
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                Event::Mouse(mouse) => {
                    let size = terminal.size()?;
                    let area = Rect::new(0, 0, size.width, size.height);
                    match self.handle_mouse(mouse, area)? {
                        MouseAction::None => {}
                        MouseAction::Launch => {
                            self.launch_selected(terminal, self.launch_mode == LaunchMode::New)?
                        }
                        MouseAction::Quit => return Ok(()),
                    }
                }
                _ => {}
            }
        }
    }

    fn profile_ids(&self) -> Vec<String> {
        self.config.profiles.keys().cloned().collect()
    }

    fn selected_profile_id(&self) -> Option<String> {
        if self.view_mode == ViewMode::AllEnabled
            || (self.view_mode == ViewMode::Home && self.home_all_selected)
        {
            return None;
        }
        self.profile_ids().get(self.profile_idx).cloned()
    }

    fn selected_profile(&self) -> Option<&Profile> {
        self.selected_profile_id()
            .and_then(|id| self.config.profiles.get(&id))
    }

    fn home_profile_item_heights(&self, panel: Rect) -> Vec<usize> {
        let mut heights = vec![
            all_enabled_lines(
                self.config
                    .profiles
                    .values()
                    .filter(|profile| profile.enabled)
                    .count(),
                self.all_enabled_model_count(),
                panel.width.saturating_sub(2),
            )
            .len(),
        ];
        heights.extend(self.profile_ids().iter().map(|id| {
            let profile = &self.config.profiles[id];
            let discovered = self
                .cache
                .profiles
                .get(id)
                .map(|cached| cached.models.as_slice())
                .unwrap_or_default();
            home_profile_lines(
                id,
                profile,
                discovery::active_models(profile, discovered).len(),
                panel.width.saturating_sub(2),
            )
            .len()
        }));
        heights
    }

    fn home_selected_index(&self) -> usize {
        if self.home_all_selected || self.config.profiles.is_empty() {
            0
        } else {
            self.profile_idx.saturating_add(1)
        }
    }

    fn select_home_index(&mut self, index: usize) {
        if index == 0 {
            self.home_all_selected = true;
            self.model_idx = 0;
            self.current_session = None;
        } else {
            self.home_all_selected = false;
            self.profile_idx = (index - 1).min(self.config.profiles.len().saturating_sub(1));
            self.model_idx = self.default_model_index();
            self.sync_session_for_profile();
        }
        self.model_offset = 0;
    }

    fn models(&self) -> Vec<ModelEntry> {
        let Some(id) = self.selected_profile_id() else {
            return vec![];
        };
        let profile = &self.config.profiles[&id];
        let discovered = self
            .cache
            .profiles
            .get(&id)
            .map(|cached| cached.models.as_slice())
            .unwrap_or_default();
        discovery::active_models(profile, discovered)
    }

    fn catalog_models(&self) -> Vec<ModelEntry> {
        let Some(id) = self.selected_profile_id() else {
            return vec![];
        };
        self.catalog_models_for(&id)
    }

    fn catalog_models_for(&self, id: &str) -> Vec<ModelEntry> {
        let profile = &self.config.profiles[id];
        let discovered = self
            .cache
            .profiles
            .get(id)
            .map(|cached| cached.models.as_slice())
            .unwrap_or_default();
        discovery::configured_models(profile, discovered)
    }

    fn all_enabled_model_count(&self) -> usize {
        self.config
            .profiles
            .iter()
            .map(|(profile_id, profile)| {
                let discovered = self
                    .cache
                    .profiles
                    .get(profile_id)
                    .map(|cached| cached.models.as_slice())
                    .unwrap_or_default();
                discovery::active_models(profile, discovered).len()
            })
            .sum()
    }

    fn all_managed_models(&self) -> Vec<GlobalModelRef> {
        let mut models = Vec::new();
        for (profile_id, profile) in self
            .config
            .profiles
            .iter()
            .filter(|(_, profile)| profile.enabled)
        {
            let discovered = self
                .cache
                .profiles
                .get(profile_id)
                .map(|cached| cached.models.as_slice())
                .unwrap_or_default();
            let active = discovery::active_models(profile, discovered)
                .into_iter()
                .map(|model| canonical_model_id(&model.id))
                .collect::<BTreeSet<_>>();
            let Some(editor) = self.create_route_editor_for(profile_id.clone()) else {
                continue;
            };
            for catalog_model in &editor.catalog {
                let mut model = catalog_model.clone();
                model.id = editor.effective_id(&model.id);
                models.push(GlobalModelRef {
                    profile_id: profile_id.clone(),
                    profile_name: profile.name.clone(),
                    enabled: active.contains(&canonical_model_id(&model.id)),
                    model,
                });
            }
        }
        models
    }

    fn selected_model(&self) -> Option<ModelEntry> {
        if self.view_mode == ViewMode::Provider {
            let fallback;
            let editor = match &self.provider_editor {
                Some(e) => e,
                None => {
                    fallback = self.create_route_editor();
                    fallback.as_ref()?
                }
            };
            let filtered = editor.filtered_indices();
            let &idx = filtered.get(editor.selected)?;
            let model = &editor.catalog[idx];
            let effective = editor.effective_id(&model.id);
            Some(ModelEntry {
                id: effective,
                label: model.label.clone(),
                description: model.description.clone(),
            })
        } else if self.view_mode == ViewMode::AllEnabled {
            self.all_managed_models()
                .get(self.model_idx)
                .map(|entry| entry.model.clone())
        } else {
            self.models().get(self.model_idx).cloned()
        }
    }

    fn toggle_focus(&mut self) {
        if self.view_mode == ViewMode::Home {
            self.focus = Focus::Profiles;
        } else if self.view_mode == ViewMode::AllEnabled {
            self.focus = Focus::Models;
        } else {
            self.focus = match self.focus {
                Focus::Models => Focus::Details,
                _ => Focus::Models,
            };
        }
    }

    fn toggle_launch_mode(&mut self) {
        self.launch_mode = match self.launch_mode {
            LaunchMode::Resume => LaunchMode::New,
            LaunchMode::New if self.current_session.is_some() => LaunchMode::Resume,
            LaunchMode::New => {
                self.status = "No saved session yet; the next launch will be new".into();
                LaunchMode::New
            }
        };
    }

    fn handle_mouse(&mut self, mouse: MouseEvent, area: Rect) -> Result<MouseAction> {
        if self.modal.is_some() {
            self.handle_modal_mouse(mouse, area)?;
            return Ok(MouseAction::None);
        }

        let ui = ui_areas(area, self.focus, self.view_mode);
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let delta = if mouse.kind == MouseEventKind::ScrollUp {
                    -1
                } else {
                    1
                };
                if ui
                    .profiles
                    .is_some_and(|panel| contains(panel, mouse.column, mouse.row))
                {
                    self.focus = Focus::Profiles;
                    self.move_selection(delta);
                } else if ui
                    .models
                    .is_some_and(|panel| contains(panel, mouse.column, mouse.row))
                {
                    self.focus = Focus::Models;
                    if self.view_mode == ViewMode::Provider {
                        if let Some(editor) = self.ensure_provider_editor() {
                            let filtered = editor.filtered_indices();
                            if !filtered.is_empty() {
                                if delta < 0 {
                                    if editor.selected > 0 {
                                        editor.selected -= 1;
                                    }
                                } else if editor.selected + 1 < filtered.len() {
                                    editor.selected += 1;
                                }
                            }
                        }
                    } else {
                        self.move_selection(delta);
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) => {
                if self.view_mode != ViewMode::Home && mouse.row <= 2 && mouse.column <= 16 {
                    self.return_home();
                    return Ok(MouseAction::None);
                }

                if let Some(panel) = ui.profiles {
                    let visible = if self.view_mode == ViewMode::Home {
                        visible_variable_items(
                            &self.home_profile_item_heights(panel),
                            self.profile_offset,
                            usize::from(panel.height.saturating_sub(2)),
                        )
                    } else {
                        usize::from(panel.height.saturating_sub(2))
                    };
                    if let Some(index) = scrollbar_index(
                        panel,
                        mouse.column,
                        mouse.row,
                        if self.view_mode == ViewMode::Home {
                            self.config.profiles.len().saturating_add(1)
                        } else {
                            self.config.profiles.len()
                        },
                        visible,
                    ) {
                        self.focus = Focus::Profiles;
                        if self.view_mode == ViewMode::Home {
                            self.select_home_index(index);
                        } else {
                            self.profile_idx = index;
                            self.model_idx = self.default_model_index();
                            self.model_offset = 0;
                            self.sync_session_for_profile();
                        }
                        return Ok(MouseAction::None);
                    }
                }
                if let Some(panel) = ui.models {
                    if self.view_mode == ViewMode::Provider {
                        let search_area = Rect::new(panel.x, panel.y, panel.width, 3);
                        let list_area = Rect::new(
                            panel.x,
                            panel.y + 3,
                            panel.width,
                            panel.height.saturating_sub(3),
                        );
                        if let Some(btn_rect) = catalog_add_button_rect(search_area)
                            && contains(btn_rect, mouse.column, mouse.row)
                        {
                            self.open_add_model_modal();
                            return Ok(MouseAction::None);
                        }
                        if contains(search_area, mouse.column, mouse.row) {
                            self.focus = Focus::Models;
                            if let Some(editor) = self.ensure_provider_editor() {
                                editor.search_active = true;
                            }
                            return Ok(MouseAction::None);
                        }
                        if contains(list_area, mouse.column, mouse.row) {
                            self.focus = Focus::Models;
                            let clicked = if let Some(editor) = self.ensure_provider_editor() {
                                let offset =
                                    route_editor_offset(editor, list_area.height.saturating_sub(2));
                                let inner_y = list_area.y + 1;
                                if mouse.row >= inner_y {
                                    let index =
                                        offset + usize::from(mouse.row.saturating_sub(inner_y));
                                    let filtered = editor.filtered_indices();
                                    if index < filtered.len() {
                                        editor.selected = index;
                                        let should_toggle = mouse.column < list_area.x + 4;
                                        if should_toggle {
                                            editor.toggle_selected();
                                        } else {
                                            editor.search_active = false;
                                        }
                                        Some((index, should_toggle))
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            };
                            if let Some((index, should_toggle)) = clicked {
                                self.model_idx = index;
                                if should_toggle {
                                    let _ = self.commit_provider_editor();
                                }
                                return Ok(MouseAction::None);
                            }
                        }
                    } else {
                        let models = self.models();
                        if let Some(index) = scrollbar_index(
                            panel,
                            mouse.column,
                            mouse.row,
                            models.len(),
                            usize::from(panel.height.saturating_sub(2) / 2),
                        ) {
                            self.focus = Focus::Models;
                            self.model_idx = index;
                            self.status_error = false;
                            self.status = format!(
                                "Selected {} · Enter or click Launch",
                                models[index].label()
                            );
                            return Ok(MouseAction::None);
                        }
                    }
                }

                if mouse.kind == MouseEventKind::Drag(MouseButton::Left) {
                    return Ok(MouseAction::None);
                }
                for (control, rect) in footer_controls(ui.footer, area.width < 100, self.view_mode)
                {
                    if !contains(rect, mouse.column, mouse.row) {
                        continue;
                    }
                    return Ok(match control {
                        FooterControl::Back => {
                            self.return_home();
                            MouseAction::None
                        }
                        FooterControl::Models => {
                            self.focus = Focus::Models;
                            MouseAction::None
                        }
                        FooterControl::Details => {
                            self.focus = Focus::Details;
                            MouseAction::None
                        }
                        FooterControl::Launch => {
                            if self.view_mode == ViewMode::Home {
                                if self.home_all_selected {
                                    self.enter_all_enabled_view();
                                } else if self.selected_profile().is_some() {
                                    self.enter_provider_view();
                                }
                                MouseAction::None
                            } else {
                                MouseAction::Launch
                            }
                        }
                        FooterControl::AddProfile => {
                            self.new_profile();
                            MouseAction::None
                        }
                        FooterControl::Resume => {
                            if self.current_session.is_some() {
                                self.launch_mode = LaunchMode::Resume;
                            } else {
                                self.set_error("There is no session to resume yet");
                            }
                            MouseAction::None
                        }
                        FooterControl::New => {
                            self.launch_mode = LaunchMode::New;
                            MouseAction::None
                        }
                        FooterControl::Sync => {
                            self.sync_all_to_claude();
                            MouseAction::None
                        }
                        FooterControl::Proxy => {
                            self.open_proxy_manager();
                            MouseAction::None
                        }
                        FooterControl::Help => {
                            self.open_help();
                            MouseAction::None
                        }
                        FooterControl::Quit => MouseAction::Quit,
                    });
                }

                if let Some(panel) = ui.profiles {
                    let index = if self.view_mode == ViewMode::Home {
                        clicked_variable_item(
                            panel,
                            mouse.row,
                            self.profile_offset,
                            &self.home_profile_item_heights(panel),
                        )
                    } else {
                        clicked_list_index(panel, mouse.column, mouse.row, self.profile_offset, 1)
                    };
                    let item_count = if self.view_mode == ViewMode::Home {
                        self.config.profiles.len().saturating_add(1)
                    } else {
                        self.config.profiles.len()
                    };
                    if let Some(index) = index.filter(|index| *index < item_count) {
                        self.focus = Focus::Profiles;
                        if self.view_mode == ViewMode::Home {
                            if index > 0 && mouse.column < panel.x.saturating_add(5) {
                                self.select_home_index(index);
                                self.toggle_selected_provider()?;
                                return Ok(MouseAction::None);
                            }
                            if self.home_selected_index() == index {
                                if index == 0 {
                                    self.enter_all_enabled_view();
                                } else {
                                    self.enter_provider_view();
                                }
                            } else {
                                self.select_home_index(index);
                            }
                        } else {
                            self.profile_idx = index;
                            self.model_idx = self.default_model_index();
                            self.model_offset = 0;
                            self.sync_session_for_profile();
                        }
                    }
                } else if let Some(panel) = ui.models
                    && self.view_mode != ViewMode::Provider
                    && let Some(index) =
                        clicked_list_index(panel, mouse.column, mouse.row, self.model_offset, 2)
                {
                    if self.view_mode == ViewMode::AllEnabled {
                        let models = self.all_managed_models();
                        if let Some(entry) = models.get(index) {
                            self.focus = Focus::Models;
                            self.model_idx = index;
                            if mouse.column < panel.x.saturating_add(4) {
                                self.toggle_selected_global_model()?;
                                return Ok(MouseAction::None);
                            }
                            self.status_error = false;
                            self.status = format!(
                                "{} · {} · Space {} · Enter 打开厂商",
                                entry.profile_name,
                                entry.model.label(),
                                if entry.enabled { "禁用" } else { "启用" }
                            );
                        }
                    } else if index < self.models().len() {
                        self.focus = Focus::Models;
                        self.model_idx = index;
                        if let Some(model) = self.selected_model() {
                            self.status_error = false;
                            self.status =
                                format!("Selected {} · click Launch to start", model.label());
                        }
                    }
                } else if let Some(details) = ui.details
                    && contains(details, mouse.column, mouse.row)
                {
                    self.focus = Focus::Details;
                    if self.view_mode == ViewMode::Provider {
                        let (showcase_card, provider_card) = provider_detail_cards(details);
                        if contains(showcase_card, mouse.column, mouse.row) {
                            if let Some((control, _)) = showcase_controls(showcase_card)
                                .into_iter()
                                .find(|(_, rect)| contains(*rect, mouse.column, mouse.row))
                            {
                                match control {
                                    ShowcaseControl::Toggle => {
                                        if let Some(editor) = self.ensure_provider_editor() {
                                            editor.toggle_selected();
                                        }
                                        self.commit_provider_editor()?;
                                        if let Some(editor) = &self.provider_editor {
                                            let filtered = editor.filtered_indices();
                                            if let Some(&idx) = filtered.get(editor.selected) {
                                                let model = &editor.catalog[idx];
                                                let enabled = editor.is_enabled(&model.id);
                                                self.status = format!(
                                                    "Model {} is now {}",
                                                    model.label(),
                                                    if enabled { "enabled" } else { "disabled" }
                                                );
                                            }
                                        }
                                        return Ok(MouseAction::None);
                                    }
                                    ShowcaseControl::Default => {
                                        self.set_selected_as_default();
                                        return Ok(MouseAction::None);
                                    }
                                    ShowcaseControl::OneM => {
                                        self.toggle_selected_model_1m();
                                        return Ok(MouseAction::None);
                                    }
                                    ShowcaseControl::Delete => {
                                        self.delete_selected_model();
                                        return Ok(MouseAction::None);
                                    }
                                }
                            }
                        } else if contains(provider_card, mouse.column, mouse.row)
                            && let Some((control, _)) = detail_controls(provider_card)
                                .into_iter()
                                .find(|(_, rect)| contains(*rect, mouse.column, mouse.row))
                        {
                            match control {
                                DetailControl::FetchModels => {
                                    self.refresh_models();
                                    self.init_provider_editor();
                                }
                                DetailControl::Edit => {
                                    self.edit_profile();
                                }
                            }
                            return Ok(MouseAction::None);
                        }
                    } else if let Some((control, _)) = detail_controls(details)
                        .into_iter()
                        .find(|(_, rect)| contains(*rect, mouse.column, mouse.row))
                    {
                        match control {
                            DetailControl::FetchModels => {
                                self.refresh_models();
                            }
                            DetailControl::Edit => {
                                self.edit_profile();
                            }
                        }
                        return Ok(MouseAction::None);
                    }
                }
            }
            _ => {}
        }
        Ok(MouseAction::None)
    }

    fn handle_modal_mouse(&mut self, mouse: MouseEvent, screen: Rect) -> Result<()> {
        let Some(modal) = self.modal.as_ref() else {
            return Ok(());
        };
        let area = modal_area_for(modal, screen);
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if let Some(Modal::Model(form)) = self.modal.as_mut() {
                    let inner = panel_inner(area);
                    let content_area = Rect::new(
                        inner.x,
                        inner.y,
                        inner.width,
                        inner.height.saturating_sub(2),
                    );
                    let (_, api_area) = model_form_areas(content_area);
                    let visible_height =
                        usize::from(panel_inner(api_area).height.saturating_sub(2));
                    form.scroll_api_list(false, 3, visible_height);
                    return Ok(());
                }
                self.handle_modal(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))?;
                return Ok(());
            }
            MouseEventKind::ScrollDown => {
                if let Some(Modal::Model(form)) = self.modal.as_mut() {
                    let inner = panel_inner(area);
                    let content_area = Rect::new(
                        inner.x,
                        inner.y,
                        inner.width,
                        inner.height.saturating_sub(2),
                    );
                    let (_, api_area) = model_form_areas(content_area);
                    let visible_height =
                        usize::from(panel_inner(api_area).height.saturating_sub(2));
                    form.scroll_api_list(true, 3, visible_height);
                    return Ok(());
                }
                self.handle_modal(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))?;
                return Ok(());
            }
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) => {}
            _ => return Ok(()),
        }
        if !contains(area, mouse.column, mouse.row) {
            return Ok(());
        }

        if matches!(self.modal, Some(Modal::Proxy(_))) {
            if mouse.kind == MouseEventKind::Drag(MouseButton::Left) {
                return Ok(());
            }
            if let Some((control, _)) = proxy_controls(area)
                .into_iter()
                .find(|(_, rect)| contains(*rect, mouse.column, mouse.row))
            {
                self.handle_modal(proxy_control_key(control))?;
                return Ok(());
            }
        } else if mouse.kind == MouseEventKind::Drag(MouseButton::Left) {
            return Ok(());
        }

        let button_count = match self.modal.as_ref() {
            Some(Modal::Help(_)) => 1,
            Some(Modal::Proxy(_)) => 0,
            Some(Modal::Model(_)) => 3,
            Some(_) => 2,
            None => 0,
        };
        if let Some(button) = modal_button_rects(area, button_count)
            .iter()
            .position(|rect| contains(*rect, mouse.column, mouse.row))
        {
            let key = match (self.modal.as_ref(), button) {
                (Some(Modal::Model(_)), 0) => {
                    self.fetch_api_models_for_form();
                    return Ok(());
                }
                (Some(Modal::Model(_)), 1) => {
                    KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
                }
                (Some(Modal::Model(_)), 2) => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                (Some(Modal::Import(_)), 0)
                | (Some(Modal::DeleteProfile | Modal::DeleteModel), 0) => {
                    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
                }
                (Some(Modal::Profile(_)), 0) => {
                    KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
                }
                (Some(Modal::Help(_)), 0) => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                (Some(_), 1) => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                _ => return Ok(()),
            };
            self.handle_modal(key)?;
            return Ok(());
        }

        let inner = panel_inner(area);
        let clicked_field = contains(inner, mouse.column, mouse.row)
            .then(|| usize::from(mouse.row.saturating_sub(inner.y)));
        match self.modal.as_mut() {
            Some(Modal::Profile(form)) => {
                if let Some(index) = clicked_field
                    && index < form.fields.len()
                {
                    form.selected = index;
                    if !form.fields[index].choices.is_empty() {
                        cycle_choice(&mut form.fields[index], true);
                    }
                }
            }
            Some(Modal::Model(form)) => {
                let content_area = Rect::new(
                    inner.x,
                    inner.y,
                    inner.width,
                    inner.height.saturating_sub(2),
                );
                let (form_area, api_area) = model_form_areas(content_area);
                if contains(form_area, mouse.column, mouse.row) {
                    form.focus_api_search = false;
                    let form_inner = panel_inner(form_area);
                    let clicked_field = contains(form_inner, mouse.column, mouse.row)
                        .then(|| usize::from(mouse.row.saturating_sub(form_inner.y)));
                    if let Some(index) = clicked_field
                        && index < form.fields.len()
                    {
                        form.selected = index;
                        if form.fields[index].toggle {
                            toggle_form_field(&mut form.fields[index]);
                        }
                    }
                } else if contains(api_area, mouse.column, mouse.row) {
                    let api_inner = panel_inner(api_area);
                    if mouse.row == api_inner.y {
                        form.focus_api_search = true;
                    } else if mouse.row >= api_inner.y.saturating_add(2) {
                        let list_row =
                            usize::from(mouse.row.saturating_sub(api_inner.y.saturating_add(2)));
                        let item_idx = form.api_scroll + list_row;
                        form.api_selected = item_idx;
                        form.pick_api_model(item_idx);
                        form.focus_api_search = false;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn move_selection(&mut self, delta: isize) {
        if self.view_mode == ViewMode::Home && self.focus == Focus::Profiles {
            let len = self.config.profiles.len().saturating_add(1);
            let current = self.home_selected_index();
            let next = ((current as isize + delta).rem_euclid(len as isize)) as usize;
            self.select_home_index(next);
            self.status_error = false;
            self.status = if self.home_all_selected {
                format!(
                    "All Enabled · {} models · Enter 管理全部启用模型",
                    self.all_enabled_model_count()
                )
            } else {
                let profile = self.selected_profile().expect("provider selected");
                format!(
                    "Selected {} · Space {}厂商 · Enter 详情",
                    profile.name,
                    if profile.enabled { "禁用" } else { "启用" }
                )
            };
            return;
        }
        let len = match self.focus {
            Focus::Profiles => self.config.profiles.len(),
            Focus::Models if self.view_mode == ViewMode::AllEnabled => {
                self.all_managed_models().len()
            }
            Focus::Models => self.models().len(),
            Focus::Details => return,
        };
        if len == 0 {
            return;
        }
        let current = match self.focus {
            Focus::Profiles => &mut self.profile_idx,
            Focus::Models => &mut self.model_idx,
            Focus::Details => return,
        };
        *current = ((*current as isize + delta).rem_euclid(len as isize)) as usize;
        if self.focus == Focus::Profiles {
            self.model_idx = self.default_model_index();
            self.model_offset = 0;
            self.sync_session_for_profile();
            if let Some(name) = self.selected_profile().map(|profile| profile.name.clone()) {
                self.status_error = false;
                self.status = if self.view_mode == ViewMode::Home {
                    format!("Selected {name} · Enter 进入详情 · e 编辑 · x 删除 · r 测速")
                } else {
                    format!("Selected {name} · Enter to choose a model")
                };
            }
        }
        if self.focus == Focus::Models {
            if self.view_mode == ViewMode::AllEnabled {
                if let Some(entry) = self.all_managed_models().get(self.model_idx) {
                    self.status_error = false;
                    self.status = format!(
                        "{} · {} · Space {} · Enter 打开厂商",
                        entry.profile_name,
                        entry.model.label(),
                        if entry.enabled { "禁用" } else { "启用" }
                    );
                }
            } else if let Some(model) = self.selected_model() {
                self.status_error = false;
                self.status = format!("Selected {} · Enter or click Launch", model.label());
            }
        }
    }

    fn default_model_index(&self) -> usize {
        self.selected_profile()
            .and_then(|profile| {
                self.models()
                    .iter()
                    .position(|model| model.id == profile.default_model)
            })
            .unwrap_or(0)
    }

    fn select_saved_model(&mut self, project: Option<&ProjectState>) {
        let wanted = project
            .map(|project| project.model_id.as_str())
            .or_else(|| {
                self.selected_profile()
                    .map(|profile| profile.default_model.as_str())
            });
        if let Some(wanted) = wanted
            && let Some(index) = self.models().iter().position(|model| model.id == wanted)
        {
            self.model_idx = index;
        }
    }

    fn sync_session_for_profile(&mut self) {
        let selected = self.selected_profile_id();
        let session = selected.as_deref().and_then(|profile_id| {
            state::load(&self.paths.state)
                .ok()
                .and_then(|saved| saved.latest_session(&self.cwd, profile_id).cloned())
        });
        self.current_session = session.map(|session| SessionCapture {
            session_id: session.session_id,
            model_id: session.model_id,
            cwd: Some(session.cwd),
            event: Some("saved".into()),
        });
        self.launch_mode = if self.current_session.is_some() {
            LaunchMode::Resume
        } else {
            LaunchMode::New
        };
    }

    fn new_profile(&mut self) {
        self.modal = Some(Modal::Profile(Box::new(ProfileForm::new())));
    }

    fn create_route_editor(&self) -> Option<RouteEditor> {
        let profile_id = self.selected_profile_id()?;
        self.create_route_editor_for(profile_id)
    }

    fn create_route_editor_for(&self, profile_id: String) -> Option<RouteEditor> {
        let profile = self.config.profiles.get(&profile_id)?;
        let references = profile
            .required_model_ids()
            .into_iter()
            .chain(profile.enabled_models.iter().cloned())
            .chain(profile.disabled_models.iter().cloned())
            .collect::<Vec<_>>();
        let one_m = references
            .iter()
            .filter(|id| has_1m_suffix(id))
            .map(|id| canonical_model_id(id))
            .collect();
        let default_model = canonical_model_id(&profile.default_model);
        let mut locked = profile
            .required_model_ids()
            .iter()
            .map(|id| canonical_model_id(id))
            .collect::<BTreeSet<_>>();
        locked.remove(&default_model);
        let catalog = normalize_model_catalog(self.catalog_models_for(&profile_id));
        let default_idx = catalog
            .iter()
            .position(|m| m.id == default_model)
            .unwrap_or(0);
        let selected = if self.model_idx < catalog.len() && self.model_idx > 0 {
            self.model_idx
        } else {
            default_idx
        };
        Some(RouteEditor {
            profile_id,
            original_profile: (*profile).clone(),
            provider_enabled: profile.enabled,
            catalog,
            enabled: profile
                .enabled_models
                .iter()
                .map(|id| canonical_model_id(id))
                .collect(),
            disabled: profile
                .disabled_models
                .iter()
                .map(|id| canonical_model_id(id))
                .collect(),
            locked,
            default_model,
            one_m,
            query: String::new(),
            selected,
            search_active: false,
            status: "Space 切换启用 · 1 切换 1M · d 设为默认 · Enter 运行".into(),
        })
    }

    fn open_add_model_modal(&mut self) {
        let cached = self
            .selected_profile_id()
            .and_then(|id| self.cache.profiles.get(&id).map(|c| c.models.clone()))
            .unwrap_or_default();
        self.modal = Some(Modal::Model(ModelForm::with_api_models(cached)));
    }

    fn fetch_api_models_for_form(&mut self) {
        let Some(profile_id) = self.selected_profile_id() else {
            return;
        };
        let profile = match self.config.profiles.get(&profile_id) {
            Some(p) => p.clone(),
            None => return,
        };
        match discovery::discover(&profile) {
            Ok(models) => {
                let count = models.len();
                let cached = CachedModels {
                    fetched_at: state::now_epoch(),
                    models: models.clone(),
                };
                let _ = discovery::update_cache(&self.paths.cache, |cache| {
                    cache.profiles.insert(profile_id.clone(), cached);
                });
                self.cache.profiles.insert(
                    profile_id,
                    CachedModels {
                        fetched_at: state::now_epoch(),
                        models: models.clone(),
                    },
                );
                if let Some(Modal::Model(form)) = &mut self.modal {
                    form.api_models = models;
                    form.api_scroll = 0;
                    form.api_selected = 0;
                    form.focus_api_search = true;
                    form.api_status =
                        format!("✓ 从 API 获取到 {count} 个模型 (输入搜索 / 点击填入)");
                }
            }
            Err(e) => {
                if let Some(Modal::Model(form)) = &mut self.modal {
                    form.api_status = format!("✗ API 获取失败: {e:#}");
                }
            }
        }
    }

    fn init_provider_editor(&mut self) {
        self.provider_editor = self.create_route_editor();
    }

    fn ensure_provider_editor(&mut self) -> Option<&mut RouteEditor> {
        if self.provider_editor.is_none() {
            self.provider_editor = self.create_route_editor();
        }
        self.provider_editor.as_mut()
    }

    fn commit_provider_editor(&mut self) -> Result<()> {
        let Some(editor) = &self.provider_editor else {
            return Ok(());
        };
        let mut profile = self
            .config
            .profiles
            .get(&editor.profile_id)
            .cloned()
            .expect("provider profile exists");
        apply_route_editor(&mut profile, editor);
        let id = editor.profile_id.clone();
        self.config = config::update(&self.paths.config, |latest| {
            latest.profiles.insert(id.clone(), profile.clone());
            Ok(())
        })?;
        if let Some(editor) = &mut self.provider_editor {
            editor.original_profile = profile;
        }
        self.sync_session_for_profile();
        Ok(())
    }

    fn enter_provider_view(&mut self) {
        self.view_mode = ViewMode::Provider;
        self.focus = Focus::Models;
        self.init_provider_editor();
        let name = self
            .selected_profile()
            .map(|p| p.name.clone())
            .unwrap_or_default();
        self.status_error = false;
        self.status = format!("Viewing {name} · Enter runs model · Esc back to providers");
    }

    fn enter_all_enabled_view(&mut self) {
        self.view_mode = ViewMode::AllEnabled;
        self.focus = Focus::Models;
        self.model_idx = self
            .model_idx
            .min(self.all_managed_models().len().saturating_sub(1));
        self.model_offset = 0;
        self.status_error = false;
        self.status = "Space 启用/禁用模型 · Enter 打开所属厂商 · Esc 返回".into();
    }

    fn return_home(&mut self) {
        self.view_mode = ViewMode::Home;
        self.focus = Focus::Profiles;
        self.status_error = false;
        self.status = "Returned to Providers home".into();
    }

    fn open_selected_global_model(&mut self) {
        let Some(selected) = self.all_managed_models().get(self.model_idx).cloned() else {
            return;
        };
        let Some(profile_idx) = self
            .profile_ids()
            .iter()
            .position(|id| id == &selected.profile_id)
        else {
            return;
        };
        self.profile_idx = profile_idx;
        self.home_all_selected = false;
        self.view_mode = ViewMode::Provider;
        self.focus = Focus::Models;
        self.provider_editor = self.create_route_editor_for(selected.profile_id);
        if let Some(editor) = &mut self.provider_editor {
            let wanted = canonical_model_id(&selected.model.id);
            if let Some(index) = editor.catalog.iter().position(|model| model.id == wanted) {
                editor.selected = index;
            }
        }
        self.sync_session_for_profile();
        self.status_error = false;
        self.status = format!(
            "Viewing {} · {}",
            selected.profile_name,
            selected.model.label()
        );
    }

    fn edit_profile(&mut self) {
        let Some(id) = self.selected_profile_id() else {
            return;
        };
        let profile = self.config.profiles[&id].clone();
        self.modal = Some(Modal::Profile(Box::new(ProfileForm::edit(id, &profile))));
    }

    fn open_proxy_manager(&mut self) {
        let manager = ProxyManager::load(&self.paths);
        self.proxy_status = manager.runtime.clone();
        self.modal = Some(Modal::Proxy(manager));
    }

    fn open_help(&mut self) {
        self.modal = Some(Modal::Help(HelpModal::for_view(self.view_mode)));
    }

    fn refresh_models(&mut self) {
        let Some(id) = self.selected_profile_id() else {
            self.set_error("Create a profile before refreshing models");
            return;
        };
        let profile = self.config.profiles[&id].clone();
        self.status = format!("Connecting to {}…", profile.base_url);
        match discovery::discover(&profile) {
            Ok(models) => {
                let count = models.len();
                let cached = CachedModels {
                    fetched_at: state::now_epoch(),
                    models,
                };
                match discovery::update_cache(&self.paths.cache, |cache| {
                    cache.profiles.insert(id.clone(), cached);
                }) {
                    Ok(cache) => self.cache = cache,
                    Err(error) => {
                        self.set_error(format!("Models found, but cache failed: {error:#}"));
                        return;
                    }
                }
                {
                    self.status_error = false;
                    self.status = format!("Connection verified · {count} models discovered");
                    self.model_idx = self.model_idx.min(self.models().len().saturating_sub(1));
                    if self.view_mode == ViewMode::Provider {
                        self.init_provider_editor();
                    }
                }
            }
            Err(error) => self.set_error(format!("Connection failed: {error:#}")),
        }
    }

    fn enable_all_models(&mut self) {
        let Some(profile_id) = self.selected_profile_id() else {
            self.set_error("Create a profile before enabling models");
            return;
        };
        let catalog = self.catalog_models();
        if catalog.is_empty() {
            self.set_error("No models are available; fetch or add a model first");
            return;
        }
        let update = config::update(&self.paths.config, |latest| {
            let profile = latest
                .profiles
                .get_mut(&profile_id)
                .context("profile was removed in another CCSW instance")?;
            let required = profile
                .required_model_ids()
                .iter()
                .map(|id| canonical_model_id(id))
                .collect::<BTreeSet<_>>();
            let existing = profile
                .enabled_models
                .iter()
                .chain(profile.required_model_ids().iter())
                .map(|id| (canonical_model_id(id), id.clone()))
                .collect::<BTreeMap<_, _>>();
            profile.enabled_models = catalog
                .iter()
                .filter_map(|model| {
                    let canonical = canonical_model_id(&model.id);
                    (!required.contains(&canonical)).then(|| {
                        existing
                            .get(&canonical)
                            .cloned()
                            .unwrap_or_else(|| model.id.clone())
                    })
                })
                .collect();
            profile.disabled_models.clear();
            Ok(())
        });
        match update {
            Ok(config) => {
                self.config = config;
                self.model_idx = self.model_idx.min(self.models().len().saturating_sub(1));
                self.status_error = false;
                self.status = format!(
                    "Enabled all {} models in {profile_id} · click Sync all to Claude",
                    self.models().len()
                );
            }
            Err(error) => self.set_error(format!("Could not enable all models: {error:#}")),
        }
    }

    fn sync_all_to_claude(&mut self) {
        match self.apply_enabled_to_claude() {
            Ok(result) => {
                self.proxy_status = proxy::status(&self.paths).ok();
                self.status_error = false;
                self.status = format!(
                    "Synced {} models from {} enabled profiles to Claude /model",
                    result.model_count,
                    self.config
                        .profiles
                        .values()
                        .filter(|profile| profile.enabled)
                        .count()
                );
            }
            Err(error) => self.set_error(format!("Could not sync Claude: {error:#}")),
        }
    }

    fn toggle_selected_provider(&mut self) -> Result<()> {
        let Some(profile_id) = self.selected_profile_id() else {
            return Ok(());
        };
        let enabled = !self.config.profiles[&profile_id].enabled;
        self.config = config::update(&self.paths.config, |latest| {
            let profile = latest
                .profiles
                .get_mut(&profile_id)
                .context("profile was removed in another CCSW instance")?;
            profile.enabled = enabled;
            if enabled {
                let default = canonical_model_id(&profile.default_model);
                profile
                    .disabled_models
                    .retain(|id| canonical_model_id(id) != default);
            }
            Ok(())
        })?;
        self.model_idx = 0;
        self.model_offset = 0;
        self.sync_session_for_profile();
        let action = if enabled { "Enabled" } else { "Disabled" };
        match self.apply_enabled_to_claude() {
            Ok(result) => {
                self.proxy_status = proxy::status(&self.paths).ok();
                self.status_error = false;
                self.status = format!(
                    "{action} provider {profile_id} · Claude /model now has {} models",
                    result.model_count
                );
            }
            Err(error) => self.set_error(format!(
                "{action} provider {profile_id}, but Claude sync failed: {error:#}"
            )),
        }
        Ok(())
    }

    fn toggle_selected_global_model(&mut self) -> Result<()> {
        let Some(selected) = self.all_managed_models().get(self.model_idx).cloned() else {
            return Ok(());
        };
        let profile = self.toggled_global_model_profile(&selected)?;
        let profile_id = selected.profile_id.clone();
        self.config = config::update(&self.paths.config, |latest| {
            latest.profiles.insert(profile_id.clone(), profile.clone());
            Ok(())
        })?;
        let remaining = self.all_managed_models().len();
        self.model_idx = self.model_idx.min(remaining.saturating_sub(1));
        let action = if selected.enabled {
            "Disabled"
        } else {
            "Enabled"
        };
        match self.apply_enabled_to_claude() {
            Ok(result) => {
                self.proxy_status = proxy::status(&self.paths).ok();
                self.status_error = false;
                self.status = format!(
                    "{action} {} · {} · Claude /model now has {} models",
                    selected.profile_name,
                    selected.model.label(),
                    result.model_count
                );
            }
            Err(error) => self.set_error(format!(
                "{action} {} · {}, but Claude sync failed: {error:#}",
                selected.profile_name,
                selected.model.label()
            )),
        }
        Ok(())
    }

    fn toggled_global_model_profile(&self, selected: &GlobalModelRef) -> Result<Profile> {
        let mut editor = self
            .create_route_editor_for(selected.profile_id.clone())
            .context("provider no longer exists")?;
        let wanted = canonical_model_id(&selected.model.id);
        let index = editor
            .catalog
            .iter()
            .position(|model| model.id == wanted)
            .with_context(|| format!("model '{}' is no longer configured", selected.model.id))?;
        editor.selected = editor
            .filtered_indices()
            .iter()
            .position(|candidate| *candidate == index)
            .unwrap_or(0);
        editor.toggle_selected();
        let mut profile = self.config.profiles[&selected.profile_id].clone();
        apply_route_editor(&mut profile, &editor);
        Ok(profile)
    }

    fn set_selected_as_default(&mut self) {
        if self.view_mode == ViewMode::Provider {
            if let Some(editor) = self.ensure_provider_editor() {
                editor.set_selected_default();
            }
            let _ = self.commit_provider_editor();
            self.status_error = false;
            let def = self
                .provider_editor
                .as_ref()
                .map(|e| e.default_model.clone())
                .unwrap_or_default();
            self.status = format!("Default model set to {def}");
            return;
        }
        let Some(profile_id) = self.selected_profile_id() else {
            return;
        };
        let Some(model) = self.selected_model() else {
            return;
        };
        let model_id = model.id.clone();
        let old_default = self
            .config
            .profiles
            .get(&profile_id)
            .map(|p| p.default_model.clone());
        let update = config::update(&self.paths.config, |latest| {
            let profile = latest
                .profiles
                .get_mut(&profile_id)
                .context("profile was removed in another CCSW instance")?;
            if let Some(old) = old_default
                && old != model_id
                && !profile.enabled_models.contains(&old)
            {
                profile.enabled_models.push(old);
            }
            profile.enabled_models.retain(|id| id != &model_id);
            profile
                .disabled_models
                .retain(|id| canonical_model_id(id) != canonical_model_id(&model_id));
            profile.default_model = model_id.clone();
            Ok(())
        });
        match update {
            Ok(config) => {
                self.config = config;
                self.select_model_id(&model_id);
                self.status_error = false;
                self.status = format!("Default model set to {model_id}");
            }
            Err(error) => self.set_error(format!("Could not set default model: {error:#}")),
        }
    }

    fn toggle_selected_model_1m(&mut self) {
        if self.view_mode == ViewMode::Provider {
            if let Some(editor) = self.ensure_provider_editor() {
                editor.toggle_selected_1m();
            }
            let _ = self.commit_provider_editor();
            self.status_error = false;
            self.status = "Toggled 1M context on selected model".into();
            return;
        }
        let Some(profile_id) = self.selected_profile_id() else {
            return;
        };
        let Some(model) = self.selected_model() else {
            return;
        };
        let old_id = model.id.clone();
        let base = canonical_model_id(&old_id);
        let new_id = if has_1m_suffix(&old_id) {
            base.clone()
        } else {
            format!("{base}[1m]")
        };
        let update = config::update(&self.paths.config, |latest| {
            let profile = latest
                .profiles
                .get_mut(&profile_id)
                .context("profile was removed in another CCSW instance")?;
            if profile.default_model == old_id {
                profile.default_model = new_id.clone();
            }
            for role in [
                &mut profile.aliases.opus,
                &mut profile.aliases.sonnet,
                &mut profile.aliases.haiku,
                &mut profile.aliases.fable,
                &mut profile.subagent_model,
            ]
            .into_iter()
            .flatten()
            {
                if *role == old_id {
                    *role = new_id.clone();
                }
            }
            for fb in &mut profile.fallback_models {
                if *fb == old_id {
                    *fb = new_id.clone();
                }
            }
            for em in &mut profile.enabled_models {
                if *em == old_id {
                    *em = new_id.clone();
                }
            }
            for disabled in &mut profile.disabled_models {
                if *disabled == old_id {
                    *disabled = new_id.clone();
                }
            }
            for m in &mut profile.models {
                if m.id == old_id {
                    m.id = new_id.clone();
                    if let Some(label) = &mut m.label {
                        if new_id.ends_with("[1m]") && !label.contains("1M") {
                            *label = format!("{label} · 1M");
                        } else if !new_id.ends_with("[1m]") {
                            *label = label.replace(" · 1M", "").replace(" 1M", "");
                        }
                    }
                }
            }
            Ok(())
        });
        match update {
            Ok(config) => {
                self.config = config;
                self.select_model_id(&new_id);
                self.status_error = false;
                self.status = if has_1m_suffix(&new_id) {
                    format!("1M context enabled for {base}")
                } else {
                    format!("1M context disabled for {base}")
                };
            }
            Err(error) => self.set_error(format!("Could not toggle 1M context: {error:#}")),
        }
    }

    fn delete_selected_model(&mut self) {
        let Some(model) = self.selected_model() else {
            return;
        };
        let base_id = canonical_model_id(&model.id);
        let is_custom = self.selected_profile().is_some_and(|profile| {
            profile
                .models
                .iter()
                .any(|entry| canonical_model_id(&entry.id) == base_id)
        });
        if is_custom {
            self.modal = Some(Modal::DeleteModel);
        } else {
            self.set_error(
                "Gateway models cannot be deleted · press Space to enable or disable this model",
            );
        }
    }

    fn launch_selected(&mut self, terminal: &mut TuiTerminal, force_new: bool) -> Result<()> {
        let Some(profile_id) = self.selected_profile_id() else {
            self.set_error("Create or import a profile first");
            return Ok(());
        };
        let Some(model) = self.selected_model() else {
            self.set_error("Add a model to this profile first");
            return Ok(());
        };
        let profile = self.config.profiles[&profile_id].clone();
        let routed_profile = match proxy::routed_profile(&self.paths, &profile_id, &profile) {
            Ok(profile) => profile,
            Err(error) => {
                self.set_error(format!("Could not prepare route: {error:#}"));
                return Ok(());
            }
        };
        self.proxy_status = proxy::status(&self.paths).ok();
        let models = self.models();
        let mode = if force_new {
            SessionMode::New
        } else if let Some(session) = &self.current_session {
            SessionMode::Resume(session.session_id.clone())
        } else {
            SessionMode::New
        };
        self.persist_selection(&profile_id, &model.id)?;
        restore_terminal(terminal)?;
        let result = runner::launch(
            &self.paths,
            LaunchRequest {
                profile: &routed_profile,
                model_id: &model.id,
                models: &models,
                mode,
                forwarded_args: self.forwarded_args.clone(),
            },
        );
        *terminal = setup_terminal()?;
        match result {
            Ok(result) => {
                let code = result.status.code().unwrap_or(-1);
                self.current_session = Some(result.capture.clone());
                self.launch_mode = LaunchMode::Resume;
                self.select_model_id(&result.capture.model_id);
                self.persist_session(&profile_id, &result.capture)?;
                self.status_error = !result.status.success();
                self.status = if result.status.success() {
                    "Claude exited · Enter resumes this session · N starts a new one".into()
                } else {
                    format!("Claude exited with status {code}; session is still resumable")
                };
            }
            Err(error) => self.set_error(format!("Could not launch Claude: {error:#}")),
        }
        Ok(())
    }

    fn select_model_id(&mut self, model_id: &str) {
        if let Some(index) = self.models().iter().position(|model| model.id == model_id) {
            self.model_idx = index;
        }
    }

    fn persist_selection(&self, profile_id: &str, model_id: &str) -> Result<()> {
        state::update(&self.paths.state, |state| {
            state.projects.insert(
                self.cwd.clone(),
                ProjectState {
                    profile_id: profile_id.into(),
                    model_id: model_id.into(),
                },
            );
        })?;
        Ok(())
    }

    fn persist_session(&self, profile_id: &str, capture: &SessionCapture) -> Result<()> {
        state::update(&self.paths.state, |state| {
            state.sessions.insert(
                capture.session_id.clone(),
                SessionRecord {
                    session_id: capture.session_id.clone(),
                    cwd: self.cwd.clone(),
                    profile_id: profile_id.into(),
                    model_id: capture.model_id.clone(),
                    updated_at: state::now_epoch(),
                },
            );
        })?;
        Ok(())
    }

    fn set_error(&mut self, message: impl Into<String>) {
        self.status_error = true;
        self.status = message.into();
    }

    fn apply_enabled_to_claude(&self) -> Result<claude_config::ApplyResult> {
        let settings = claude_config::settings_path()?;
        let selected = self
            .selected_profile_id()
            .filter(|id| self.profile_has_active_models(id));
        let default_profile_id = selected.or_else(|| {
            self.config
                .profiles
                .iter()
                .find(|(id, _)| self.profile_has_active_models(id))
                .map(|(id, _)| id.clone())
        });
        if let Some(default_profile_id) = default_profile_id {
            claude_config::apply_all(
                &settings,
                &self.paths,
                &self.config,
                &self.cache,
                &default_profile_id,
            )
        } else {
            proxy::clear_aggregate_models(&self.paths)?;
            claude_config::clear(&settings)
        }
    }

    fn profile_has_active_models(&self, profile_id: &str) -> bool {
        let Some(profile) = self.config.profiles.get(profile_id) else {
            return false;
        };
        let discovered = self
            .cache
            .profiles
            .get(profile_id)
            .map(|cached| cached.models.as_slice())
            .unwrap_or_default();
        !discovery::active_models(profile, discovered).is_empty()
    }

    fn handle_modal(&mut self, key: KeyEvent) -> Result<()> {
        let Some(mut modal) = self.modal.take() else {
            return Ok(());
        };
        match &mut modal {
            Modal::Import(candidate) => match key.code {
                KeyCode::Char('i') | KeyCode::Enter => {
                    let id = unique_profile_id("imported", &self.config.profiles);
                    let profile = candidate.profile.clone();
                    self.config = config::update(&self.paths.config, |latest| {
                        latest.profiles.insert(id.clone(), profile);
                        Ok(())
                    })?;
                    self.profile_idx = 0;
                    self.model_idx = self.default_model_index();
                    self.status =
                        "Imported existing Claude settings; the source file was not changed".into();
                    self.status_error = false;
                    return Ok(());
                }
                KeyCode::Char('s') | KeyCode::Esc => return Ok(()),
                _ => {}
            },
            Modal::Help(help) => match key.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter => {
                    return Ok(());
                }
                KeyCode::Tab | KeyCode::Right => help.move_section(true),
                KeyCode::BackTab | KeyCode::Left => help.move_section(false),
                KeyCode::Down | KeyCode::PageDown => help.scroll(true),
                KeyCode::Up | KeyCode::PageUp => help.scroll(false),
                KeyCode::Char('1') => help.select(0),
                KeyCode::Char('2') => help.select(1),
                KeyCode::Char('3') => help.select(2),
                KeyCode::Char('4') => help.select(3),
                _ => {}
            },
            Modal::DeleteProfile => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    if let Some(id) = self.selected_profile_id() {
                        self.config = config::update(&self.paths.config, |latest| {
                            latest.profiles.remove(&id);
                            Ok(())
                        })?;
                        self.cache = discovery::update_cache(&self.paths.cache, |cache| {
                            cache.profiles.remove(&id);
                        })?;
                        self.profile_idx = self
                            .profile_idx
                            .min(self.config.profiles.len().saturating_sub(1));
                        self.model_idx = 0;
                        self.status = format!("Deleted profile {id}");
                    }
                    return Ok(());
                }
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                _ => {}
            },
            Modal::DeleteModel => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    if let (Some(profile_id), Some(model)) =
                        (self.selected_profile_id(), self.selected_model())
                    {
                        let model_base = canonical_model_id(&model.id);
                        let is_manual = self.config.profiles[&profile_id]
                            .models
                            .iter()
                            .any(|entry| canonical_model_id(&entry.id) == model_base);
                        if !is_manual {
                            self.set_error(
                                "Gateway models cannot be deleted · use Space to disable them",
                            );
                        } else {
                            let model_id = model.id.clone();
                            let deleting_default = canonical_model_id(
                                &self.config.profiles[&profile_id].default_model,
                            ) == model_base;
                            let replacement = self.provider_editor.as_ref().and_then(|editor| {
                                editor
                                    .catalog
                                    .iter()
                                    .find(|entry| {
                                        entry.id != model_base && editor.is_enabled(&entry.id)
                                    })
                                    .or_else(|| {
                                        editor.catalog.iter().find(|entry| entry.id != model_base)
                                    })
                                    .map(|entry| editor.effective_id(&entry.id))
                            });
                            if deleting_default && replacement.is_none() {
                                self.set_error(
                                    "The only model cannot be deleted · add another model first",
                                );
                                return Ok(());
                            }
                            self.config = config::update(&self.paths.config, |latest| {
                                let profile = latest
                                    .profiles
                                    .get_mut(&profile_id)
                                    .context("profile was removed in another CCSW instance")?;
                                profile
                                    .models
                                    .retain(|entry| canonical_model_id(&entry.id) != model_base);
                                profile
                                    .enabled_models
                                    .retain(|id| canonical_model_id(id) != model_base);
                                profile
                                    .disabled_models
                                    .retain(|id| canonical_model_id(id) != model_base);
                                for alias in [
                                    &mut profile.aliases.opus,
                                    &mut profile.aliases.sonnet,
                                    &mut profile.aliases.haiku,
                                    &mut profile.aliases.fable,
                                    &mut profile.subagent_model,
                                ] {
                                    if alias
                                        .as_ref()
                                        .is_some_and(|id| canonical_model_id(id) == model_base)
                                    {
                                        *alias = None;
                                    }
                                }
                                profile
                                    .fallback_models
                                    .retain(|id| canonical_model_id(id) != model_base);
                                if let Some(replacement) = &replacement
                                    && deleting_default
                                {
                                    let replacement_base = canonical_model_id(replacement);
                                    profile
                                        .disabled_models
                                        .retain(|id| canonical_model_id(id) != replacement_base);
                                    profile.default_model = replacement.clone();
                                }
                                Ok(())
                            })?;
                            self.model_idx =
                                self.model_idx.min(self.models().len().saturating_sub(1));
                            self.status_error = false;
                            self.status = format!("Deleted custom model {model_id}");
                            self.init_provider_editor();
                        }
                    }
                    return Ok(());
                }
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                _ => {}
            },
            Modal::Proxy(manager) => {
                let control = match key.code {
                    KeyCode::Esc | KeyCode::Char('P') | KeyCode::Char('q') => return Ok(()),
                    KeyCode::Tab | KeyCode::Right | KeyCode::Down => {
                        manager.move_selection(true);
                        None
                    }
                    KeyCode::BackTab | KeyCode::Left | KeyCode::Up => {
                        manager.move_selection(false);
                        None
                    }
                    KeyCode::Enter | KeyCode::Char(' ') => Some(manager.selected_control()),
                    KeyCode::Char('s') => Some(ProxyControl::Start),
                    KeyCode::Char('x') => Some(ProxyControl::Stop),
                    KeyCode::Char('r') => Some(ProxyControl::Refresh),
                    KeyCode::Char('i') => Some(ProxyControl::EnableAtLogin),
                    KeyCode::Char('u') => Some(ProxyControl::DisableAtLogin),
                    _ => None,
                };
                if let Some(control) = control {
                    manager.selected = proxy_control_index(control);
                    if manager.activate(&self.paths, control) {
                        self.proxy_status = manager.runtime.clone();
                        return Ok(());
                    }
                    self.proxy_status = manager.runtime.clone();
                }
            }
            Modal::Profile(form) => {
                let outcome = handle_form_key(&mut form.fields, &mut form.selected, key);
                if outcome == FormOutcome::Close {
                    return Ok(());
                }
                if outcome == FormOutcome::Submit
                    || (key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('s'))
                {
                    match form.to_profile() {
                        Ok((id, profile)) => {
                            let original = form.original_id.clone();
                            let original_profile = form.original_profile.clone();
                            let update = config::update(&self.paths.config, |latest| {
                                if let (Some(original_id), Some(original_profile)) =
                                    (&original, &original_profile)
                                    && latest.profiles.get(original_id) != Some(original_profile)
                                {
                                    anyhow::bail!(
                                        "profile '{original_id}' changed in another CCSW instance"
                                    );
                                }
                                if latest.profiles.contains_key(&id)
                                    && original.as_deref() != Some(id.as_str())
                                {
                                    anyhow::bail!("profile id '{id}' already exists");
                                }
                                if let Some(original) = &original
                                    && original != &id
                                {
                                    latest.profiles.remove(original);
                                }
                                latest.profiles.insert(id.clone(), profile);
                                Ok(())
                            });
                            self.config = match update {
                                Ok(config) => config,
                                Err(error) => {
                                    self.set_error(format!("Cannot save profile: {error:#}"));
                                    self.modal = Some(modal);
                                    return Ok(());
                                }
                            };
                            if let Some(original) = &form.original_id
                                && original != &id
                            {
                                self.cache = discovery::update_cache(&self.paths.cache, |cache| {
                                    if let Some(cached) = cache.profiles.remove(original) {
                                        cache.profiles.insert(id.clone(), cached);
                                    }
                                })?;
                            }
                            self.profile_idx = self
                                .profile_ids()
                                .iter()
                                .position(|candidate| candidate == &id)
                                .unwrap_or(0);
                            self.model_idx = self.default_model_index();
                            self.status_error = false;
                            self.status = format!("Saved profile {id}");
                            self.init_provider_editor();
                            return Ok(());
                        }
                        Err(error) => self.set_error(format!("Cannot save profile: {error:#}")),
                    }
                }
            }
            Modal::Model(form) => {
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && (key.code == KeyCode::Char('f') || key.code == KeyCode::Char('r'))
                {
                    self.modal = Some(modal);
                    self.fetch_api_models_for_form();
                    return Ok(());
                }

                if key.code == KeyCode::Tab {
                    form.focus_api_search = !form.focus_api_search;
                    return Ok(());
                }

                if form.focus_api_search {
                    match key.code {
                        KeyCode::Esc => {
                            if !form.api_query.is_empty() {
                                form.api_query.clear();
                                form.api_query_cursor = 0;
                                form.api_scroll = 0;
                                form.api_selected = 0;
                            } else {
                                form.focus_api_search = false;
                            }
                            return Ok(());
                        }
                        KeyCode::Down => {
                            form.move_api_selection(true, 12);
                            return Ok(());
                        }
                        KeyCode::Up => {
                            form.move_api_selection(false, 12);
                            return Ok(());
                        }
                        KeyCode::PageDown => {
                            form.scroll_api_list(true, 8, 12);
                            return Ok(());
                        }
                        KeyCode::PageUp => {
                            form.scroll_api_list(false, 8, 12);
                            return Ok(());
                        }
                        KeyCode::Enter => {
                            form.pick_api_model(form.api_selected);
                            form.focus_api_search = false;
                            return Ok(());
                        }
                        KeyCode::Backspace => {
                            let chars: Vec<char> = form.api_query.chars().collect();
                            if form.api_query_cursor > 0 && !chars.is_empty() {
                                let mut new_s = String::new();
                                for (i, &ch) in chars.iter().enumerate() {
                                    if i != form.api_query_cursor - 1 {
                                        new_s.push(ch);
                                    }
                                }
                                form.api_query = new_s;
                                form.api_query_cursor = form.api_query_cursor.saturating_sub(1);
                                form.api_scroll = 0;
                                form.api_selected = 0;
                            }
                            return Ok(());
                        }
                        KeyCode::Left => {
                            form.api_query_cursor = form.api_query_cursor.saturating_sub(1);
                            return Ok(());
                        }
                        KeyCode::Right => {
                            let len = form.api_query.chars().count();
                            if form.api_query_cursor < len {
                                form.api_query_cursor += 1;
                            }
                            return Ok(());
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            let chars: Vec<char> = form.api_query.chars().collect();
                            let mut new_s = String::new();
                            let mut inserted = false;
                            for (i, &ch) in chars.iter().enumerate() {
                                if i == form.api_query_cursor {
                                    new_s.push(c);
                                    inserted = true;
                                }
                                new_s.push(ch);
                            }
                            if !inserted {
                                new_s.push(c);
                            }
                            form.api_query = new_s;
                            form.api_query_cursor += 1;
                            form.api_scroll = 0;
                            form.api_selected = 0;
                            return Ok(());
                        }
                        _ => return Ok(()),
                    }
                }

                let outcome = handle_form_key(&mut form.fields, &mut form.selected, key);
                if outcome == FormOutcome::Close {
                    return Ok(());
                }
                if outcome == FormOutcome::Submit
                    || (key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('s'))
                {
                    let base_id = canonical_model_id(form.fields[0].value.trim());
                    if base_id.trim().is_empty() {
                        self.set_error("Model id cannot be empty");
                    } else if let Some(profile_id) = self.selected_profile_id() {
                        let model = form.to_model();
                        let saved_model = model.clone();
                        let enable_now = form.enable_now();
                        self.config = config::update(&self.paths.config, |latest| {
                            let profile = latest
                                .profiles
                                .get_mut(&profile_id)
                                .context("profile was removed in another CCSW instance")?;
                            let saved_base = canonical_model_id(&saved_model.id);
                            profile
                                .models
                                .retain(|entry| canonical_model_id(&entry.id) != saved_base);
                            profile.models.push(saved_model.clone());
                            profile
                                .enabled_models
                                .retain(|id| canonical_model_id(id) != saved_base);
                            profile
                                .disabled_models
                                .retain(|id| canonical_model_id(id) != saved_base);
                            if enable_now {
                                if !profile.required_model_ids().contains(&saved_model.id) {
                                    profile.enabled_models.push(saved_model.id.clone());
                                }
                            } else {
                                profile.disabled_models.push(saved_model.id.clone());
                            }
                            Ok(())
                        })?;
                        self.select_model_id(&model.id);
                        self.status_error = false;
                        self.status = format!(
                            "Saved model {} · {}",
                            model.id,
                            if enable_now { "enabled" } else { "disabled" }
                        );
                        self.init_provider_editor();
                        if let Some(editor) = &mut self.provider_editor
                            && let Some(index) = editor
                                .catalog
                                .iter()
                                .position(|entry| canonical_model_id(&entry.id) == base_id)
                        {
                            editor.selected = index;
                        }
                        return Ok(());
                    }
                }
            }
        }
        self.modal = Some(modal);
        Ok(())
    }

    fn draw(&mut self, frame: &mut ratatui::Frame) {
        let area = frame.area();
        let rows = app_rows(area);
        self.draw_route(frame, rows[0]);
        let ui = ui_areas(area, self.focus, self.view_mode);
        if let Some(profiles) = ui.profiles {
            self.draw_profiles(frame, profiles);
        }
        if let Some(models) = ui.models {
            self.draw_models(frame, models);
        }
        if let Some(details) = ui.details {
            self.draw_details(frame, details, self.focus == Focus::Details);
        }
        self.draw_status(frame, ui.footer, area.width < 100);
        if let Some(modal) = &self.modal {
            self.draw_modal(frame, modal);
        }
    }

    fn draw_route(&self, frame: &mut ratatui::Frame, area: Rect) {
        let profile = self
            .selected_profile()
            .map(|profile| profile.name.as_str())
            .unwrap_or("no profile");
        let model = self
            .selected_model()
            .map(|model| model.label().to_owned())
            .unwrap_or_else(|| "no model".into());
        let session = self
            .current_session
            .as_ref()
            .map(|capture| short_id(&capture.session_id))
            .unwrap_or("new");
        let line = match self.view_mode {
            ViewMode::Home => Line::from(vec![
                Span::styled(
                    " CCSW ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(ROUTE)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "  Routers / 厂商列表",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  ·  {} 个厂商", self.config.profiles.len()),
                    Style::default().fg(MUTED),
                ),
            ]),
            ViewMode::Provider => Line::from(vec![
                Span::styled(
                    " ‹ 返回 (Esc) ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(ROUTE)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {profile}  "),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled("→", Style::default().fg(ROUTE)),
                Span::styled(
                    format!("  {model}  "),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled("→", Style::default().fg(ROUTE)),
                Span::styled(format!("  {session}"), Style::default().fg(MUTED)),
            ]),
            ViewMode::AllEnabled => Line::from(vec![
                Span::styled(
                    " ‹ 返回 (Esc) ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(ROUTE)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "  All Enabled / 全部启用",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  ·  {} models", self.all_enabled_model_count()),
                    Style::default().fg(CONNECTED),
                ),
            ]),
        };
        frame.render_widget(
            Paragraph::new(line)
                .wrap(Wrap { trim: true })
                .block(Block::default().borders(Borders::BOTTOM)),
            area,
        );
    }

    fn draw_profiles(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let ids = self.profile_ids();
        let is_home = self.view_mode == ViewMode::Home;
        let content_width = area.width.saturating_sub(2);
        let mut item_heights = Vec::with_capacity(ids.len().saturating_add(1));
        let mut items = Vec::with_capacity(ids.len().saturating_add(1));
        if is_home {
            let lines = all_enabled_lines(
                self.config
                    .profiles
                    .values()
                    .filter(|profile| profile.enabled)
                    .count(),
                self.all_enabled_model_count(),
                content_width,
            );
            item_heights.push(lines.len());
            items.push(ListItem::new(lines));
        }
        items.extend(ids.iter().map(|id| {
            let profile = &self.config.profiles[id];
            if is_home {
                let discovered = self
                    .cache
                    .profiles
                    .get(id)
                    .map(|cached| cached.models.as_slice())
                    .unwrap_or_default();
                let enabled_count = discovery::active_models(profile, discovered).len();
                let lines = home_profile_lines(id, profile, enabled_count, content_width);
                item_heights.push(lines.len());
                ListItem::new(lines)
            } else {
                item_heights.push(1);
                ListItem::new(Line::from(vec![
                    Span::styled("● ", Style::default().fg(CONNECTED)),
                    Span::raw(profile.name.clone()),
                    Span::styled(format!("  {id}"), Style::default().fg(MUTED)),
                ]))
            }
        }));
        let title = if is_home {
            " Providers / 路由厂商 "
        } else {
            " Routes "
        };
        let selected = if is_home {
            self.home_selected_index()
        } else {
            self.profile_idx
        };
        let mut state = ListState::default()
            .with_offset(self.profile_offset)
            .with_selected((!items.is_empty()).then_some(selected));
        let item_count = items.len();
        frame.render_stateful_widget(
            List::new(items)
                .block(panel(title, self.focus == Focus::Profiles))
                .highlight_style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(ROUTE)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol(" "),
            area,
            &mut state,
        );
        self.profile_offset = state.offset();
        let visible_items = if is_home {
            visible_variable_items(
                &item_heights,
                self.profile_offset,
                usize::from(area.height.saturating_sub(2)),
            )
        } else {
            usize::from(area.height.saturating_sub(2))
        };
        draw_scrollbar(frame, area, item_count, selected, visible_items);
    }

    fn draw_models(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        if self.view_mode == ViewMode::AllEnabled {
            let models = self.all_managed_models();
            let enabled_count = models.iter().filter(|entry| entry.enabled).count();
            let items = models
                .iter()
                .map(|entry| {
                    ListItem::new(vec![
                        Line::from(vec![
                            Span::styled(
                                if entry.enabled { "● " } else { "○ " },
                                Style::default().fg(if entry.enabled { CONNECTED } else { MUTED }),
                            ),
                            Span::styled(
                                entry.profile_name.clone(),
                                Style::default().fg(ROUTE).add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!("  {}", entry.profile_id),
                                Style::default().fg(MUTED),
                            ),
                            Span::raw("  ·  "),
                            Span::styled(
                                entry.model.label().to_owned(),
                                Style::default().add_modifier(Modifier::BOLD),
                            ),
                        ]),
                        Line::from(vec![
                            Span::styled("  ", Style::default()),
                            Span::styled(entry.model.id.clone(), Style::default().fg(MUTED)),
                        ]),
                    ])
                })
                .collect::<Vec<_>>();
            let mut state = ListState::default()
                .with_offset(self.model_offset)
                .with_selected((!items.is_empty()).then_some(self.model_idx));
            let title = format!(
                " All enabled models / 全部启用模型 · {enabled_count}/{} enabled ",
                models.len()
            );
            frame.render_stateful_widget(
                List::new(items)
                    .block(panel(&title, true))
                    .highlight_style(
                        Style::default()
                            .fg(Color::Black)
                            .bg(ROUTE)
                            .add_modifier(Modifier::BOLD),
                    )
                    .highlight_symbol(" "),
                area,
                &mut state,
            );
            self.model_offset = state.offset();
            draw_scrollbar(
                frame,
                area,
                models.len(),
                self.model_idx,
                usize::from(area.height.saturating_sub(2) / 2),
            );
            if models.is_empty() {
                frame.render_widget(
                    Paragraph::new(
                        "No managed models.\nAdd models in a provider, or enable a provider from Home.",
                    )
                    .style(Style::default().fg(MUTED))
                    .alignment(Alignment::Center)
                    .block(panel(&title, true)),
                    area,
                );
            }
            return;
        }
        if self.view_mode == ViewMode::Provider {
            let fallback;
            let editor = match &self.provider_editor {
                Some(e) => e,
                None => {
                    fallback = self.create_route_editor();
                    match &fallback {
                        Some(e) => e,
                        None => {
                            frame.render_widget(
                                Paragraph::new("No provider selected")
                                    .style(Style::default().fg(MUTED))
                                    .alignment(Alignment::Center)
                                    .block(panel(" Models ", false)),
                                area,
                            );
                            return;
                        }
                    }
                }
            };

            let search_area = Rect::new(area.x, area.y, area.width, 3);
            let list_area = Rect::new(
                area.x,
                area.y + 3,
                area.width,
                area.height.saturating_sub(3),
            );

            let search_title = if editor.search_active {
                " 🔍 搜索模型 · Esc 退出 "
            } else {
                " 🔍 搜索模型 · / 开始输入 "
            };
            let search_block = panel(search_title, editor.search_active);
            let search_inner = panel_inner(search_area);
            frame.render_widget(search_block, search_area);
            if search_inner.width > 0 && search_inner.height > 0 {
                let filtered_len = editor.filtered_indices().len();
                let total_len = editor.catalog.len();
                let enabled_len = editor
                    .catalog
                    .iter()
                    .filter(|m| editor.is_enabled(&m.id))
                    .count();
                let stats_text =
                    format!("({filtered_len}/{total_len} 可用 · {enabled_len} 已启用)");
                let query_text = if editor.query.is_empty() {
                    if editor.search_active {
                        "输入模型名称或 ID 搜索...".to_owned()
                    } else {
                        "/ 输入关键字筛选模型...".to_owned()
                    }
                } else {
                    editor.query.clone()
                };
                let line = Line::from(vec![
                    Span::styled(" ", Style::default()),
                    Span::styled(
                        query_text,
                        if editor.query.is_empty() {
                            Style::default().fg(MUTED)
                        } else {
                            Style::default()
                                .fg(Color::White)
                                .add_modifier(Modifier::BOLD)
                        },
                    ),
                    if editor.search_active {
                        Span::styled("▌", Style::default().fg(CONNECTED))
                    } else {
                        Span::raw("")
                    },
                    if search_inner.width >= 55 {
                        Span::styled(format!("  {stats_text}"), Style::default().fg(MUTED))
                    } else {
                        Span::raw("")
                    },
                ]);
                let add_btn = catalog_add_button_rect(search_area);
                let text_width = if let Some(btn) = add_btn {
                    search_inner
                        .width
                        .saturating_sub(btn.width.saturating_add(1))
                } else {
                    search_inner.width
                };
                let text_area = Rect::new(
                    search_inner.x,
                    search_inner.y,
                    text_width,
                    search_inner.height,
                );
                frame.render_widget(Paragraph::new(line), text_area);
                if let Some(btn_rect) = add_btn {
                    let label = if btn_rect.width >= 14 {
                        "[+ Add model (a)]"
                    } else {
                        "[+ a]"
                    };
                    frame.render_widget(
                        Paragraph::new(label).alignment(Alignment::Center).style(
                            Style::default()
                                .fg(Color::Black)
                                .bg(CONNECTED)
                                .add_modifier(Modifier::BOLD),
                        ),
                        btn_rect,
                    );
                }
            }

            let filtered = editor.filtered_indices();
            let list_title = " 模型目录 · ◆ 默认  ● 启用  ○ 禁用 ";
            let block = panel(
                list_title,
                self.focus == Focus::Models && !editor.search_active,
            );
            let inner = panel_inner(list_area);
            frame.render_widget(block, list_area);
            if inner.width == 0 || inner.height == 0 {
                return;
            }

            let manual_map = self
                .selected_profile()
                .map(|p| {
                    p.models
                        .iter()
                        .map(|m| (m.id.as_str(), ()))
                        .collect::<BTreeMap<_, _>>()
                })
                .unwrap_or_default();

            let items: Vec<ListItem> = filtered
                .iter()
                .map(|&idx| {
                    let model = &editor.catalog[idx];
                    let is_en = editor.is_enabled(&model.id);
                    let is_1m = editor.one_m.contains(&model.id);
                    let is_def = model.id == editor.default_model;
                    let is_man = manual_map.contains_key(model.id.as_str());
                    let is_req = editor.is_required(&model.id) && !is_def;

                    let marker = if is_def && is_en {
                        "◆"
                    } else if is_def && !is_en {
                        "◇"
                    } else if is_req {
                        "◈"
                    } else if is_en {
                        "●"
                    } else {
                        "○"
                    };
                    let marker_color = if is_def && is_en {
                        WARNING
                    } else if is_def && !is_en {
                        MUTED
                    } else if is_req || is_en {
                        CONNECTED
                    } else {
                        MUTED
                    };

                    let mut spans = vec![
                        Span::styled(
                            format!("{marker} "),
                            Style::default()
                                .fg(marker_color)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(model.label(), Style::default().fg(Color::White)),
                        Span::styled(format!("  {}", model.id), Style::default().fg(MUTED)),
                        Span::styled(
                            if is_1m { " 1M ●" } else { " 1M ○" },
                            Style::default().fg(if is_1m { CONNECTED } else { MUTED }),
                        ),
                    ];
                    if is_def {
                        spans.push(Span::styled(
                            " [默认]",
                            Style::default().fg(WARNING).add_modifier(Modifier::BOLD),
                        ));
                    }
                    if is_man {
                        spans.push(Span::styled(" [自定义]", Style::default().fg(ROUTE)));
                    }
                    ListItem::new(Line::from(spans))
                })
                .collect();

            let offset = route_editor_offset(editor, inner.height);
            let mut state = ListState::default()
                .with_offset(offset)
                .with_selected((!items.is_empty()).then_some(editor.selected));
            frame.render_stateful_widget(
                List::new(items)
                    .highlight_style(
                        Style::default()
                            .fg(Color::Black)
                            .bg(ROUTE)
                            .add_modifier(Modifier::BOLD),
                    )
                    .highlight_symbol(" "),
                inner,
                &mut state,
            );
            draw_scrollbar(
                frame,
                list_area,
                filtered.len(),
                editor.selected,
                usize::from(inner.height),
            );
            return;
        }

        let models = self.models();
        let manual: BTreeMap<_, _> = self
            .selected_profile()
            .map(|profile| {
                profile
                    .models
                    .iter()
                    .map(|model| (model.id.as_str(), ()))
                    .collect()
            })
            .unwrap_or_default();
        let items = models
            .iter()
            .map(|model| {
                let source = if manual.contains_key(model.id.as_str()) {
                    "manual"
                } else {
                    "gateway"
                };
                ListItem::new(vec![
                    Line::from(Span::raw(model.label())),
                    Line::from(vec![
                        Span::styled(&model.id, Style::default().fg(MUTED)),
                        Span::styled(format!("  {source}"), Style::default().fg(ROUTE)),
                    ]),
                ])
            })
            .collect::<Vec<_>>();
        let title = " Enabled models ";
        let mut state = ListState::default()
            .with_offset(self.model_offset)
            .with_selected((!items.is_empty()).then_some(self.model_idx));
        frame.render_stateful_widget(
            List::new(items)
                .block(panel(title, self.focus == Focus::Models))
                .highlight_style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(ROUTE)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol(" "),
            area,
            &mut state,
        );
        self.model_offset = state.offset();
        draw_scrollbar(
            frame,
            area,
            models.len(),
            self.model_idx,
            usize::from(area.height.saturating_sub(2) / 2),
        );
    }

    fn draw_showcase(
        &self,
        frame: &mut ratatui::Frame,
        area: Rect,
        editor: &RouteEditor,
        profile: &Profile,
    ) {
        frame.render_widget(panel(" ◆ Selected model / 当前模型 ", true), area);
        let inner = panel_inner(area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let filtered = editor.filtered_indices();
        let Some(&idx) = filtered.get(editor.selected) else {
            frame.render_widget(
                Paragraph::new("无匹配模型 (No matching model)")
                    .style(Style::default().fg(MUTED))
                    .alignment(Alignment::Center),
                inner,
            );
            return;
        };
        let model = &editor.catalog[idx];
        let effective_id = editor.effective_id(&model.id);
        let is_default = model.id == editor.default_model;
        let is_enabled = editor.is_enabled(&model.id);
        let is_1m = editor.one_m.contains(&model.id);
        let is_manual = profile
            .models
            .iter()
            .any(|m| canonical_model_id(&m.id) == model.id);
        let alias = profile
            .aliases
            .iter()
            .find(|(_, id)| canonical_model_id(id) == model.id)
            .map(|(role, _)| role);

        let lines = vec![
            Line::from(vec![
                Span::styled(" 模型: ", Style::default().fg(MUTED)),
                Span::styled(
                    model.label(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                if is_default {
                    Span::styled(
                        "  ★ 默认模型",
                        Style::default().fg(WARNING).add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::raw("")
                },
            ]),
            Line::from(vec![
                Span::styled(" 标识: ", Style::default().fg(MUTED)),
                Span::styled(
                    &effective_id,
                    Style::default().fg(ROUTE).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled(" 状态: ", Style::default().fg(MUTED)),
                Span::styled(
                    if is_enabled {
                        "● 已启用"
                    } else {
                        "○ 已禁用"
                    },
                    Style::default()
                        .fg(if is_enabled { CONNECTED } else { MUTED })
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled(" 上下文: ", Style::default().fg(MUTED)),
                Span::styled(
                    if is_1m { "1M 扩展" } else { "标准 200k" },
                    Style::default().fg(if is_1m { CONNECTED } else { MUTED }),
                ),
            ]),
            Line::from(vec![
                Span::styled(" 来源: ", Style::default().fg(MUTED)),
                Span::raw(if is_manual { "自定义" } else { "网关" }),
                if let Some(alias) = alias {
                    Span::styled(format!("   角色: {alias}"), Style::default().fg(ROUTE))
                } else {
                    Span::raw("")
                },
            ]),
        ];
        let controls = showcase_controls(area);
        let controls_top = controls
            .iter()
            .map(|(_, rect)| rect.y)
            .min()
            .unwrap_or(inner.y.saturating_add(inner.height));
        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            Rect::new(
                inner.x,
                inner.y,
                inner.width,
                controls_top.saturating_sub(inner.y),
            ),
        );
        for (control, rect) in controls {
            let (label, style) = match control {
                ShowcaseControl::Toggle => (
                    if is_enabled {
                        "[Space 禁用]"
                    } else {
                        "[Space 启用]"
                    },
                    Style::default()
                        .fg(Color::Black)
                        .bg(ROUTE)
                        .add_modifier(Modifier::BOLD),
                ),
                ShowcaseControl::Default => (
                    "[d 设为默认]",
                    Style::default().fg(WARNING).add_modifier(Modifier::BOLD),
                ),
                ShowcaseControl::OneM => (
                    if is_1m {
                        "[1 关闭 1M]"
                    } else {
                        "[1 开启 1M]"
                    },
                    Style::default().fg(ROUTE).add_modifier(Modifier::BOLD),
                ),
                ShowcaseControl::Delete => (
                    if is_manual {
                        "[x 删除]"
                    } else {
                        "[网关模型不可删除]"
                    },
                    Style::default().fg(if is_manual { ERROR } else { MUTED }),
                ),
            };
            frame.render_widget(
                Paragraph::new(label)
                    .alignment(Alignment::Center)
                    .style(style),
                rect,
            );
        }
    }

    fn draw_provider_details(
        &self,
        frame: &mut ratatui::Frame,
        area: Rect,
        profile: &Profile,
        active: bool,
    ) {
        let title = if active {
            " 厂商连接 · r 刷新 · E 编辑 "
        } else {
            " 厂商连接 (Provider) "
        };
        frame.render_widget(panel(title, active), area);
        let inner = panel_inner(area);
        let button_rows = 1;
        let content = Rect::new(
            inner.x,
            inner.y,
            inner.width,
            inner.height.saturating_sub(button_rows),
        );
        let api_format = if profile.api_format.is_openai() {
            let proxy = self
                .proxy_status
                .as_ref()
                .map(|status| {
                    if status.running {
                        format!("proxy ● {}", status.listen)
                    } else {
                        "proxy ○ stopped".into()
                    }
                })
                .unwrap_or_else(|| "proxy ?".into());
            format!("{} · {proxy}", profile.api_format.label())
        } else {
            profile.api_format.label().into()
        };
        let mut lines = vec![
            detail(
                "Provider",
                if profile.enabled {
                    "● enabled"
                } else {
                    "○ disabled"
                },
            ),
            detail("API format", &api_format),
            detail("Endpoint", &profile.base_url),
            detail("Credential", &profile.credential.masked()),
            detail("Default", &profile.default_model),
            detail(
                "Models",
                &format!(
                    "{} enabled / {} available",
                    self.models().len(),
                    self.catalog_models().len()
                ),
            ),
            detail(
                "Claude /model",
                &format!(
                    "{} models across {} providers",
                    self.all_enabled_model_count(),
                    self.config.profiles.len()
                ),
            ),
            Line::raw(""),
        ];
        for (role, model) in profile.aliases.iter() {
            lines.push(detail(&format!("{role} alias"), model));
        }
        if let Some(model) = &profile.subagent_model {
            lines.push(detail("subagent", model));
        }
        if !profile.fallback_models.is_empty() {
            lines.push(detail("fallback", &profile.fallback_models.join(" → ")));
        }
        lines.push(Line::raw(""));
        if let Some(session) = &self.current_session {
            lines.push(detail("Resume", &session.session_id));
            lines.push(detail("Last model", &session.model_id));
        } else {
            lines.push(Line::styled(
                "Next launch starts a new session",
                Style::default().fg(MUTED),
            ));
        }
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), content);
        draw_detail_controls(frame, area);
    }

    fn draw_details(&self, frame: &mut ratatui::Frame, area: Rect, active: bool) {
        let Some(profile) = self.selected_profile() else {
            let title = if active {
                " Route details · Esc back "
            } else {
                " Route details "
            };
            frame.render_widget(panel(title, active), area);
            let inner = panel_inner(area);
            frame.render_widget(
                Paragraph::new("Create a route to connect Claude Code to a gateway.")
                    .style(Style::default().fg(MUTED))
                    .wrap(Wrap { trim: true }),
                inner,
            );
            return;
        };

        if self.view_mode == ViewMode::Provider {
            let fallback;
            let editor = match &self.provider_editor {
                Some(e) => e,
                None => {
                    fallback = self.create_route_editor();
                    match &fallback {
                        Some(e) => e,
                        None => return,
                    }
                }
            };

            let (showcase_card, provider_card) = provider_detail_cards(area);
            self.draw_showcase(frame, showcase_card, editor, profile);
            self.draw_provider_details(frame, provider_card, profile, active);
            return;
        }

        self.draw_provider_details(frame, area, profile, active);
    }

    fn draw_status(&self, frame: &mut ratatui::Frame, area: Rect, compact: bool) {
        for (control, rect) in footer_controls(area, compact, self.view_mode) {
            let (label, style) = self.footer_control_style(control, compact, area.width < 55);
            frame.render_widget(
                Paragraph::new(label)
                    .alignment(Alignment::Center)
                    .style(style),
                rect,
            );
        }
        let color = if self.status_error { ERROR } else { MUTED };
        let footer = Line::from(vec![
            Span::styled(
                format!(" {} ", if self.status_error { "!" } else { "·" }),
                Style::default().fg(color),
            ),
            Span::styled(&self.status, Style::default().fg(color)),
        ]);
        if area.height > 1 {
            frame.render_widget(
                Paragraph::new(footer).wrap(Wrap { trim: true }),
                Rect {
                    y: area.y.saturating_add(1),
                    height: area.height.saturating_sub(1),
                    ..area
                },
            );
        }
    }

    fn footer_control_style(
        &self,
        control: FooterControl,
        compact: bool,
        tiny: bool,
    ) -> (String, Style) {
        let selected = match control {
            FooterControl::Models => self.focus == Focus::Models,
            FooterControl::Details => self.focus == Focus::Details,
            FooterControl::Resume => self.launch_mode == LaunchMode::Resume,
            FooterControl::New => self.launch_mode == LaunchMode::New,
            _ => false,
        };
        let dot = if selected { '●' } else { '○' };
        let label = match (control, compact, tiny) {
            (FooterControl::Back, _, true) => "‹".into(),
            (FooterControl::Models, _, true) => format!("{dot} M"),
            (FooterControl::Details, _, true) => format!("{dot} D"),
            (FooterControl::Launch, _, true) => {
                if self.view_mode == ViewMode::Home {
                    "Enter".into()
                } else {
                    "▶".into()
                }
            }
            (FooterControl::AddProfile, _, true) => "+".into(),
            (FooterControl::Sync, _, true) => "⇄".into(),
            (FooterControl::Proxy, _, true) => "Px".into(),
            (FooterControl::Help, _, true) => "?".into(),
            (FooterControl::Quit, _, true) => "×".into(),
            (FooterControl::Resume, _, true) => "R".into(),
            (FooterControl::New, _, true) => "N".into(),
            (FooterControl::Back, true, false) => "‹ Back".into(),
            (FooterControl::Back, false, false) => "‹ Back (Esc)".into(),
            (FooterControl::Models, _, false) => format!("{dot} Models"),
            (FooterControl::Details, _, false) => format!("{dot} Details"),
            (FooterControl::Launch, true, false) => {
                if self.view_mode == ViewMode::Home {
                    "Enter".into()
                } else {
                    "▶ Run".into()
                }
            }
            (FooterControl::Launch, false, false) => {
                if self.view_mode == ViewMode::Home {
                    "Enter 详情".into()
                } else {
                    "▶ Launch".into()
                }
            }
            (FooterControl::AddProfile, true, false) => "+ Route".into(),
            (FooterControl::AddProfile, false, false) => "+ New route".into(),
            (FooterControl::Resume, true, false) => format!("R{dot}"),
            (FooterControl::Resume, false, false) => format!("{dot} Resume"),
            (FooterControl::New, true, false) => format!("N{dot}"),
            (FooterControl::New, false, false) => format!("{dot} New"),
            (FooterControl::Sync, true, false) => "⇄ Sync".into(),
            (FooterControl::Sync, false, false) => "⇄ Sync all".into(),
            (FooterControl::Proxy, true, false) => format!(
                "{} Px",
                if self
                    .proxy_status
                    .as_ref()
                    .is_some_and(|status| status.running)
                {
                    '●'
                } else {
                    '○'
                }
            ),
            (FooterControl::Proxy, false, false) => format!(
                "{} Proxy",
                if self
                    .proxy_status
                    .as_ref()
                    .is_some_and(|status| status.running)
                {
                    '●'
                } else {
                    '○'
                }
            ),
            (FooterControl::Help, true, false) => "?".into(),
            (FooterControl::Help, false, false) => "Help".into(),
            (FooterControl::Quit, true, false) => "×".into(),
            (FooterControl::Quit, false, false) => "Quit".into(),
        };
        let style = if control == FooterControl::Back {
            Style::default()
                .fg(Color::Black)
                .bg(ROUTE)
                .add_modifier(Modifier::BOLD)
        } else if matches!(
            control,
            FooterControl::Launch | FooterControl::AddProfile | FooterControl::Sync
        ) {
            Style::default()
                .fg(Color::Black)
                .bg(
                    if matches!(control, FooterControl::Launch | FooterControl::AddProfile)
                        && self.view_mode == ViewMode::Home
                    {
                        CONNECTED
                    } else {
                        ROUTE
                    },
                )
                .add_modifier(Modifier::BOLD)
        } else if selected {
            Style::default()
                .fg(Color::Black)
                .bg(CONNECTED)
                .add_modifier(Modifier::BOLD)
        } else if control == FooterControl::Resume && self.current_session.is_none() {
            Style::default().fg(MUTED).add_modifier(Modifier::DIM)
        } else if control == FooterControl::Proxy {
            Style::default().fg(
                if self
                    .proxy_status
                    .as_ref()
                    .is_some_and(|status| status.running)
                {
                    CONNECTED
                } else {
                    WARNING
                },
            )
        } else {
            Style::default().fg(MUTED)
        };
        (label, style)
    }

    fn draw_modal(&self, frame: &mut ratatui::Frame, modal: &Modal) {
        let area = modal_area_for(modal, frame.area());
        frame.render_widget(Clear, area);
        match modal {
            Modal::Import(candidate) => {
                let mut lines = vec![
                    Line::styled(
                        "Existing Claude settings found",
                        Style::default().fg(ROUTE).add_modifier(Modifier::BOLD),
                    ),
                    Line::raw(""),
                ];
                lines.extend(candidate.summary().into_iter().map(Line::raw));
                lines.extend([
                    Line::raw(""),
                    Line::styled(
                        "The source file will not be changed.",
                        Style::default().fg(MUTED),
                    ),
                    Line::raw(""),
                    Line::styled("Enter/i import · s skip", Style::default().fg(WARNING)),
                ]);
                frame.render_widget(
                    Paragraph::new(lines).block(panel(" Import preview ", true)),
                    area,
                );
                draw_modal_buttons(frame, area, &["Import", "Skip"]);
            }
            Modal::Profile(form) => {
                draw_form(
                    frame,
                    area,
                    " Profile · click a field to edit ",
                    &form.fields,
                    form.selected,
                );
                draw_modal_buttons(frame, area, &["Save", "Cancel"]);
            }
            Modal::Model(form) => {
                draw_model_form(frame, area, form);
                draw_modal_buttons(frame, area, &["Fetch API (Ctrl+F)", "Save", "Cancel"]);
            }
            Modal::Proxy(manager) => draw_proxy_manager(frame, area, manager),
            Modal::DeleteProfile => {
                draw_confirmation(
                    frame,
                    area,
                    "Delete this profile? Cached model data will also be removed.",
                );
                draw_modal_buttons(frame, area, &["Delete", "Cancel"]);
            }
            Modal::DeleteModel => {
                draw_confirmation(
                    frame,
                    area,
                    "Delete this manual model? Gateway models cannot be deleted here.",
                );
                draw_modal_buttons(frame, area, &["Delete", "Cancel"]);
            }
            Modal::Help(help) => draw_help(frame, area, help),
        }
    }
}

fn help_commands(section: HelpSection) -> &'static [(&'static str, &'static str)] {
    match section {
        HelpSection::Home => &[
            ("↑↓ / j k", "选择 All Enabled 或厂商"),
            ("Enter / Click", "打开当前选中项"),
            ("Space", "启用/禁用厂商，并同步 Claude /model"),
            ("n / e / x", "新建 / 编辑 / 删除厂商"),
            ("r / t", "测试连接并刷新模型目录"),
            ("m / R / N", "切换模式 / 恢复会话 / 新建会话"),
            ("p / P", "同步全部模型 / 管理后台代理"),
            ("q", "退出 CCSW"),
        ],
        HelpSection::AllEnabled => &[
            ("↑↓ / j k", "跨厂商选择模型"),
            ("PgUp / PgDn", "快速翻页；Home / End 跳转首尾"),
            ("Space", "启用或禁用模型，并同步 Claude /model"),
            ("Enter", "打开该模型所属厂商"),
            ("p / P", "同步全部模型 / 管理后台代理"),
            ("Esc", "返回厂商首页"),
        ],
        HelpSection::Provider => &[
            ("↑↓ / j k", "浏览模型；窄窗口用 Tab 切换面板"),
            ("/ / Esc", "搜索模型 / 清空搜索或返回首页"),
            ("Space / d / 1", "切换启用 / 设为默认 / 切换 1M"),
            ("Enter / R / N", "运行选中模型 / 恢复 / 新建会话"),
            ("m", "切换 Resume / New 启动模式"),
            ("A / C", "启用筛选结果 / 清空非必要启用项"),
            ("a / x", "添加模型 / 删除自定义模型"),
            ("E / r / p / P", "编辑厂商 / 刷新 / 同步 / 代理"),
        ],
        HelpSection::Forms => &[
            ("↑↓ / Tab", "切换字段；Shift+Tab 返回上一项"),
            ("Enter", "确认选项或前往下一项；末项保存"),
            ("←→ / Home End", "移动文本光标"),
            ("Backspace / Del", "删除字符；Ctrl+U 清空当前字段"),
            ("Space / ←→", "切换开关或选项"),
            ("Ctrl+F / Ctrl+R", "自定义模型表单：从厂商 API 刷新模型"),
            ("Ctrl+S", "保存更改"),
            ("Esc", "取消；模型搜索中先清空搜索"),
        ],
    }
}

fn help_tabs(active: HelpSection) -> Line<'static> {
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

fn help_content(section: HelpSection, wide: bool) -> Vec<Line<'static>> {
    let heading = match section {
        HelpSection::Home => "Home · 厂商首页",
        HelpSection::AllEnabled => "All Enabled · 全部模型",
        HelpSection::Provider => "Provider · 厂商与模型",
        HelpSection::Forms => "Forms · 表单输入",
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
        Span::styled(" 启用  ", Style::default().fg(MUTED)),
        Span::styled("○", Style::default().fg(MUTED)),
        Span::styled(" 禁用  ", Style::default().fg(MUTED)),
        Span::styled("◆", Style::default().fg(ROUTE)),
        Span::styled(" 默认  ", Style::default().fg(MUTED)),
        Span::styled("◈", Style::default().fg(WARNING)),
        Span::styled(" 角色依赖", Style::default().fg(MUTED)),
    ]));
    lines
}

fn draw_help(frame: &mut ratatui::Frame, area: Rect, help: &HelpModal) {
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
            Paragraph::new("←→ / Tab 切换分区 · ↑↓ 滚动 · 1–4 直达")
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

impl ProxyManager {
    const CONTROLS: [ProxyControl; 6] = [
        ProxyControl::Start,
        ProxyControl::Stop,
        ProxyControl::Refresh,
        ProxyControl::EnableAtLogin,
        ProxyControl::DisableAtLogin,
        ProxyControl::Close,
    ];

    fn load(paths: &AppPaths) -> Self {
        let mut manager = Self {
            runtime: None,
            service: None,
            selected: 0,
            message: "Sync all starts the proxy; this page may be closed afterward.".into(),
            error: false,
        };
        manager.refresh_state(paths, None);
        manager
    }

    fn selected_control(&self) -> ProxyControl {
        Self::CONTROLS[self.selected.min(Self::CONTROLS.len() - 1)]
    }

    fn move_selection(&mut self, forward: bool) {
        if forward {
            self.selected = (self.selected + 1) % Self::CONTROLS.len();
        } else {
            self.selected = self
                .selected
                .checked_sub(1)
                .unwrap_or(Self::CONTROLS.len() - 1);
        }
    }

    fn activate(&mut self, paths: &AppPaths, control: ProxyControl) -> bool {
        if control == ProxyControl::Close {
            return true;
        }
        let result = match control {
            ProxyControl::Start if self.runtime.as_ref().is_some_and(|status| status.running) => {
                Ok("Proxy is already running".into())
            }
            ProxyControl::Start => proxy::start(paths, None)
                .map(|status| format!("Proxy started at {}", status.listen)),
            ProxyControl::Stop if !self.runtime.as_ref().is_some_and(|status| status.running) => {
                Ok("Proxy is already stopped".into())
            }
            ProxyControl::Stop => proxy::stop(paths).map(|()| "Proxy stopped".into()),
            ProxyControl::Refresh => Ok("Status refreshed".into()),
            ProxyControl::EnableAtLogin => proxy::install(paths)
                .map(|path| format!("Start at login enabled · {}", path.display())),
            ProxyControl::DisableAtLogin => proxy::uninstall().map(|path| match path {
                Some(path) => format!("Start at login disabled · removed {}", path.display()),
                None => "Start at login is already disabled".into(),
            }),
            ProxyControl::Close => unreachable!(),
        };
        match result {
            Ok(message) => self.refresh_state(paths, Some((message, false))),
            Err(error) => self.refresh_state(paths, Some((format!("{error:#}"), true))),
        }
        false
    }

    fn refresh_state(&mut self, paths: &AppPaths, message: Option<(String, bool)>) {
        let runtime = proxy::status(paths);
        let service = proxy::service_status();
        self.runtime = runtime.as_ref().ok().cloned();
        self.service = service.as_ref().ok().cloned();
        if let Some((message, error)) = message {
            self.message = message;
            self.error = error;
        } else if let Err(error) = runtime {
            self.message = format!("Could not read proxy status: {error:#}");
            self.error = true;
        } else if let Err(error) = service {
            self.message = format!("Could not read start-at-login status: {error:#}");
            self.error = true;
        }
    }
}

impl ProfileForm {
    fn new() -> Self {
        Self::from_values(
            None,
            vec![],
            vec![],
            vec![],
            "",
            "",
            "https://",
            "anthropic",
            "bearer",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
        )
    }

    fn edit(id: String, profile: &Profile) -> Self {
        let (kind, credential) = match &profile.credential {
            Credential::Bearer { value } => ("bearer", value.as_str()),
            Credential::XApiKey { value } => ("x-api-key", value.as_str()),
            Credential::ApiKey { value } => ("api-key", value.as_str()),
            Credential::None => ("none", ""),
        };
        let format = match profile.api_format {
            ApiFormat::Anthropic => "anthropic",
            ApiFormat::OpenaiChat => "openai-chat",
            ApiFormat::OpenaiResponses => "openai-responses",
        };
        let mut form = Self::from_values(
            Some(id.clone()),
            profile.models.clone(),
            profile.enabled_models.clone(),
            profile.disabled_models.clone(),
            &id,
            &profile.name,
            &profile.base_url,
            format,
            kind,
            credential,
            &profile.default_model,
            profile.aliases.opus.as_deref().unwrap_or(""),
            profile.aliases.sonnet.as_deref().unwrap_or(""),
            profile.aliases.haiku.as_deref().unwrap_or(""),
            profile.aliases.fable.as_deref().unwrap_or(""),
            profile.subagent_model.as_deref().unwrap_or(""),
            &profile.fallback_models.join(","),
        );
        form.provider_enabled = profile.enabled;
        form.original_profile = Some(profile.clone());
        form
    }

    #[allow(clippy::too_many_arguments)]
    fn from_values(
        original_id: Option<String>,
        models: Vec<ModelEntry>,
        enabled_models: Vec<String>,
        disabled_models: Vec<String>,
        id: &str,
        name: &str,
        base_url: &str,
        api_format: &str,
        kind: &str,
        credential: &str,
        default: &str,
        opus: &str,
        sonnet: &str,
        haiku: &str,
        fable: &str,
        subagent: &str,
        fallback: &str,
    ) -> Self {
        let fields = vec![
            field("ID", id),
            field("Name", name),
            choice_field(
                "API format",
                api_format,
                &["anthropic", "openai-chat", "openai-responses"],
            ),
            field("Base URL", base_url),
            choice_field(
                "Auth kind",
                kind,
                &["bearer", "x-api-key", "api-key", "none"],
            ),
            secret_field("Credential", credential),
            field("Default model", default),
            field("Opus", opus),
            field("Sonnet", sonnet),
            field("Haiku", haiku),
            field("Fable", fable),
            field("Subagent", subagent),
            field("Fallbacks (comma)", fallback),
        ];
        Self {
            original_id,
            original_profile: None,
            provider_enabled: true,
            models,
            enabled_models,
            disabled_models,
            fields,
            selected: 0,
        }
    }

    fn to_profile(&self) -> Result<(String, Profile)> {
        let value = |index: usize| self.fields[index].value.trim().to_owned();
        let id = value(0);
        config::validate_profile_id(&id)?;
        let api_format = match value(2).as_str() {
            "anthropic" => ApiFormat::Anthropic,
            "openai-chat" => ApiFormat::OpenaiChat,
            "openai-responses" => ApiFormat::OpenaiResponses,
            other => anyhow::bail!("unknown API format {other}"),
        };
        let credential = match value(4).as_str() {
            "bearer" => Credential::Bearer { value: value(5) },
            "x-api-key" => Credential::XApiKey { value: value(5) },
            "api-key" => Credential::ApiKey { value: value(5) },
            "none" | "" => Credential::None,
            other => anyhow::bail!("unknown auth kind {other}"),
        };
        let optional = |index: usize| {
            let value = value(index);
            (!value.is_empty()).then_some(value)
        };
        let profile = Profile {
            name: value(1),
            enabled: self.provider_enabled,
            base_url: value(3),
            api_format,
            credential,
            default_model: value(6),
            aliases: RoleModels {
                opus: optional(7),
                sonnet: optional(8),
                haiku: optional(9),
                fable: optional(10),
            },
            subagent_model: optional(11),
            fallback_models: value(12)
                .split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .collect(),
            enabled_models: self.enabled_models.clone(),
            disabled_models: self.disabled_models.clone(),
            models: self.models.clone(),
        };
        profile.validate()?;
        Ok((id, profile))
    }
}

impl ModelForm {
    #[cfg(test)]
    fn new() -> Self {
        Self::with_api_models(vec![])
    }

    fn with_api_models(api_models: Vec<ModelEntry>) -> Self {
        let api_models = config::deduplicate_model_entries(api_models);
        let count = api_models.len();
        let api_status = if count > 0 {
            format!("已加载缓存的 {count} 个 API 模型 (输入关键词过滤 / 点击填入)")
        } else {
            "点击下方 [Fetch API] 按钮可实时从网关获取可用模型".into()
        };
        Self {
            fields: vec![
                field("Model ID", ""),
                field("Label", ""),
                field("Description", ""),
                toggle_field("1M context", false),
                toggle_field("Enable now", true),
            ],
            selected: 0,
            api_models,
            api_query: String::new(),
            api_query_cursor: 0,
            api_scroll: 0,
            api_selected: 0,
            focus_api_search: false,
            api_status,
        }
    }

    fn filtered_api_models(&self) -> Vec<&ModelEntry> {
        let q = self.api_query.trim().to_lowercase();
        self.api_models
            .iter()
            .filter(|m| {
                q.is_empty()
                    || m.id.to_lowercase().contains(&q)
                    || m.label
                        .as_deref()
                        .is_some_and(|l| l.to_lowercase().contains(&q))
                    || m.description
                        .as_deref()
                        .is_some_and(|d| d.to_lowercase().contains(&q))
            })
            .collect()
    }

    fn scroll_api_list(&mut self, down: bool, delta: usize, visible_height: usize) {
        let total = self.filtered_api_models().len();
        if total == 0 {
            self.api_scroll = 0;
            self.api_selected = 0;
            return;
        }
        let max_scroll = total.saturating_sub(visible_height.max(1));
        if down {
            self.api_scroll = (self.api_scroll + delta).min(max_scroll);
        } else {
            self.api_scroll = self.api_scroll.saturating_sub(delta);
        }
        if self.api_selected < self.api_scroll {
            self.api_selected = self.api_scroll;
        } else if self.api_selected >= self.api_scroll + visible_height.max(1) {
            self.api_selected = self.api_scroll + visible_height.max(1) - 1;
        }
    }

    fn move_api_selection(&mut self, down: bool, visible_height: usize) {
        let total = self.filtered_api_models().len();
        if total == 0 {
            self.api_scroll = 0;
            self.api_selected = 0;
            return;
        }
        if down {
            if self.api_selected + 1 < total {
                self.api_selected += 1;
            }
        } else {
            self.api_selected = self.api_selected.saturating_sub(1);
        }
        let h = visible_height.max(1);
        if self.api_selected >= self.api_scroll + h {
            self.api_scroll = self.api_selected + 1 - h;
        }
        if self.api_selected < self.api_scroll {
            self.api_scroll = self.api_selected;
        }
    }

    fn pick_api_model(&mut self, index: usize) {
        let model = self.filtered_api_models().get(index).copied().cloned();
        if let Some(model) = model {
            let base_id = canonical_model_id(&model.id);
            self.fields[0].value = base_id.clone();
            self.fields[0].cursor = self.fields[0].char_count();
            if let Some(label) = &model.label {
                self.fields[1].value = label.clone();
                self.fields[1].cursor = self.fields[1].char_count();
            } else {
                self.fields[1].value = base_id.clone();
                self.fields[1].cursor = self.fields[1].char_count();
            }
            if let Some(desc) = &model.description {
                self.fields[2].value = desc.clone();
                self.fields[2].cursor = self.fields[2].char_count();
            }
            if has_1m_suffix(&model.id) {
                self.fields[3].value = "true".into();
            }
            self.api_status = format!("✓ 已填入 API 模型: {base_id}");
        }
    }

    fn to_model(&self) -> ModelEntry {
        let optional = |index: usize| {
            let value = self.fields[index].value.trim();
            (!value.is_empty()).then(|| value.to_owned())
        };
        let base_id = canonical_model_id(self.fields[0].value.trim());
        let one_m = self.fields[3].value == "true";
        let label = optional(1).map(|label| {
            if one_m && !label.to_ascii_lowercase().contains("1m") {
                format!("{label} · 1M")
            } else {
                label
            }
        });
        ModelEntry {
            id: if one_m && !base_id.is_empty() {
                format!("{base_id}[1m]")
            } else {
                base_id
            },
            label,
            description: optional(2),
        }
    }

    fn enable_now(&self) -> bool {
        self.fields[4].value == "true"
    }
}

impl RouteEditor {
    fn filtered_indices(&self) -> Vec<usize> {
        let query = self.query.trim().to_lowercase();
        self.catalog
            .iter()
            .enumerate()
            .filter_map(|(index, model)| {
                let matches = query.is_empty()
                    || model.id.to_lowercase().contains(&query)
                    || model
                        .label
                        .as_deref()
                        .is_some_and(|label| label.to_lowercase().contains(&query))
                    || model
                        .description
                        .as_deref()
                        .is_some_and(|description| description.to_lowercase().contains(&query));
                matches.then_some(index)
            })
            .collect()
    }

    fn selected_catalog_index(&self) -> Option<usize> {
        self.filtered_indices().get(self.selected).copied()
    }

    fn selected_model(&self) -> Option<&ModelEntry> {
        self.selected_catalog_index()
            .and_then(|index| self.catalog.get(index))
    }

    fn is_required(&self, id: &str) -> bool {
        if !self.provider_enabled {
            return false;
        }
        if self.disabled.contains(id) {
            return false;
        }
        id == self.default_model || self.locked.contains(id)
    }

    fn is_enabled(&self, id: &str) -> bool {
        if !self.provider_enabled {
            return false;
        }
        if self.disabled.contains(id) {
            return false;
        }
        self.is_required(id) || self.enabled.contains(id)
    }

    fn effective_id(&self, id: &str) -> String {
        let base = canonical_model_id(id);
        if self.one_m.contains(&base) {
            format!("{base}[1m]")
        } else {
            base
        }
    }

    fn toggle_selected(&mut self) {
        if !self.provider_enabled {
            self.status = "Provider is disabled · enable it from the home page first".into();
            return;
        }
        let Some(id) = self.selected_model().map(|model| model.id.clone()) else {
            self.status = "No model matches this search".into();
            return;
        };
        if self.is_enabled(&id) {
            self.enabled.remove(&id);
            self.disabled.insert(id.clone());
            self.locked.remove(&id);
            if self.default_model == id {
                if let Some(next) = self
                    .catalog
                    .iter()
                    .find(|m| self.is_enabled(&m.id) && m.id != id)
                {
                    self.default_model = next.id.clone();
                    self.status =
                        format!("Disabled {id}; default switched to {}", self.default_model);
                } else {
                    self.status = format!("Disabled {id}");
                }
            } else {
                self.status = format!("Disabled {id}");
            }
        } else {
            self.disabled.remove(&id);
            self.enabled.insert(id.clone());
            if !self.is_enabled(&self.default_model) {
                self.default_model = id.clone();
            }
            self.status = format!("Enabled {id}");
        }
    }

    fn toggle_selected_1m(&mut self) {
        let Some(id) = self.selected_model().map(|model| model.id.clone()) else {
            self.status = "No model matches this search".into();
            return;
        };
        if self.one_m.remove(&id) {
            self.status = format!("1M context disabled for {id}");
        } else {
            self.one_m.insert(id.clone());
            self.status = format!("1M context enabled for {id}");
        }
    }

    fn set_selected_default(&mut self) {
        let Some(id) = self.selected_model().map(|model| model.id.clone()) else {
            self.status = "No model matches this search".into();
            return;
        };
        self.disabled.remove(&id);
        self.enabled.remove(&id);
        self.default_model = id.clone();
        self.status = format!("Default model set to {id}");
    }

    fn enable_all_filtered(&mut self) {
        let indices = self.filtered_indices();
        let mut count = 0;
        for index in indices {
            let id = self.catalog[index].id.clone();
            self.disabled.remove(&id);
            if !self.is_required(&id) && self.enabled.insert(id) {
                count += 1;
            }
        }
        self.status = format!("Enabled {count} models");
    }

    fn disable_all_filtered(&mut self) {
        let indices = self.filtered_indices();
        let mut count = 0;
        for index in indices {
            let id = self.catalog[index].id.clone();
            if !self.is_required(&id) && self.enabled.remove(&id) {
                count += 1;
            }
        }
        self.status = format!("Disabled {count} models (required models kept)");
    }
}

fn has_1m_suffix(id: &str) -> bool {
    id.to_ascii_lowercase().ends_with("[1m]")
}

fn canonical_model_id(id: &str) -> String {
    if has_1m_suffix(id) {
        id[..id.len().saturating_sub(4)].to_owned()
    } else {
        id.to_owned()
    }
}

fn normalize_model_catalog(models: Vec<ModelEntry>) -> Vec<ModelEntry> {
    let mut normalized: BTreeMap<String, ModelEntry> = BTreeMap::new();
    for model in models {
        let id = canonical_model_id(&model.id);
        let label = model.label.map(|label| {
            label
                .strip_suffix(" · 1M")
                .or_else(|| label.strip_suffix(" 1M"))
                .unwrap_or(&label)
                .to_owned()
        });
        let entry = normalized.entry(id.clone()).or_insert_with(|| ModelEntry {
            id,
            label: None,
            description: None,
        });
        if entry.label.is_none() {
            entry.label = label;
        }
        if entry.description.is_none() {
            entry.description = model.description;
        }
    }
    normalized.into_values().collect()
}

fn apply_route_editor(profile: &mut Profile, editor: &RouteEditor) {
    if editor.is_enabled(&editor.default_model) {
        profile.default_model = editor.effective_id(&editor.default_model);
    } else if let Some(next) = editor.catalog.iter().find(|m| editor.is_enabled(&m.id)) {
        profile.default_model = editor.effective_id(&next.id);
    } else {
        profile.default_model = editor.effective_id(&editor.default_model);
    }
    for model in [
        &mut profile.aliases.opus,
        &mut profile.aliases.sonnet,
        &mut profile.aliases.haiku,
        &mut profile.aliases.fable,
        &mut profile.subagent_model,
    ]
    .into_iter()
    .flatten()
    {
        let base = canonical_model_id(model);
        if editor.is_enabled(&base) {
            *model = editor.effective_id(model);
        }
    }
    profile.fallback_models = profile
        .fallback_models
        .iter()
        .filter(|id| editor.is_enabled(&canonical_model_id(id)))
        .map(|id| editor.effective_id(id))
        .collect();
    let def_canonical = canonical_model_id(&profile.default_model);
    profile.enabled_models = editor
        .catalog
        .iter()
        .filter(|m| editor.is_enabled(&m.id) && m.id != def_canonical)
        .map(|m| editor.effective_id(&m.id))
        .collect();
    profile.disabled_models = editor
        .catalog
        .iter()
        .filter(|m| editor.disabled.contains(&m.id))
        .map(|m| editor.effective_id(&m.id))
        .collect();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormOutcome {
    Stay,
    Close,
    Submit,
}

impl FormField {
    fn char_count(&self) -> usize {
        self.value.chars().count()
    }

    fn clamp_cursor(&mut self) {
        let count = self.char_count();
        if self.cursor > count {
            self.cursor = count;
        }
    }

    fn insert_char(&mut self, ch: char) {
        self.clamp_cursor();
        let byte_offset = self
            .value
            .char_indices()
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.value.len());
        self.value.insert(byte_offset, ch);
        self.cursor += 1;
    }

    fn delete_backward(&mut self) {
        self.clamp_cursor();
        if self.cursor > 0 {
            self.cursor -= 1;
            let byte_offset = self
                .value
                .char_indices()
                .nth(self.cursor)
                .map(|(i, _)| i)
                .unwrap_or(self.value.len());
            self.value.remove(byte_offset);
        }
    }

    fn delete_forward(&mut self) {
        self.clamp_cursor();
        if self.cursor < self.char_count() {
            let byte_offset = self
                .value
                .char_indices()
                .nth(self.cursor)
                .map(|(i, _)| i)
                .unwrap_or(self.value.len());
            self.value.remove(byte_offset);
        }
    }

    fn clear_text(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }
}

fn handle_form_key(fields: &mut [FormField], selected: &mut usize, key: KeyEvent) -> FormOutcome {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
        return FormOutcome::Submit;
    }
    match key.code {
        KeyCode::Esc => return FormOutcome::Close,
        KeyCode::Tab | KeyCode::Down => {
            *selected = (*selected + 1) % fields.len();
            fields[*selected].clamp_cursor();
        }
        KeyCode::BackTab | KeyCode::Up => {
            *selected = selected.checked_sub(1).unwrap_or(fields.len() - 1);
            fields[*selected].clamp_cursor();
        }
        KeyCode::Enter => {
            if fields[*selected].toggle {
                toggle_form_field(&mut fields[*selected]);
            } else if !fields[*selected].choices.is_empty() {
                cycle_choice(&mut fields[*selected], true);
            } else if *selected + 1 < fields.len() {
                *selected += 1;
                fields[*selected].clamp_cursor();
            } else {
                return FormOutcome::Submit;
            }
        }
        KeyCode::Char(' ') if fields[*selected].toggle => {
            toggle_form_field(&mut fields[*selected]);
        }
        KeyCode::Char(' ') if !fields[*selected].choices.is_empty() => {
            cycle_choice(&mut fields[*selected], true);
        }
        KeyCode::Right if !fields[*selected].choices.is_empty() => {
            cycle_choice(&mut fields[*selected], true);
        }
        KeyCode::Right if !fields[*selected].toggle => {
            let max = fields[*selected].char_count();
            fields[*selected].cursor = (fields[*selected].cursor + 1).min(max);
        }
        KeyCode::Left if !fields[*selected].choices.is_empty() => {
            cycle_choice(&mut fields[*selected], false);
        }
        KeyCode::Left if !fields[*selected].toggle => {
            fields[*selected].cursor = fields[*selected].cursor.saturating_sub(1);
        }
        KeyCode::Home if !fields[*selected].toggle && fields[*selected].choices.is_empty() => {
            fields[*selected].cursor = 0;
        }
        KeyCode::Char('a')
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && !fields[*selected].toggle
                && fields[*selected].choices.is_empty() =>
        {
            fields[*selected].cursor = 0;
        }
        KeyCode::End if !fields[*selected].toggle && fields[*selected].choices.is_empty() => {
            fields[*selected].cursor = fields[*selected].char_count();
        }
        KeyCode::Char('e')
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && !fields[*selected].toggle
                && fields[*selected].choices.is_empty() =>
        {
            fields[*selected].cursor = fields[*selected].char_count();
        }
        KeyCode::Char('u')
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && !fields[*selected].toggle
                && fields[*selected].choices.is_empty() =>
        {
            fields[*selected].clear_text();
        }
        KeyCode::Backspace if !fields[*selected].toggle && fields[*selected].choices.is_empty() => {
            fields[*selected].delete_backward();
        }
        KeyCode::Delete if !fields[*selected].toggle && fields[*selected].choices.is_empty() => {
            fields[*selected].delete_forward();
        }
        KeyCode::Char(ch)
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !fields[*selected].toggle
                && fields[*selected].choices.is_empty() =>
        {
            fields[*selected].insert_char(ch);
        }
        _ => {}
    }
    FormOutcome::Stay
}

fn field(label: &'static str, value: &str) -> FormField {
    let count = value.chars().count();
    FormField {
        label,
        value: value.into(),
        cursor: count,
        secret: false,
        toggle: false,
        choices: &[],
    }
}

fn secret_field(label: &'static str, value: &str) -> FormField {
    let count = value.chars().count();
    FormField {
        label,
        value: value.into(),
        cursor: count,
        secret: true,
        toggle: false,
        choices: &[],
    }
}

fn toggle_field(label: &'static str, enabled: bool) -> FormField {
    FormField {
        label,
        value: enabled.to_string(),
        cursor: 0,
        secret: false,
        toggle: true,
        choices: &[],
    }
}

fn choice_field(label: &'static str, value: &str, choices: &'static [&'static str]) -> FormField {
    FormField {
        label,
        value: value.into(),
        cursor: 0,
        secret: false,
        toggle: false,
        choices,
    }
}

fn cycle_choice(field: &mut FormField, forward: bool) {
    let current = field
        .choices
        .iter()
        .position(|choice| *choice == field.value)
        .unwrap_or(0);
    let index = if forward {
        (current + 1) % field.choices.len()
    } else {
        current.checked_sub(1).unwrap_or(field.choices.len() - 1)
    };
    field.value = field.choices[index].into();
    field.cursor = 0;
}

fn toggle_form_field(field: &mut FormField) {
    field.value = (field.value != "true").to_string();
    field.cursor = 0;
}

fn draw_form(
    frame: &mut ratatui::Frame,
    area: Rect,
    title: &str,
    fields: &[FormField],
    selected: usize,
) {
    let inner = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(ROUTE))
        .inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(panel(title, true), area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(1); fields.len()])
        .split(inner);
    for (index, field) in fields.iter().enumerate() {
        let shown = if !field.choices.is_empty() {
            format!("‹ {} ›", field.value)
        } else if field.toggle {
            if field.value == "true" {
                "[● ON]".into()
            } else {
                "[○ OFF]".into()
            }
        } else if field.secret && index != selected && !field.value.is_empty() {
            "••••••".into()
        } else if index == selected {
            let chars: Vec<char> = field.value.chars().collect();
            let cursor = field.cursor.min(chars.len());
            let mut s = String::new();
            for (i, &ch) in chars.iter().enumerate() {
                if i == cursor {
                    s.push('▌');
                }
                s.push(ch);
            }
            if cursor >= chars.len() {
                s.push('▌');
            }
            s
        } else {
            field.value.clone()
        };
        let line = Line::from(vec![
            Span::styled(
                format!("{:>18}  ", field.label),
                Style::default().fg(if index == selected { ROUTE } else { MUTED }),
            ),
            Span::styled(
                shown,
                Style::default()
                    .fg(if field.toggle && field.value == "true" {
                        CONNECTED
                    } else {
                        Color::Reset
                    })
                    .add_modifier(if index == selected {
                        Modifier::REVERSED
                    } else {
                        Modifier::empty()
                    }),
            ),
        ]);
        frame.render_widget(Paragraph::new(line), rows[index]);
    }
}

fn model_form_areas(area: Rect) -> (Rect, Rect) {
    if area.width >= 80 {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(38), Constraint::Min(36)])
            .split(area);
        (cols[0], cols[1])
    } else {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(8), Constraint::Min(6)])
            .split(area);
        (rows[0], rows[1])
    }
}

fn draw_model_form(frame: &mut ratatui::Frame, area: Rect, form: &ModelForm) {
    frame.render_widget(Clear, area);
    frame.render_widget(
        panel(
            " 添加模型 (Add Model) · 支持手动输入或从右侧 API 候选列表直接点选 ",
            true,
        ),
        area,
    );
    let inner = panel_inner(area);
    let content_area = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(2),
    );
    let (form_area, api_area) = model_form_areas(content_area);

    frame.render_widget(
        panel(" 模型信息 (Model Info) ", !form.focus_api_search),
        form_area,
    );
    let form_inner = panel_inner(form_area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(1); form.fields.len()])
        .split(form_inner);
    for (index, field) in form.fields.iter().enumerate() {
        let is_current = index == form.selected && !form.focus_api_search;
        let shown = if field.toggle {
            if field.label == "Enable now" {
                if field.value == "true" {
                    "[● 保存后启用]".into()
                } else {
                    "[○ 保存但暂不启用]".into()
                }
            } else if field.value == "true" {
                "[● ON 1M 长上下文]".into()
            } else {
                "[○ OFF 标准上下文]".into()
            }
        } else if is_current {
            let chars: Vec<char> = field.value.chars().collect();
            let cursor = field.cursor.min(chars.len());
            let mut s = String::new();
            for (i, &ch) in chars.iter().enumerate() {
                if i == cursor {
                    s.push('▌');
                }
                s.push(ch);
            }
            if cursor >= chars.len() {
                s.push('▌');
            }
            s
        } else {
            field.value.clone()
        };
        let line = Line::from(vec![
            Span::styled(
                format!("{:>13}  ", field.label),
                Style::default().fg(if is_current { ROUTE } else { MUTED }),
            ),
            Span::styled(
                shown,
                Style::default()
                    .fg(if field.toggle && field.value == "true" {
                        CONNECTED
                    } else {
                        Color::Reset
                    })
                    .add_modifier(if is_current {
                        Modifier::REVERSED
                    } else {
                        Modifier::empty()
                    }),
            ),
        ]);
        if index < rows.len() {
            frame.render_widget(Paragraph::new(line), rows[index]);
        }
    }

    let api_count = form.api_models.len();
    let filtered = form.filtered_api_models();
    let filtered_count = filtered.len();
    let api_title = if form.api_query.is_empty() {
        format!(" ⟳ 网关 API 可选模型 ({api_count}) ")
    } else {
        format!(" ⟳ 网关 API 可选模型 ({filtered_count}/{api_count}) ")
    };
    frame.render_widget(panel(&api_title, form.focus_api_search), api_area);
    let api_inner = panel_inner(api_area);
    if form.api_models.is_empty() {
        let msg = vec![
            Line::raw(""),
            Line::styled(" 暂无已缓存的 API 模型", Style::default().fg(MUTED)),
            Line::raw(""),
            Line::styled(
                " 点击下方 [Fetch API (Ctrl+F)] 按钮",
                Style::default().fg(ROUTE),
            ),
            Line::styled(
                " 即可实时从网关获取全部可用模型列表",
                Style::default().fg(ROUTE),
            ),
            Line::raw(""),
            Line::styled(
                format!(" 提示: {}", form.api_status),
                Style::default().fg(WARNING),
            ),
        ];
        frame.render_widget(Paragraph::new(msg).wrap(Wrap { trim: false }), api_inner);
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .split(api_inner);

        let search_text = if form.focus_api_search {
            let chars: Vec<char> = form.api_query.chars().collect();
            let cursor = form.api_query_cursor.min(chars.len());
            let mut s = String::new();
            for (i, &ch) in chars.iter().enumerate() {
                if i == cursor {
                    s.push('▌');
                }
                s.push(ch);
            }
            if cursor >= chars.len() {
                s.push('▌');
            }
            s
        } else if form.api_query.is_empty() {
            "点击此处或按 Tab 搜索过滤模型...".into()
        } else {
            form.api_query.clone()
        };

        let search_line = Line::from(vec![
            Span::styled(
                " 🔍 搜索: ",
                Style::default().fg(if form.focus_api_search { ROUTE } else { MUTED }),
            ),
            Span::styled(
                search_text,
                Style::default()
                    .fg(if form.focus_api_search {
                        Color::White
                    } else {
                        MUTED
                    })
                    .add_modifier(if form.focus_api_search {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
        ]);
        frame.render_widget(Paragraph::new(search_line), chunks[0]);

        let div_text = "─".repeat(usize::from(chunks[1].width));
        frame.render_widget(
            Paragraph::new(div_text).style(Style::default().fg(Color::DarkGray)),
            chunks[1],
        );

        let list_area = chunks[2];
        let list_height = usize::from(list_area.height);

        if filtered.is_empty() {
            let empty_msg = vec![
                Line::raw(""),
                Line::styled(
                    format!(" 未找到匹配 \"{}\" 的模型", form.api_query),
                    Style::default().fg(WARNING),
                ),
                Line::styled(" 按 Esc 清除搜索关键词", Style::default().fg(MUTED)),
            ];
            frame.render_widget(Paragraph::new(empty_msg), list_area);
        } else {
            let visible_items: Vec<ListItem> = filtered
                .iter()
                .enumerate()
                .skip(form.api_scroll)
                .take(list_height)
                .map(|(idx, model)| {
                    let is_active = form.focus_api_search && idx == form.api_selected;
                    let mut spans = vec![
                        Span::styled(
                            if is_active { "▶ " } else { "● " },
                            Style::default().fg(if is_active { WARNING } else { CONNECTED }),
                        ),
                        Span::styled(
                            &model.id,
                            Style::default()
                                .fg(if is_active { WARNING } else { Color::White })
                                .add_modifier(Modifier::BOLD),
                        ),
                    ];
                    if let Some(label) = &model.label {
                        spans.push(Span::styled(
                            format!(" ({label})"),
                            Style::default().fg(MUTED),
                        ));
                    }
                    ListItem::new(Line::from(spans))
                })
                .collect();
            frame.render_widget(List::new(visible_items), list_area);

            if filtered_count > list_height {
                draw_scrollbar(
                    frame,
                    list_area,
                    filtered_count,
                    form.api_scroll,
                    list_height,
                );
            }
        }
    }
}

fn draw_proxy_manager(frame: &mut ratatui::Frame, area: Rect, manager: &ProxyManager) {
    frame.render_widget(panel(" Proxy control · P ", true), area);
    let inner = panel_inner(area);
    let running = manager
        .runtime
        .as_ref()
        .is_some_and(|status| status.running);
    let installed = manager
        .service
        .as_ref()
        .is_some_and(|status| status.installed);
    let runtime_color = if running { CONNECTED } else { WARNING };
    let rail = Line::from(vec![
        Span::styled("Claude Code", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled("  ──▶  ", Style::default().fg(MUTED)),
        Span::styled(
            format!("CCSW proxy {}", if running { '●' } else { '○' }),
            Style::default()
                .fg(runtime_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ──▶  ", Style::default().fg(MUTED)),
        Span::styled("Provider APIs", Style::default().fg(ROUTE)),
    ]);
    frame.render_widget(
        Paragraph::new(rail).alignment(Alignment::Center),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );

    let runtime = manager.runtime.as_ref();
    let service = manager.service.as_ref();
    let details = vec![
        Line::raw(""),
        detail(
            "Runtime",
            if running {
                "● Running in background"
            } else {
                "○ Stopped"
            },
        ),
        detail(
            "Listen",
            runtime
                .map(|status| status.listen.as_str())
                .unwrap_or("unknown"),
        ),
        detail(
            "PID / routes",
            &runtime
                .map(|status| {
                    format!(
                        "{} / {}",
                        status
                            .pid
                            .map(|pid| pid.to_string())
                            .unwrap_or_else(|| "—".into()),
                        status.routes
                    )
                })
                .unwrap_or_else(|| "unknown".into()),
        ),
        Line::raw(""),
        detail(
            "Start at login",
            if installed {
                "● Enabled"
            } else {
                "○ Disabled"
            },
        ),
        detail(
            "Service",
            service.map(|status| status.manager).unwrap_or("unknown"),
        ),
        detail(
            "Definition",
            &service
                .map(|status| status.path.display().to_string())
                .unwrap_or_else(|| "unknown".into()),
        ),
        Line::raw(""),
        Line::styled(
            "Sync all starts the proxy automatically. You can close the TUI afterward.",
            Style::default().fg(MUTED),
        ),
    ];
    frame.render_widget(
        Paragraph::new(details).wrap(Wrap { trim: false }),
        Rect::new(
            inner.x,
            inner.y.saturating_add(1),
            inner.width,
            inner.height.saturating_sub(6),
        ),
    );

    frame.render_widget(
        Paragraph::new(manager.message.as_str())
            .alignment(Alignment::Center)
            .style(Style::default().fg(if manager.error { ERROR } else { runtime_color })),
        Rect::new(
            inner.x,
            area.y.saturating_add(area.height).saturating_sub(6),
            inner.width,
            1,
        ),
    );
    draw_proxy_controls(frame, area, manager);
}

fn proxy_controls(area: Rect) -> Vec<(ProxyControl, Rect)> {
    let bottom = area.y.saturating_add(area.height);
    let first = [
        (ProxyControl::Start, "Start"),
        (ProxyControl::Stop, "Stop"),
        (ProxyControl::Refresh, "Refresh"),
    ];
    let second = [
        (ProxyControl::EnableAtLogin, "Enable at login"),
        (ProxyControl::DisableAtLogin, "Disable at login"),
        (ProxyControl::Close, "Close"),
    ];
    proxy_button_row_rects(area, bottom.saturating_sub(4), &first)
        .into_iter()
        .chain(proxy_button_row_rects(
            area,
            bottom.saturating_sub(2),
            &second,
        ))
        .collect()
}

fn proxy_button_row_rects(
    area: Rect,
    y: u16,
    buttons: &[(ProxyControl, &str)],
) -> Vec<(ProxyControl, Rect)> {
    let gap = 2_u16;
    let widths = buttons
        .iter()
        .map(|(_, label)| {
            u16::try_from(label.chars().count())
                .unwrap_or(u16::MAX)
                .saturating_add(4)
        })
        .collect::<Vec<_>>();
    let total = widths.iter().copied().sum::<u16>().saturating_add(
        gap.saturating_mul(u16::try_from(buttons.len().saturating_sub(1)).unwrap_or(u16::MAX)),
    );
    let mut x = area.x.saturating_add(area.width.saturating_sub(total) / 2);
    buttons
        .iter()
        .zip(widths)
        .map(|((control, _), width)| {
            let rect = Rect::new(x, y, width.min(area.width), 1);
            x = x.saturating_add(width).saturating_add(gap);
            (*control, rect)
        })
        .collect()
}

fn proxy_control_index(control: ProxyControl) -> usize {
    ProxyManager::CONTROLS
        .iter()
        .position(|candidate| *candidate == control)
        .unwrap_or(0)
}

fn proxy_control_key(control: ProxyControl) -> KeyEvent {
    let code = match control {
        ProxyControl::Start => KeyCode::Char('s'),
        ProxyControl::Stop => KeyCode::Char('x'),
        ProxyControl::Refresh => KeyCode::Char('r'),
        ProxyControl::EnableAtLogin => KeyCode::Char('i'),
        ProxyControl::DisableAtLogin => KeyCode::Char('u'),
        ProxyControl::Close => KeyCode::Esc,
    };
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn draw_proxy_controls(frame: &mut ratatui::Frame, area: Rect, manager: &ProxyManager) {
    let running = manager
        .runtime
        .as_ref()
        .is_some_and(|status| status.running);
    let installed = manager
        .service
        .as_ref()
        .is_some_and(|status| status.installed);
    for (control, rect) in proxy_controls(area) {
        let label = match control {
            ProxyControl::Start => "Start",
            ProxyControl::Stop => "Stop",
            ProxyControl::Refresh => "Refresh",
            ProxyControl::EnableAtLogin => "Enable at login",
            ProxyControl::DisableAtLogin => "Disable at login",
            ProxyControl::Close => "Close",
        };
        let disabled = matches!(control, ProxyControl::Start) && running
            || matches!(control, ProxyControl::Stop) && !running
            || matches!(control, ProxyControl::EnableAtLogin) && installed
            || matches!(control, ProxyControl::DisableAtLogin) && !installed;
        let selected = manager.selected_control() == control;
        let style = if selected {
            Style::default()
                .fg(Color::Black)
                .bg(if disabled { MUTED } else { ROUTE })
                .add_modifier(Modifier::BOLD)
        } else if disabled {
            Style::default().fg(MUTED).add_modifier(Modifier::DIM)
        } else if matches!(control, ProxyControl::Start | ProxyControl::EnableAtLogin) {
            Style::default().fg(CONNECTED)
        } else if matches!(control, ProxyControl::Stop | ProxyControl::DisableAtLogin) {
            Style::default().fg(WARNING)
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

fn draw_confirmation(frame: &mut ratatui::Frame, area: Rect, message: &str) {
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw(message),
            Line::raw(""),
            Line::styled(
                "Enter/y confirm · n/Esc cancel",
                Style::default().fg(WARNING),
            ),
        ])
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .block(panel(" Confirm ", true)),
        area,
    );
}

fn ui_areas(area: Rect, focus: Focus, view_mode: ViewMode) -> UiAreas {
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

fn app_rows(area: Rect) -> [Rect; 3] {
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

fn footer_controls(area: Rect, compact: bool, view_mode: ViewMode) -> Vec<(FooterControl, Rect)> {
    if area.width == 0 || area.height == 0 {
        return vec![];
    }
    const HOME_WIDE: &[(FooterControl, u16)] = &[
        (FooterControl::Launch, 13),
        (FooterControl::AddProfile, 13),
        (FooterControl::Sync, 14),
        (FooterControl::Proxy, 10),
        (FooterControl::Help, 8),
        (FooterControl::Quit, 8),
    ];
    const HOME_COMPACT: &[(FooterControl, u16)] = &[
        (FooterControl::Launch, 9),
        (FooterControl::AddProfile, 9),
        (FooterControl::Sync, 8),
        (FooterControl::Proxy, 7),
        (FooterControl::Help, 5),
        (FooterControl::Quit, 5),
    ];
    const PROVIDER_WIDE: &[(FooterControl, u16)] = &[
        (FooterControl::Back, 14),
        (FooterControl::Launch, 11),
        (FooterControl::Sync, 14),
        (FooterControl::Resume, 11),
        (FooterControl::New, 8),
        (FooterControl::Proxy, 10),
        (FooterControl::Help, 8),
        (FooterControl::Quit, 8),
    ];
    const PROVIDER_COMPACT: &[(FooterControl, u16)] = &[
        (FooterControl::Back, 8),
        (FooterControl::Models, 8),
        (FooterControl::Details, 8),
        (FooterControl::Launch, 7),
        (FooterControl::Sync, 8),
        (FooterControl::Help, 5),
        (FooterControl::Quit, 5),
    ];
    const HOME_TINY: &[(FooterControl, u16)] = &[
        (FooterControl::Launch, 7),
        (FooterControl::AddProfile, 7),
        (FooterControl::Sync, 6),
        (FooterControl::Help, 3),
        (FooterControl::Quit, 3),
    ];
    const PROVIDER_TINY: &[(FooterControl, u16)] = &[
        (FooterControl::Back, 6),
        (FooterControl::Models, 5),
        (FooterControl::Details, 5),
        (FooterControl::Launch, 5),
        (FooterControl::Sync, 5),
        (FooterControl::Help, 3),
        (FooterControl::Quit, 3),
    ];
    const ALL_WIDE: &[(FooterControl, u16)] = &[
        (FooterControl::Back, 14),
        (FooterControl::Sync, 14),
        (FooterControl::Proxy, 10),
        (FooterControl::Help, 8),
        (FooterControl::Quit, 8),
    ];
    const ALL_COMPACT: &[(FooterControl, u16)] = &[
        (FooterControl::Back, 8),
        (FooterControl::Sync, 8),
        (FooterControl::Proxy, 7),
        (FooterControl::Help, 5),
        (FooterControl::Quit, 5),
    ];
    const ALL_TINY: &[(FooterControl, u16)] = &[
        (FooterControl::Back, 6),
        (FooterControl::Sync, 5),
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

fn provider_detail_cards(area: Rect) -> (Rect, Rect) {
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

fn showcase_controls(area: Rect) -> Vec<(ShowcaseControl, Rect)> {
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

fn catalog_add_button_rect(search_area: Rect) -> Option<Rect> {
    let inner = panel_inner(search_area);
    if inner.width >= 10 {
        let btn_w = if inner.width >= 35 { 17 } else { 6 };
        let btn_x = inner.x + inner.width.saturating_sub(btn_w);
        Some(Rect::new(btn_x, inner.y, btn_w, 1))
    } else {
        None
    }
}

fn detail_controls(area: Rect) -> Vec<(DetailControl, Rect)> {
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

fn draw_detail_controls(frame: &mut ratatui::Frame, area: Rect) {
    for (control, rect) in detail_controls(area) {
        let (label, style) = match control {
            DetailControl::FetchModels => (
                "[Fetch models (r)]",
                Style::default()
                    .fg(Color::Black)
                    .bg(ROUTE)
                    .add_modifier(Modifier::BOLD),
            ),
            DetailControl::Edit => ("[Edit route (E)]", Style::default().fg(WARNING)),
        };
        frame.render_widget(
            Paragraph::new(label)
                .alignment(Alignment::Center)
                .style(style),
            rect,
        );
    }
}

fn panel_inner(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

fn contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

fn clicked_list_index(
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

fn scrollbar_index(
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

fn modal_area(screen: Rect) -> Rect {
    centered_rect(
        72.min(screen.width.saturating_sub(4)),
        24.min(screen.height.saturating_sub(2)),
        screen,
    )
}

fn modal_area_for(modal: &Modal, screen: Rect) -> Rect {
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

fn route_editor_offset(editor: &RouteEditor, viewport_height: u16) -> usize {
    let visible = usize::from(viewport_height.max(1));
    editor.selected.saturating_add(1).saturating_sub(visible)
}

fn modal_button_rects(area: Rect, count: usize) -> Vec<Rect> {
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

fn draw_modal_buttons(frame: &mut ratatui::Frame, area: Rect, labels: &[&str]) {
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

fn draw_scrollbar(
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

fn panel(title: &str, active: bool) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(if active { ROUTE } else { MUTED }))
}

fn detail(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<12}"), Style::default().fg(MUTED)),
        Span::raw(value.to_owned()),
    ])
}

fn wrap_styled_segments(segments: Vec<(String, Style)>, max_width: u16) -> Vec<Line<'static>> {
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

fn all_enabled_lines(provider_count: usize, model_count: usize, width: u16) -> Vec<Line<'static>> {
    let active = model_count > 0;
    let mut lines = wrap_styled_segments(
        vec![
            (
                if active { " ● " } else { " ○ " }.into(),
                Style::default().fg(if active { CONNECTED } else { MUTED }),
            ),
            (
                "All Enabled / 全部启用".into(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            (
                format!("  {model_count} models · {provider_count} providers"),
                Style::default().fg(CONNECTED),
            ),
        ],
        width,
    );
    lines.push(Line::raw(""));
    lines
}

fn home_profile_lines(
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

fn visible_variable_items(heights: &[usize], offset: usize, viewport_height: usize) -> usize {
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

fn clicked_variable_item(area: Rect, row: u16, offset: usize, heights: &[usize]) -> Option<usize> {
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

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn unique_profile_id(base: &str, profiles: &BTreeMap<String, Profile>) -> String {
    if !profiles.contains_key(base) {
        return base.into();
    }
    (2..)
        .map(|index| format!("{base}-{index}"))
        .find(|id| !profiles.contains_key(id))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use std::path::PathBuf;

    #[test]
    fn renders_empty_state_in_narrow_terminal() {
        let paths = AppPaths {
            config: PathBuf::from("/tmp/config"),
            state: PathBuf::from("/tmp/state"),
            cache: PathBuf::from("/tmp/cache"),
            runtime_dir: PathBuf::from("/tmp/runtime"),
        };
        let mut app = App {
            paths,
            config: Config::default(),
            cache: ModelCache::default(),
            cwd: "/tmp".into(),
            view_mode: ViewMode::Home,
            home_all_selected: false,
            profile_idx: 0,
            model_idx: 0,
            profile_offset: 0,
            model_offset: 0,
            focus: Focus::Profiles,
            launch_mode: LaunchMode::New,
            status: "Ready".into(),
            status_error: false,
            modal: None,
            current_session: None,
            proxy_status: None,
            forwarded_args: vec![],
            provider_editor: None,
        };
        let backend = TestBackend::new(72, 22);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("All Enabled"));
        assert!(rendered.contains("0 models"));
        assert!(rendered.contains("Routers"));
        assert!(rendered.contains("Enter"));
    }

    #[test]
    fn renders_home_and_provider_at_minimal_terminal_size() {
        let mut app = interactive_test_app();
        let backend = TestBackend::new(36, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();

        app.enter_provider_view();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        app.focus = Focus::Details;
        terminal.draw(|frame| app.draw(frame)).unwrap();
    }

    #[test]
    fn help_opens_on_the_current_view_and_switches_sections() {
        let mut app = interactive_test_app();
        app.view_mode = ViewMode::AllEnabled;
        app.open_help();
        assert!(matches!(
            app.modal,
            Some(Modal::Help(HelpModal {
                section: HelpSection::AllEnabled,
                ..
            }))
        ));

        app.handle_modal(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
            .unwrap();
        assert!(matches!(
            app.modal,
            Some(Modal::Help(HelpModal {
                section: HelpSection::Provider,
                ..
            }))
        ));
        app.handle_modal(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE))
            .unwrap();
        assert!(matches!(
            app.modal,
            Some(Modal::Help(HelpModal {
                section: HelpSection::Home,
                ..
            }))
        ));
        app.handle_modal(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert!(app.modal.is_none());
    }

    #[test]
    fn help_renders_in_full_and_narrow_terminals() {
        let mut app = interactive_test_app();
        app.view_mode = ViewMode::Provider;
        app.open_help();

        let backend = TestBackend::new(90, 26);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("1 Home"));
        assert!(rendered.contains("Help · Provider"));
        assert!(rendered.contains("Space / d / 1"));

        let backend = TestBackend::new(36, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Help"));
        assert!(rendered.contains("Provider"));
    }

    #[test]
    fn all_enabled_is_the_first_home_row_and_excludes_disabled_providers() {
        let mut app = interactive_test_app();
        app.config.profiles.get_mut("two").unwrap().enabled = false;

        let models = app.all_managed_models();
        assert_eq!(models.len(), 2);
        assert!(models.iter().all(|entry| entry.profile_id == "one"));
        assert_eq!(
            app.home_profile_item_heights(Rect::new(0, 0, 120, 30))
                .len(),
            3
        );

        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.find("All Enabled").unwrap() < rendered.find("One").unwrap());
        assert!(rendered.contains("2 models · 1 providers"));

        app.home_all_selected = true;
        app.enter_all_enabled_view();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("All enabled models"));
        assert!(rendered.contains("model-a"));
        assert!(!rendered.contains("Two  two"));
    }

    #[test]
    fn disabled_provider_editor_marks_every_model_unavailable() {
        let mut app = interactive_test_app();
        app.config.profiles.get_mut("one").unwrap().enabled = false;
        let mut editor = app.create_route_editor_for("one".into()).unwrap();

        assert!(
            editor
                .catalog
                .iter()
                .all(|model| !editor.is_enabled(&model.id))
        );
        editor.toggle_selected();
        assert!(editor.status.contains("Provider is disabled"));
        assert!(
            editor
                .catalog
                .iter()
                .all(|model| !editor.is_enabled(&model.id))
        );
    }

    #[test]
    fn all_enabled_page_can_disable_and_reenable_the_same_model() {
        let mut app = interactive_test_app();
        app.home_all_selected = true;
        app.enter_all_enabled_view();

        let selected = app
            .all_managed_models()
            .into_iter()
            .find(|entry| entry.profile_id == "one" && entry.model.id == "model-b")
            .unwrap();
        assert!(selected.enabled);
        let disabled = app.toggled_global_model_profile(&selected).unwrap();
        assert!(disabled.disabled_models.contains(&"model-b".into()));
        assert!(!disabled.enabled_models.contains(&"model-b".into()));
        app.config.profiles.insert("one".into(), disabled);

        let selected = app
            .all_managed_models()
            .into_iter()
            .find(|entry| entry.profile_id == "one" && entry.model.id == "model-b")
            .unwrap();
        assert!(!selected.enabled, "disabled model must remain visible");
        let enabled = app.toggled_global_model_profile(&selected).unwrap();
        assert!(!enabled.disabled_models.contains(&"model-b".into()));
        assert!(enabled.enabled_models.contains(&"model-b".into()));
    }

    #[test]
    fn aggregate_editor_uses_the_target_provider_catalog() {
        let mut app = interactive_test_app();
        app.home_all_selected = true;
        app.enter_all_enabled_view();

        let editor = app.create_route_editor_for("two".into()).unwrap();
        assert_eq!(
            editor
                .catalog
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["model-a", "model-b"]
        );
    }

    #[test]
    fn mouse_wheel_and_click_navigate_lists() {
        let mut app = interactive_test_app();
        let screen = Rect::new(0, 0, 120, 30);
        let areas = ui_areas(screen, app.focus, app.view_mode);
        let profiles = areas.profiles.unwrap();
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: profiles.x + 1,
                row: profiles.y + 1,
                modifiers: KeyModifiers::NONE,
            },
            screen,
        )
        .unwrap();
        assert_eq!(app.profile_idx, 1);
        assert_eq!(app.focus, Focus::Profiles);

        app.view_mode = ViewMode::Provider;
        app.focus = Focus::Models;
        let models = ui_areas(screen, app.focus, app.view_mode).models.unwrap();
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: models.x + 1,
                row: models.y + 5,
                modifiers: KeyModifiers::NONE,
            },
            screen,
        )
        .unwrap();
        assert_eq!(app.model_idx, 1);
        assert_eq!(app.focus, Focus::Models);
    }

    #[test]
    fn mouse_click_changes_launch_mode_switch() {
        let mut app = interactive_test_app();
        app.current_session = Some(SessionCapture {
            session_id: "session-test".into(),
            model_id: "model-a".into(),
            cwd: Some("/tmp".into()),
            event: Some("test".into()),
        });
        app.launch_mode = LaunchMode::Resume;
        app.enter_provider_view();
        let screen = Rect::new(0, 0, 120, 30);
        let footer = ui_areas(screen, app.focus, app.view_mode).footer;
        let new_button = footer_controls(footer, false, app.view_mode)
            .into_iter()
            .find(|(control, _)| *control == FooterControl::New)
            .unwrap()
            .1;
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: new_button.x,
                row: new_button.y,
                modifiers: KeyModifiers::NONE,
            },
            screen,
        )
        .unwrap();
        assert_eq!(app.launch_mode, LaunchMode::New);
    }

    #[test]
    fn scrollbar_click_and_drag_cover_the_full_list() {
        let area = Rect::new(4, 3, 20, 12);
        assert_eq!(scrollbar_index(area, 23, 4, 100, 10), Some(0));
        assert_eq!(scrollbar_index(area, 23, 13, 100, 10), Some(99));
        assert_eq!(scrollbar_index(area, 22, 13, 100, 10), None);

        let mut app = interactive_test_app();
        let template = app.config.profiles["one"].clone();
        for index in 0..30 {
            app.config
                .profiles
                .insert(format!("route-{index:02}"), template.clone());
        }
        let screen = Rect::new(0, 0, 120, 20);
        let panel = ui_areas(screen, app.focus, app.view_mode).profiles.unwrap();
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: panel.x + panel.width - 1,
                row: panel.y + panel.height - 2,
                modifiers: KeyModifiers::NONE,
            },
            screen,
        )
        .unwrap();
        assert_eq!(app.profile_idx, app.config.profiles.len() - 1);
    }

    #[test]
    fn route_details_exposes_clickable_provider_editor() {
        let mut app = interactive_test_app();
        app.view_mode = ViewMode::Provider;
        app.focus = Focus::Details;
        let screen = Rect::new(0, 0, 120, 30);
        let details = ui_areas(screen, app.focus, app.view_mode).details.unwrap();
        let edit = detail_controls(details)
            .into_iter()
            .find(|(control, _)| *control == DetailControl::Edit)
            .unwrap()
            .1;
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: edit.x,
                row: edit.y,
                modifiers: KeyModifiers::NONE,
            },
            screen,
        )
        .unwrap();
        assert!(matches!(app.modal, Some(Modal::Profile(_))));
    }

    #[test]
    fn provider_page_keeps_global_sync_and_removes_duplicate_manager_controls() {
        let mut app = interactive_test_app();
        app.view_mode = ViewMode::Provider;
        app.focus = Focus::Details;
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(!rendered.contains("Manage models"));
        assert!(!rendered.contains("Enable all"));
        assert!(rendered.contains("Claude /model"));
        assert!(
            footer_controls(Rect::new(0, 27, 120, 3), false, app.view_mode)
                .iter()
                .any(|(control, _)| *control == FooterControl::Sync)
        );
    }

    #[test]
    fn proxy_manager_renders_and_supports_keyboard_and_mouse_navigation() {
        let mut app = interactive_test_app();
        app.modal = Some(Modal::Proxy(ProxyManager {
            runtime: Some(proxy::ProxyStatus {
                running: true,
                listen: "127.0.0.1:17321".into(),
                routes: 3,
                pid: Some(4242),
            }),
            service: Some(proxy::ProxyServiceStatus {
                installed: true,
                manager: "launchd",
                path: PathBuf::from("/tmp/com.ccsw.proxy.plist"),
            }),
            selected: 0,
            message: "Ready".into(),
            error: false,
        }));
        let screen = Rect::new(0, 0, 100, 30);
        let backend = TestBackend::new(screen.width, screen.height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Proxy control"));
        assert!(rendered.contains("Running in background"));
        assert!(rendered.contains("Enable at login"));
        assert!(rendered.contains("Disable at login"));

        app.handle_modal(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
            .unwrap();
        assert!(matches!(
            app.modal,
            Some(Modal::Proxy(ProxyManager { selected: 1, .. }))
        ));

        let modal = modal_area_for(app.modal.as_ref().unwrap(), screen);
        let close = proxy_controls(modal)
            .into_iter()
            .find(|(control, _)| *control == ProxyControl::Close)
            .unwrap()
            .1;
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: close.x,
                row: close.y,
                modifiers: KeyModifiers::NONE,
            },
            screen,
        )
        .unwrap();
        assert!(app.modal.is_none());
    }

    #[test]
    fn route_editor_searches_toggles_and_changes_default() {
        let models = ["alpha", "beta", "gamma"]
            .into_iter()
            .map(|id| ModelEntry {
                id: id.into(),
                label: Some(id.to_uppercase()),
                description: None,
            })
            .collect();
        let mut editor = RouteEditor {
            profile_id: "route".into(),
            original_profile: interactive_test_app().config.profiles["one"].clone(),
            provider_enabled: true,
            catalog: models,
            enabled: BTreeSet::new(),
            disabled: BTreeSet::new(),
            locked: BTreeSet::new(),
            default_model: "alpha".into(),
            one_m: BTreeSet::new(),
            query: "bet".into(),
            selected: 0,
            search_active: false,
            status: String::new(),
        };
        assert_eq!(editor.filtered_indices(), [1]);
        editor.toggle_selected();
        assert!(editor.enabled.contains("beta"));
        editor.toggle_selected_1m();
        assert_eq!(editor.effective_id("beta"), "beta[1m]");
        editor.set_selected_default();
        assert_eq!(editor.default_model, "beta");
        assert!(!editor.enabled.contains("beta"));
        assert!(editor.is_enabled("beta"));
        assert!(!editor.is_enabled("gamma"));
    }

    #[test]
    fn manual_model_form_supports_keyboard_1m_toggle() {
        let mut form = ModelForm::new();
        form.fields[0].value = "manual-model[1m]".into();
        form.fields[1].value = "Manual model".into();
        form.selected = 3;

        handle_form_key(
            &mut form.fields,
            &mut form.selected,
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
        );
        let model = form.to_model();
        assert_eq!(model.id, "manual-model[1m]");
        assert_eq!(model.label.as_deref(), Some("Manual model · 1M"));

        handle_form_key(
            &mut form.fields,
            &mut form.selected,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert_eq!(form.to_model().id, "manual-model");
    }

    #[test]
    fn model_form_can_save_a_model_without_enabling_it() {
        let mut form = ModelForm::new();
        form.fields[0].value = "parked-model".into();
        form.fields[4].value = "false".into();

        assert_eq!(form.to_model().id, "parked-model");
        assert!(!form.enable_now());
    }

    #[test]
    fn saving_a_disabled_model_keeps_it_visible_in_the_provider_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = interactive_test_app();
        app.paths.config = dir.path().join("config.toml");
        std::fs::write(&app.paths.config, toml::to_string(&app.config).unwrap()).unwrap();
        app.enter_provider_view();

        let mut form = ModelForm::new();
        form.fields[0].value = "parked-model".into();
        form.fields[4].value = "false".into();
        app.modal = Some(Modal::Model(form));
        app.handle_modal(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
            .unwrap();

        let profile = &app.config.profiles["one"];
        assert!(
            profile
                .models
                .iter()
                .any(|model| model.id == "parked-model")
        );
        assert!(
            profile
                .disabled_models
                .iter()
                .any(|id| id == "parked-model")
        );
        assert!(!profile.enabled_models.iter().any(|id| id == "parked-model"));
        let selected = app
            .provider_editor
            .as_ref()
            .unwrap()
            .selected_model()
            .unwrap();
        assert_eq!(selected.id, "parked-model");
        assert!(
            !app.provider_editor
                .as_ref()
                .unwrap()
                .is_enabled("parked-model")
        );
    }

    #[test]
    fn blank_manual_model_stays_invalid_with_1m_enabled() {
        let mut form = ModelForm::new();
        form.fields[3].value = "true".into();
        assert!(canonical_model_id(form.fields[0].value.trim()).is_empty());
        assert!(form.to_model().id.is_empty());
    }

    #[test]
    fn profile_form_cycles_api_format_and_auth_choices() {
        let mut form = ProfileForm::new();
        form.fields[0].value = "openai".into();
        form.fields[1].value = "OpenAI".into();
        form.fields[3].value = "https://api.example/v1".into();
        form.fields[5].value = "secret".into();
        form.fields[6].value = "gpt-test".into();
        form.selected = 2;
        handle_form_key(
            &mut form.fields,
            &mut form.selected,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        form.selected = 4;
        handle_form_key(
            &mut form.fields,
            &mut form.selected,
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
        );
        let (_, profile) = form.to_profile().unwrap();
        assert_eq!(profile.api_format, ApiFormat::OpenaiChat);
        assert!(matches!(profile.credential, Credential::XApiKey { .. }));
    }

    #[test]
    fn route_editor_persists_1m_for_default_aliases_and_enabled_models() {
        let mut app = interactive_test_app();
        let profile = app.config.profiles.get_mut("one").unwrap();
        profile.aliases.sonnet = Some("model-a".into());
        let editor = RouteEditor {
            profile_id: "one".into(),
            original_profile: profile.clone(),
            provider_enabled: true,
            catalog: profile.models.clone(),
            enabled: BTreeSet::from(["model-b".into()]),
            disabled: BTreeSet::new(),
            locked: BTreeSet::from(["model-a".into()]),
            default_model: "model-a".into(),
            one_m: BTreeSet::from(["model-a".into(), "model-b".into()]),
            query: String::new(),
            selected: 0,
            search_active: false,
            status: String::new(),
        };
        apply_route_editor(profile, &editor);
        assert_eq!(profile.default_model, "model-a[1m]");
        assert_eq!(profile.aliases.sonnet.as_deref(), Some("model-a[1m]"));
        assert_eq!(profile.enabled_models, ["model-b[1m]"]);
    }

    #[test]
    fn form_field_cursor_and_text_editing() {
        let mut f = field("Test", "hello");
        assert_eq!(f.cursor, 5);
        f.insert_char('!');
        assert_eq!(f.value, "hello!");
        assert_eq!(f.cursor, 6);

        // move left twice
        f.cursor = 4;
        f.delete_backward();
        assert_eq!(f.value, "helo!");
        assert_eq!(f.cursor, 3);

        f.delete_forward();
        assert_eq!(f.value, "hel!");
        assert_eq!(f.cursor, 3);

        f.clear_text();
        assert_eq!(f.value, "");
        assert_eq!(f.cursor, 0);
    }

    #[test]
    fn form_enter_navigates_and_submits() {
        let mut fields = vec![
            field("Name", "test"),
            toggle_field("Active", true),
            field("Target", "url"),
        ];
        let mut selected = 0;

        // Enter on field 0 advances to field 1
        let outcome = handle_form_key(
            &mut fields,
            &mut selected,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert_eq!(outcome, FormOutcome::Stay);
        assert_eq!(selected, 1);

        // Enter on toggle field 1 toggles value
        let outcome = handle_form_key(
            &mut fields,
            &mut selected,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert_eq!(outcome, FormOutcome::Stay);
        assert_eq!(fields[1].value, "false");
        assert_eq!(selected, 1);

        // Tab to field 2
        let outcome = handle_form_key(
            &mut fields,
            &mut selected,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        );
        assert_eq!(outcome, FormOutcome::Stay);
        assert_eq!(selected, 2);

        // Enter on last field returns Submit
        let outcome = handle_form_key(
            &mut fields,
            &mut selected,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert_eq!(outcome, FormOutcome::Submit);
    }

    #[test]
    fn route_editor_batch_enable_and_disable() {
        let catalog = vec![
            ModelEntry {
                id: "default-m".into(),
                label: None,
                description: None,
            },
            ModelEntry {
                id: "m-1".into(),
                label: None,
                description: None,
            },
            ModelEntry {
                id: "m-2".into(),
                label: None,
                description: None,
            },
        ];
        let mut editor = RouteEditor {
            profile_id: "test".into(),
            original_profile: interactive_test_app().config.profiles["one"].clone(),
            provider_enabled: true,
            catalog,
            enabled: BTreeSet::new(),
            disabled: BTreeSet::new(),
            locked: BTreeSet::new(),
            default_model: "default-m".into(),
            one_m: BTreeSet::new(),
            query: String::new(),
            selected: 0,
            search_active: false,
            status: String::new(),
        };

        editor.enable_all_filtered();
        assert!(editor.enabled.contains("m-1"));
        assert!(editor.enabled.contains("m-2"));
        assert!(!editor.enabled.contains("default-m")); // default is required, not duplicated in enabled

        editor.disable_all_filtered();
        assert!(!editor.enabled.contains("m-1"));
        assert!(!editor.enabled.contains("m-2"));
        assert!(editor.is_enabled("default-m")); // default model stays effectively enabled
    }

    #[test]
    fn app_esc_navigation_unfocuses_subpanels() {
        let mut app = interactive_test_app();
        app.view_mode = ViewMode::Provider;
        app.focus = Focus::Details;

        // In Provider view, Esc returns to Home view and Focus::Profiles
        app.view_mode = ViewMode::Home;
        app.focus = Focus::Profiles;
        assert_eq!(app.view_mode, ViewMode::Home);
        assert_eq!(app.focus, Focus::Profiles);
    }

    #[test]
    fn home_screen_enter_and_esc_transitions() {
        let mut app = interactive_test_app();
        assert_eq!(app.view_mode, ViewMode::Home);
        assert_eq!(app.focus, Focus::Profiles);

        // Enter transitions to Provider view
        if app.selected_profile().is_some() {
            app.view_mode = ViewMode::Provider;
            app.focus = Focus::Models;
        }
        assert_eq!(app.view_mode, ViewMode::Provider);
        assert_eq!(app.focus, Focus::Models);

        // Esc transitions back to Home view
        app.view_mode = ViewMode::Home;
        app.focus = Focus::Profiles;
        assert_eq!(app.view_mode, ViewMode::Home);
        assert_eq!(app.focus, Focus::Profiles);
    }

    #[test]
    fn home_screen_mouse_click_drills_down_to_provider() {
        let mut app = interactive_test_app();
        let screen = Rect::new(0, 0, 120, 30);
        let areas = ui_areas(screen, app.focus, app.view_mode);
        let panel = areas.profiles.unwrap();

        // The virtual All Enabled row is first; click the first provider below it.
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: panel.x + 8,
                row: panel.y + 4,
                modifiers: KeyModifiers::NONE,
            },
            screen,
        )
        .unwrap();
        assert_eq!(app.view_mode, ViewMode::Provider);
        assert_eq!(app.focus, Focus::Models);

        // Clicking the header back button returns to Home
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 4,
                row: 1,
                modifiers: KeyModifiers::NONE,
            },
            screen,
        )
        .unwrap();
        assert_eq!(app.view_mode, ViewMode::Home);
        assert_eq!(app.focus, Focus::Profiles);
    }

    #[test]
    fn models_list_keybindings_set_default_and_toggle_1m() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = interactive_test_app();
        app.paths.config = dir.path().join("config.toml");
        std::fs::write(&app.paths.config, toml::to_string(&app.config).unwrap()).unwrap();

        // Model 0 is model-a (default), Model 1 is model-b
        app.view_mode = ViewMode::Provider;
        app.focus = Focus::Models;
        app.model_idx = 1;
        assert_eq!(app.selected_model().unwrap().id, "model-b");

        // Set selected as default
        app.set_selected_as_default();
        assert_eq!(app.config.profiles["one"].default_model, "model-b");

        // Toggle 1M on selected model (model-b)
        app.toggle_selected_model_1m();
        assert_eq!(app.config.profiles["one"].default_model, "model-b[1m]");
        assert!(app.selected_model().unwrap().id.ends_with("[1m]"));

        // Toggle 1M off
        app.toggle_selected_model_1m();
        assert_eq!(app.config.profiles["one"].default_model, "model-b");
        assert_eq!(app.selected_model().unwrap().id, "model-b");
    }

    #[test]
    fn provider_screen_directly_displays_model_catalog_and_showcase() {
        let mut app = interactive_test_app();
        app.enter_provider_view();
        assert_eq!(app.view_mode, ViewMode::Provider);
        assert!(app.provider_editor.is_some());

        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("Selected model"));
        assert!(rendered.contains("Provider"));
        assert!(rendered.contains("One  →  model-a"));
        assert!(rendered.contains("model-b"));
    }

    #[test]
    fn mouse_click_showcase_buttons_perform_actions() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = interactive_test_app();
        app.paths.config = dir.path().join("config.toml");
        std::fs::write(&app.paths.config, toml::to_string(&app.config).unwrap()).unwrap();

        app.enter_provider_view();
        let screen = Rect::new(0, 0, 120, 30);
        let details = ui_areas(screen, app.focus, app.view_mode).details.unwrap();
        let (showcase_card, _) = provider_detail_cards(details);
        let controls = showcase_controls(showcase_card);

        // Click Default button
        let default_btn = controls
            .iter()
            .find(|(c, _)| *c == ShowcaseControl::Default)
            .unwrap()
            .1;
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: default_btn.x + 1,
                row: default_btn.y,
                modifiers: KeyModifiers::NONE,
            },
            screen,
        )
        .unwrap();
        assert_eq!(app.config.profiles["one"].default_model, "model-a");

        // Click 1M button
        let onem_btn = controls
            .iter()
            .find(|(c, _)| *c == ShowcaseControl::OneM)
            .unwrap()
            .1;
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: onem_btn.x + 1,
                row: onem_btn.y,
                modifiers: KeyModifiers::NONE,
            },
            screen,
        )
        .unwrap();
        assert!(app.config.profiles["one"].default_model.ends_with("[1m]"));

        // Toggle is separate from deletion and keeps the catalog entry.
        let toggle_btn = controls
            .iter()
            .find(|(c, _)| *c == ShowcaseControl::Toggle)
            .unwrap()
            .1;
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: toggle_btn.x + 1,
                row: toggle_btn.y,
                modifiers: KeyModifiers::NONE,
            },
            screen,
        )
        .unwrap();
        assert!(!app.provider_editor.as_ref().unwrap().is_enabled("model-a"));
        assert_eq!(app.config.profiles["one"].models.len(), 2);
    }

    #[test]
    fn mouse_click_catalog_add_and_detail_edit() {
        let mut app = interactive_test_app();
        app.enter_provider_view();
        let screen = Rect::new(0, 0, 120, 30);

        // 1. Click catalog add button
        let models = ui_areas(screen, app.focus, app.view_mode).models.unwrap();
        let search_area = Rect::new(models.x, models.y, models.width, 3);
        let add_btn = catalog_add_button_rect(search_area).unwrap();
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: add_btn.x + 1,
                row: add_btn.y,
                modifiers: KeyModifiers::NONE,
            },
            screen,
        )
        .unwrap();
        assert!(matches!(app.modal, Some(Modal::Model(_))));

        // Close modal
        app.modal = None;

        // 2. The details card only keeps provider-specific refresh/edit actions.
        let details = ui_areas(screen, app.focus, app.view_mode).details.unwrap();
        let (_, provider_card) = provider_detail_cards(details);
        assert_eq!(detail_controls(provider_card).len(), 2);
        let edit_btn = detail_controls(provider_card)
            .into_iter()
            .find(|(c, _)| *c == DetailControl::Edit)
            .unwrap()
            .1;
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: edit_btn.x + 1,
                row: edit_btn.y,
                modifiers: KeyModifiers::NONE,
            },
            screen,
        )
        .unwrap();
        assert!(matches!(app.modal, Some(Modal::Profile(_))));
    }

    #[test]
    fn model_can_be_disabled_and_enabled_freely() {
        let mut app = interactive_test_app();
        app.view_mode = ViewMode::Provider;
        app.init_provider_editor();
        let editor = app.provider_editor.as_mut().unwrap();

        // Initially default model "model-a" is enabled
        assert!(editor.is_enabled("model-a"));
        assert_eq!(editor.default_model, "model-a");

        // Select model-a (index 0) and toggle it to disable
        editor.selected = 0;
        editor.toggle_selected();
        assert!(!editor.is_enabled("model-a"));
        // Since model-b was enabled, default model should be switched to model-b
        assert_eq!(editor.default_model, "model-b");

        // Toggle model-a again to re-enable it
        editor.toggle_selected();
        assert!(editor.is_enabled("model-a"));

        // Single model profile toggle test (like flatkey)
        let single_catalog = vec![ModelEntry {
            id: "only-model".into(),
            label: None,
            description: None,
        }];
        let mut single_editor = RouteEditor {
            profile_id: "single".into(),
            original_profile: app.config.profiles["one"].clone(),
            provider_enabled: true,
            catalog: single_catalog,
            enabled: BTreeSet::new(),
            disabled: BTreeSet::new(),
            locked: BTreeSet::new(),
            default_model: "only-model".into(),
            one_m: BTreeSet::new(),
            query: String::new(),
            selected: 0,
            search_active: false,
            status: String::new(),
        };
        assert!(single_editor.is_enabled("only-model"));
        single_editor.toggle_selected();
        assert!(!single_editor.is_enabled("only-model"));
        assert!(single_editor.status.contains("Disabled only-model"));

        single_editor.toggle_selected();
        assert!(single_editor.is_enabled("only-model"));
        assert!(single_editor.status.contains("Enabled only-model"));
    }

    #[test]
    fn disabling_a_model_persists_without_deleting_its_catalog_entry() {
        let mut app = interactive_test_app();
        let dir = tempfile::tempdir().unwrap();
        app.paths.config = dir.path().join("config.toml");
        std::fs::write(&app.paths.config, toml::to_string(&app.config).unwrap()).unwrap();
        app.enter_provider_view();

        let editor = app.provider_editor.as_mut().unwrap();
        editor.selected = 1;
        editor.toggle_selected();
        app.commit_provider_editor().unwrap();

        let profile = &app.config.profiles["one"];
        assert_eq!(profile.models.len(), 2);
        assert!(profile.models.iter().any(|model| model.id == "model-b"));
        assert!(profile.disabled_models.iter().any(|id| id == "model-b"));
        assert!(
            !discovery::active_models(profile, &[])
                .iter()
                .any(|model| model.id == "model-b")
        );

        app.init_provider_editor();
        assert!(!app.provider_editor.as_ref().unwrap().is_enabled("model-b"));
    }

    #[test]
    fn narrow_layout_uses_one_provider_panel_and_wraps_profile_cards() {
        let mut app = interactive_test_app();
        app.config.profiles.get_mut("one").unwrap().base_url =
            "https://gateway.example.com/a/very/long/path/that/must/wrap".into();
        let narrow = Rect::new(0, 0, 44, 22);

        app.enter_provider_view();
        let model_view = ui_areas(narrow, Focus::Models, app.view_mode);
        assert!(model_view.models.is_some());
        assert!(model_view.details.is_none());
        let details_view = ui_areas(narrow, Focus::Details, app.view_mode);
        assert!(details_view.models.is_none());
        assert!(details_view.details.is_some());

        let profile = &app.config.profiles["one"];
        let lines = home_profile_lines("one", profile, 2, 42);
        assert!(lines.len() > 5);
        assert!(lines.iter().all(|line| line.width() <= 42));
    }

    #[test]
    fn model_form_api_model_picker_populates_fields() {
        let api_models = vec![
            ModelEntry {
                id: "qwen-max-latest".into(),
                label: Some("Qwen Max Latest".into()),
                description: Some("Alibaba Cloud flagship model".into()),
            },
            ModelEntry {
                id: "deepseek-v4-flash[1m]".into(),
                label: Some("DeepSeek V4 Flash".into()),
                description: Some("Fast reasoning model".into()),
            },
        ];

        let mut form = ModelForm::with_api_models(api_models);
        assert_eq!(form.filtered_api_models().len(), 2);

        // Filter by keyword "deep"
        form.api_query = "deep".into();
        assert_eq!(form.filtered_api_models().len(), 1);
        assert_eq!(form.filtered_api_models()[0].id, "deepseek-v4-flash[1m]");

        // Pick the filtered model (index 0)
        form.pick_api_model(0);
        assert_eq!(form.fields[0].value, "deepseek-v4-flash");
        assert_eq!(form.fields[1].value, "DeepSeek V4 Flash");
        assert_eq!(form.fields[2].value, "Fast reasoning model");
        assert_eq!(form.fields[3].value, "true");

        // Form to model conversion
        let model = form.to_model();
        assert_eq!(model.id, "deepseek-v4-flash[1m]");
        assert!(model.label.unwrap().contains("1M"));
    }

    #[test]
    fn model_form_search_and_scrolling() {
        let api_models = (0..20)
            .map(|i| ModelEntry {
                id: format!("model-{i:02}"),
                label: Some(format!("Model {i}")),
                description: None,
            })
            .collect::<Vec<_>>();

        let mut form = ModelForm::with_api_models(api_models);
        assert_eq!(form.filtered_api_models().len(), 20);
        assert_eq!(form.api_scroll, 0);
        assert_eq!(form.api_selected, 0);
        assert!(!form.focus_api_search);

        // Scroll down in list
        form.scroll_api_list(true, 5, 10);
        assert_eq!(form.api_scroll, 5);

        // Scroll up in list
        form.scroll_api_list(false, 3, 10);
        assert_eq!(form.api_scroll, 2);

        // Move selection down
        form.move_api_selection(true, 10);
        assert_eq!(form.api_selected, 6);

        // Move selection up
        form.move_api_selection(false, 10);
        assert_eq!(form.api_selected, 5);

        // Search query filtering
        form.api_query = "model-1".into();
        let filtered = form.filtered_api_models();
        assert_eq!(filtered.len(), 10); // model-10 .. model-19
        assert_eq!(filtered[0].id, "model-10");
    }

    #[test]
    fn provider_catalog_only_shows_added_models_not_unselected_gateway_models() {
        let mut app = interactive_test_app();
        // Insert cached discovered models from remote router (e.g. 10 models)
        let discovered = (0..10)
            .map(|i| ModelEntry {
                id: format!("gateway-model-{i}"),
                label: Some(format!("Gateway Model {i}")),
                description: None,
            })
            .collect();
        app.cache.profiles.insert(
            "one".into(),
            CachedModels {
                fetched_at: 1000,
                models: discovered,
            },
        );

        // Profile "one" only has "model-a" and "model-b"
        let catalog = app.catalog_models();
        let ids: Vec<&str> = catalog.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["model-a", "model-b"]);
        assert!(!ids.contains(&"gateway-model-0"));
    }

    fn interactive_test_app() -> App {
        let model = |id: &str| ModelEntry {
            id: id.into(),
            label: None,
            description: None,
        };
        let profile = |name: &str| Profile {
            name: name.into(),
            enabled: true,
            base_url: "https://example.com".into(),
            api_format: ApiFormat::Anthropic,
            credential: Credential::None,
            default_model: "model-a".into(),
            aliases: RoleModels::default(),
            subagent_model: None,
            fallback_models: vec![],
            enabled_models: vec!["model-b".into()],
            disabled_models: vec![],
            models: vec![model("model-a"), model("model-b")],
        };
        let mut config = Config::default();
        config.profiles.insert("one".into(), profile("One"));
        config.profiles.insert("two".into(), profile("Two"));
        App {
            paths: AppPaths {
                config: PathBuf::from("/tmp/ccsw-test-config"),
                state: PathBuf::from("/tmp/ccsw-test-state"),
                cache: PathBuf::from("/tmp/ccsw-test-cache"),
                runtime_dir: PathBuf::from("/tmp/ccsw-test-runtime"),
            },
            config,
            cache: ModelCache::default(),
            cwd: "/tmp".into(),
            view_mode: ViewMode::Home,
            home_all_selected: false,
            profile_idx: 0,
            model_idx: 0,
            profile_offset: 0,
            model_offset: 0,
            focus: Focus::Profiles,
            launch_mode: LaunchMode::New,
            status: "Ready".into(),
            status_error: false,
            modal: None,
            current_session: None,
            proxy_status: None,
            forwarded_args: vec![],
            provider_editor: None,
        }
    }
}
