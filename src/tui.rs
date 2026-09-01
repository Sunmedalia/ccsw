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
    details: Rect,
    footer: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FooterControl {
    Profiles,
    Models,
    Details,
    Launch,
    Resume,
    New,
    Sync,
    Test,
    Help,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetailControl {
    EnableAll,
    SyncAll,
    Manage,
    Edit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouteControl {
    Enable,
    OneM,
    Default,
    Fetch,
    Add,
    Save,
    Apply,
    Edit,
    Cancel,
}

enum Modal {
    Import(Box<ImportCandidate>),
    Profile(ProfileForm),
    Model(ModelForm),
    Route(RouteEditor),
    DeleteProfile,
    DeleteModel,
    Help,
}

struct ProfileForm {
    original_id: Option<String>,
    original_profile: Option<Profile>,
    models: Vec<ModelEntry>,
    enabled_models: Vec<String>,
    fields: Vec<FormField>,
    selected: usize,
}

struct ModelForm {
    fields: Vec<FormField>,
    selected: usize,
}

struct RouteEditor {
    profile_id: String,
    original_profile: Profile,
    catalog: Vec<ModelEntry>,
    enabled: BTreeSet<String>,
    locked: BTreeSet<String>,
    default_model: String,
    one_m: BTreeSet<String>,
    query: String,
    selected: usize,
    search_active: bool,
    status: String,
}

struct FormField {
    label: &'static str,
    value: String,
    secret: bool,
    toggle: bool,
    choices: &'static [&'static str],
}

pub struct App {
    paths: AppPaths,
    config: Config,
    cache: ModelCache,
    cwd: String,
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
        status: "Select a route, then press Enter to choose a model".into(),
        status_error: false,
        modal: None,
        current_session,
        proxy_status,
        forwarded_args,
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
                    match key.code {
                        KeyCode::Char('q') => return Ok(()),
                        KeyCode::Char('?') => self.modal = Some(Modal::Help),
                        KeyCode::Tab | KeyCode::BackTab => self.toggle_focus(),
                        KeyCode::Left | KeyCode::Char('h') => self.cycle_focus(-1),
                        KeyCode::Right | KeyCode::Char('l') => self.cycle_focus(1),
                        KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
                        KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
                        KeyCode::Char('m') => self.toggle_launch_mode(),
                        KeyCode::Char(' ') if self.focus == Focus::Details => {
                            self.manage_models(false)
                        }
                        KeyCode::Char(' ') => self.toggle_launch_mode(),
                        KeyCode::Char('/') => self.manage_models(true),
                        KeyCode::Char('n') => self.new_profile(),
                        KeyCode::Char('e') => self.manage_models(false),
                        KeyCode::Char('E') => self.edit_profile(),
                        KeyCode::Char('d') if self.selected_profile().is_some() => {
                            self.modal = Some(Modal::DeleteProfile);
                        }
                        KeyCode::Char('a') if self.selected_profile().is_some() => {
                            self.modal = Some(Modal::Model(ModelForm::new()));
                        }
                        KeyCode::Char('x') if self.selected_model().is_some() => {
                            self.modal = Some(Modal::DeleteModel);
                        }
                        KeyCode::Char('r') => self.refresh_models(),
                        KeyCode::Char('t') => self.refresh_models(),
                        KeyCode::Char('A') => self.enable_all_models(),
                        KeyCode::Char('p') => self.sync_all_to_claude(),
                        KeyCode::Char('N') => self.launch_selected(terminal, true)?,
                        KeyCode::Enter if self.focus == Focus::Details => self.manage_models(false),
                        KeyCode::Enter if self.focus == Focus::Profiles => {
                            self.focus = Focus::Models;
                            self.status_error = false;
                            self.status = "Choose an enabled model · Enter launches Claude".into();
                        }
                        KeyCode::Enter => {
                            self.launch_selected(terminal, self.launch_mode == LaunchMode::New)?
                        }
                        _ => {}
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
        self.profile_ids().get(self.profile_idx).cloned()
    }

    fn selected_profile(&self) -> Option<&Profile> {
        self.selected_profile_id()
            .and_then(|id| self.config.profiles.get(&id))
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
        let profile = &self.config.profiles[&id];
        let discovered = self
            .cache
            .profiles
            .get(&id)
            .map(|cached| cached.models.as_slice())
            .unwrap_or_default();
        discovery::merged_models(profile, discovered)
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

    fn selected_model(&self) -> Option<ModelEntry> {
        self.models().get(self.model_idx).cloned()
    }

    fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Profiles => Focus::Models,
            Focus::Models => Focus::Details,
            Focus::Details => Focus::Profiles,
        };
    }

    fn cycle_focus(&mut self, delta: isize) {
        let current = match self.focus {
            Focus::Profiles => 0_isize,
            Focus::Models => 1,
            Focus::Details => 2,
        };
        self.focus = match (current + delta).rem_euclid(3) {
            0 => Focus::Profiles,
            1 => Focus::Models,
            _ => Focus::Details,
        };
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

        let ui = ui_areas(area, self.focus);
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
                    self.move_selection(delta);
                }
            }
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(panel) = ui.profiles
                    && let Some(index) = scrollbar_index(
                        panel,
                        mouse.column,
                        mouse.row,
                        self.config.profiles.len(),
                        usize::from(panel.height.saturating_sub(2)),
                    )
                {
                    self.focus = Focus::Profiles;
                    self.profile_idx = index;
                    self.model_idx = self.default_model_index();
                    self.model_offset = 0;
                    self.sync_session_for_profile();
                    return Ok(MouseAction::None);
                }
                if let Some(panel) = ui.models {
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
                        self.status =
                            format!("Selected {} · Enter or click Launch", models[index].label());
                        return Ok(MouseAction::None);
                    }
                }

                if mouse.kind == MouseEventKind::Drag(MouseButton::Left) {
                    return Ok(MouseAction::None);
                }
                for (control, rect) in footer_controls(ui.footer, area.width < 110) {
                    if !contains(rect, mouse.column, mouse.row) {
                        continue;
                    }
                    return Ok(match control {
                        FooterControl::Profiles => {
                            self.focus = Focus::Profiles;
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
                        FooterControl::Launch => MouseAction::Launch,
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
                        FooterControl::Test => {
                            self.refresh_models();
                            MouseAction::None
                        }
                        FooterControl::Help => {
                            self.modal = Some(Modal::Help);
                            MouseAction::None
                        }
                        FooterControl::Quit => MouseAction::Quit,
                    });
                }

                if let Some(panel) = ui.profiles
                    && let Some(index) =
                        clicked_list_index(panel, mouse.column, mouse.row, self.profile_offset, 1)
                    && index < self.config.profiles.len()
                {
                    self.focus = Focus::Profiles;
                    if self.profile_idx != index {
                        self.profile_idx = index;
                        self.model_idx = self.default_model_index();
                        self.model_offset = 0;
                        self.sync_session_for_profile();
                    }
                } else if let Some(panel) = ui.models
                    && let Some(index) =
                        clicked_list_index(panel, mouse.column, mouse.row, self.model_offset, 2)
                    && index < self.models().len()
                {
                    self.focus = Focus::Models;
                    self.model_idx = index;
                    if let Some(model) = self.selected_model() {
                        self.status_error = false;
                        self.status = format!("Selected {} · click Launch to start", model.label());
                    }
                } else if contains(ui.details, mouse.column, mouse.row) {
                    self.focus = Focus::Details;
                    if let Some((control, _)) = detail_controls(ui.details)
                        .into_iter()
                        .find(|(_, rect)| contains(*rect, mouse.column, mouse.row))
                    {
                        match control {
                            DetailControl::EnableAll => self.enable_all_models(),
                            DetailControl::SyncAll => self.sync_all_to_claude(),
                            DetailControl::Manage => self.manage_models(false),
                            DetailControl::Edit => self.edit_profile(),
                        }
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
                self.handle_modal(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))?;
                return Ok(());
            }
            MouseEventKind::ScrollDown => {
                self.handle_modal(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))?;
                return Ok(());
            }
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) => {}
            _ => return Ok(()),
        }
        if !contains(area, mouse.column, mouse.row) {
            return Ok(());
        }

        if matches!(self.modal, Some(Modal::Route(_))) {
            let list = route_model_list_area(area);
            let filtered_len = match self.modal.as_ref() {
                Some(Modal::Route(editor)) => editor.filtered_indices().len(),
                _ => 0,
            };
            if let Some(index) = scrollbar_index(
                list,
                mouse.column,
                mouse.row,
                filtered_len,
                usize::from(list.height.saturating_sub(2)),
            ) {
                if let Some(Modal::Route(editor)) = self.modal.as_mut() {
                    editor.selected = index;
                    editor.status = format!("Model {} of {filtered_len}", index + 1);
                }
                return Ok(());
            }
            if mouse.kind == MouseEventKind::Drag(MouseButton::Left) {
                return Ok(());
            }
            if let Some((control, _)) = route_controls(area)
                .into_iter()
                .find(|(_, rect)| contains(*rect, mouse.column, mouse.row))
            {
                self.handle_modal(route_control_key(control))?;
                return Ok(());
            }
            if contains(route_edit_area(area), mouse.column, mouse.row) {
                self.handle_modal(KeyEvent::new(KeyCode::Char('E'), KeyModifiers::NONE))?;
                return Ok(());
            }
        } else if mouse.kind == MouseEventKind::Drag(MouseButton::Left) {
            return Ok(());
        }

        let button_count = match self.modal.as_ref() {
            Some(Modal::Help) => 1,
            Some(Modal::Route(_)) => 0,
            Some(_) => 2,
            None => 0,
        };
        if let Some(button) = modal_button_rects(area, button_count)
            .iter()
            .position(|rect| contains(*rect, mouse.column, mouse.row))
        {
            let key = match (self.modal.as_ref(), button) {
                (Some(Modal::Import(_)), 0)
                | (Some(Modal::DeleteProfile | Modal::DeleteModel), 0) => {
                    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
                }
                (Some(Modal::Profile(_) | Modal::Model(_)), 0) => {
                    KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
                }
                (Some(Modal::Help), 0) => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
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
            Some(Modal::Route(editor)) => {
                if contains(route_search_area(area), mouse.column, mouse.row) {
                    editor.search_active = true;
                    return Ok(());
                }
                let list = route_model_list_area(area);
                let list_inner = panel_inner(list);
                if contains(list_inner, mouse.column, mouse.row) {
                    let offset = route_editor_offset(editor, list.height.saturating_sub(2));
                    let index = offset + usize::from(mouse.row.saturating_sub(list_inner.y));
                    if index < editor.filtered_indices().len() {
                        editor.selected = index;
                        if mouse.column < list_inner.x.saturating_add(4) {
                            editor.toggle_selected();
                        }
                    }
                }
            }
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
                if let Some(index) = clicked_field
                    && index < form.fields.len()
                {
                    form.selected = index;
                    if form.fields[index].toggle {
                        toggle_form_field(&mut form.fields[index]);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn move_selection(&mut self, delta: isize) {
        let len = match self.focus {
            Focus::Profiles => self.config.profiles.len(),
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
                self.status = format!("Selected {name} · Enter to choose a model");
            }
        }
        if self.focus == Focus::Models
            && let Some(model) = self.selected_model()
        {
            self.status_error = false;
            self.status = format!("Selected {} · Enter or click Launch", model.label());
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
        self.modal = Some(Modal::Profile(ProfileForm::new()));
    }

    fn manage_models(&mut self, search_active: bool) {
        let Some(profile_id) = self.selected_profile_id() else {
            self.set_error("Create a route before managing models");
            return;
        };
        let profile = &self.config.profiles[&profile_id];
        let references = profile
            .required_model_ids()
            .into_iter()
            .chain(profile.enabled_models.iter().cloned())
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
        let editor = RouteEditor {
            profile_id,
            original_profile: profile.clone(),
            catalog: normalize_model_catalog(self.catalog_models()),
            enabled: profile
                .enabled_models
                .iter()
                .map(|id| canonical_model_id(id))
                .collect(),
            locked,
            default_model,
            one_m,
            query: String::new(),
            selected: 0,
            search_active,
            status: "Enter enables · 1 toggles 1M · d sets default · r refreshes".into(),
        };
        self.modal = Some(Modal::Route(editor));
    }

    fn edit_profile(&mut self) {
        let Some(id) = self.selected_profile_id() else {
            return;
        };
        let profile = self.config.profiles[&id].clone();
        self.modal = Some(Modal::Profile(ProfileForm::edit(id, &profile)));
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
        let Some(default_profile_id) = self.selected_profile_id() else {
            self.set_error("Create a profile before syncing Claude");
            return;
        };
        match self.apply_all_to_claude(&default_profile_id) {
            Ok(result) => {
                self.proxy_status = proxy::status(&self.paths).ok();
                self.status_error = false;
                self.status = format!(
                    "Synced {} models from {} profiles to Claude /model",
                    result.model_count,
                    self.config.profiles.len()
                );
            }
            Err(error) => self.set_error(format!("Could not sync Claude: {error:#}")),
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

    fn commit_route_editor(&mut self, editor: &RouteEditor) -> Result<()> {
        let mut profile = self
            .config
            .profiles
            .get(&editor.profile_id)
            .cloned()
            .expect("route editor profile exists");
        apply_route_editor(&mut profile, editor);
        let id = editor.profile_id.clone();
        let original = editor.original_profile.clone();
        self.config = config::update(&self.paths.config, |latest| {
            if latest.profiles.get(&id) != Some(&original) {
                anyhow::bail!(
                    "profile '{id}' changed in another CCSW instance; reopen model management"
                );
            }
            latest.profiles.insert(id, profile);
            Ok(())
        })?;
        self.model_idx = self.default_model_index();
        self.model_offset = 0;
        Ok(())
    }

    fn apply_all_to_claude(&self, default_profile_id: &str) -> Result<claude_config::ApplyResult> {
        claude_config::apply_all(
            &claude_config::settings_path()?,
            &self.paths,
            &self.config,
            &self.cache,
            default_profile_id,
        )
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
            Modal::Help => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Enter) {
                    return Ok(());
                }
            }
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
                KeyCode::Char('n') | KeyCode::Esc => return Ok(()),
                _ => {}
            },
            Modal::DeleteModel => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Enter => {
                        if let (Some(profile_id), Some(model)) =
                            (self.selected_profile_id(), self.selected_model())
                        {
                            let is_manual = self.config.profiles[&profile_id]
                                .models
                                .iter()
                                .any(|entry| entry.id == model.id);
                            if !is_manual {
                                self.set_error("Discovered models cannot be deleted; refresh or edit the gateway");
                            } else {
                                let model_id = model.id.clone();
                                self.config = config::update(&self.paths.config, |latest| {
                                    let profile = latest
                                        .profiles
                                        .get_mut(&profile_id)
                                        .context("profile was removed in another CCSW instance")?;
                                    profile.models.retain(|entry| entry.id != model_id);
                                    profile.enabled_models.retain(|id| id != &model_id);
                                    Ok(())
                                })?;
                                self.model_idx =
                                    self.model_idx.min(self.models().len().saturating_sub(1));
                                self.status = format!("Deleted manual model {}", model.id);
                            }
                        }
                        return Ok(());
                    }
                    KeyCode::Char('n') | KeyCode::Esc => return Ok(()),
                    _ => {}
                }
            }
            Modal::Route(editor) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
                    self.commit_route_editor(editor)?;
                    self.focus = Focus::Models;
                    self.status_error = false;
                    self.status = format!(
                        "Saved {} enabled models for {}",
                        self.models().len(),
                        editor.profile_id
                    );
                    return Ok(());
                }
                if editor.search_active {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter => editor.search_active = false,
                        KeyCode::Backspace => {
                            editor.query.pop();
                            editor.selected = 0;
                        }
                        KeyCode::Up | KeyCode::Down => {
                            editor.move_selection(matches!(key.code, KeyCode::Down))
                        }
                        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            editor.query.push(ch);
                            editor.selected = 0;
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Esc => return Ok(()),
                        KeyCode::Char('/') => editor.search_active = true,
                        KeyCode::Up | KeyCode::Char('k') => editor.move_selection(false),
                        KeyCode::Down | KeyCode::Char('j') => editor.move_selection(true),
                        KeyCode::Enter | KeyCode::Char(' ') => editor.toggle_selected(),
                        KeyCode::Char('1') => editor.toggle_selected_1m(),
                        KeyCode::Char('d') => editor.set_selected_default(),
                        KeyCode::Char('r') => {
                            self.commit_route_editor(editor)?;
                            self.refresh_models();
                            self.manage_models(false);
                            return Ok(());
                        }
                        KeyCode::Char('a') => {
                            self.commit_route_editor(editor)?;
                            self.modal = Some(Modal::Model(ModelForm::new()));
                            return Ok(());
                        }
                        KeyCode::Char('e') | KeyCode::Char('E') => {
                            self.commit_route_editor(editor)?;
                            self.edit_profile();
                            return Ok(());
                        }
                        KeyCode::Char('p') => {
                            self.commit_route_editor(editor)?;
                            let result = self.apply_all_to_claude(&editor.profile_id)?;
                            self.proxy_status = proxy::status(&self.paths).ok();
                            self.focus = Focus::Models;
                            self.status_error = false;
                            self.status = format!(
                                "Synced {} models from all profiles to {}",
                                result.model_count,
                                result.path.display()
                            );
                            return Ok(());
                        }
                        _ => {}
                    }
                }
            }
            Modal::Profile(form) => {
                if handle_form_key(&mut form.fields, &mut form.selected, key) {
                    return Ok(());
                }
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
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
                            return Ok(());
                        }
                        Err(error) => self.set_error(format!("Cannot save profile: {error:#}")),
                    }
                }
            }
            Modal::Model(form) => {
                if handle_form_key(&mut form.fields, &mut form.selected, key) {
                    return Ok(());
                }
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
                    let base_id = canonical_model_id(form.fields[0].value.trim());
                    if base_id.trim().is_empty() {
                        self.set_error("Model id cannot be empty");
                    } else if let Some(profile_id) = self.selected_profile_id() {
                        let model = form.to_model();
                        let saved_model = model.clone();
                        self.config = config::update(&self.paths.config, |latest| {
                            let profile = latest
                                .profiles
                                .get_mut(&profile_id)
                                .context("profile was removed in another CCSW instance")?;
                            profile.models.retain(|entry| entry.id != saved_model.id);
                            profile.models.push(saved_model.clone());
                            if !profile.required_model_ids().contains(&saved_model.id)
                                && !profile.enabled_models.contains(&saved_model.id)
                            {
                                profile.enabled_models.push(saved_model.id.clone());
                            }
                            Ok(())
                        })?;
                        self.select_model_id(&model.id);
                        self.status_error = false;
                        self.status = format!("Saved model {}", model.id);
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
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(3),
            ])
            .split(area);
        self.draw_route(frame, rows[0]);
        let ui = ui_areas(area, self.focus);
        if let Some(profiles) = ui.profiles {
            self.draw_profiles(frame, profiles);
        }
        if let Some(models) = ui.models {
            self.draw_models(frame, models);
        }
        self.draw_details(frame, ui.details, self.focus == Focus::Details);
        self.draw_status(frame, ui.footer, area.width < 110);
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
        let line = Line::from(vec![
            Span::styled(
                " CCSW ",
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
        ]);
        frame.render_widget(
            Paragraph::new(line).block(Block::default().borders(Borders::BOTTOM)),
            area,
        );
    }

    fn draw_profiles(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let ids = self.profile_ids();
        let items: Vec<_> = ids
            .iter()
            .map(|id| {
                let profile = &self.config.profiles[id];
                ListItem::new(Line::from(vec![
                    Span::styled("● ", Style::default().fg(CONNECTED)),
                    Span::raw(profile.name.clone()),
                    Span::styled(format!("  {id}"), Style::default().fg(MUTED)),
                ]))
            })
            .collect();
        let title = if self.focus == Focus::Profiles {
            " Profiles · n/e/d "
        } else {
            " Profiles "
        };
        let mut state = ListState::default()
            .with_offset(self.profile_offset)
            .with_selected((!items.is_empty()).then_some(self.profile_idx));
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
        draw_scrollbar(
            frame,
            area,
            ids.len(),
            self.profile_idx,
            usize::from(area.height.saturating_sub(2)),
        );
        if ids.is_empty() {
            frame.render_widget(
                Paragraph::new("No routes yet.\nPress n to create one.")
                    .style(Style::default().fg(MUTED))
                    .block(panel(title, self.focus == Focus::Profiles)),
                area,
            );
        }
    }

    fn draw_models(&mut self, frame: &mut ratatui::Frame, area: Rect) {
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
        let title = if self.focus == Focus::Models {
            " Enabled models · / manage · a/x manual "
        } else {
            " Enabled models "
        };
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

    fn draw_details(&self, frame: &mut ratatui::Frame, area: Rect, active: bool) {
        let title = if active {
            " Route details · A enable all · p sync Claude "
        } else {
            " Route details "
        };
        frame.render_widget(panel(title, active), area);
        let inner = panel_inner(area);
        let content = Rect::new(
            inner.x,
            inner.y,
            inner.width,
            inner.height.saturating_sub(3),
        );
        let Some(profile) = self.selected_profile() else {
            frame.render_widget(
                Paragraph::new("Create a route to connect Claude Code to a gateway.")
                    .style(Style::default().fg(MUTED))
                    .wrap(Wrap { trim: true }),
                content,
            );
            return;
        };
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

    fn draw_status(&self, frame: &mut ratatui::Frame, area: Rect, compact: bool) {
        for (control, rect) in footer_controls(area, compact) {
            let (label, style) = self.footer_control_style(control, compact);
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
            Span::styled(
                "   click or scroll · m toggles session mode",
                Style::default().fg(MUTED),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(footer),
            Rect {
                y: area.y.saturating_add(2),
                height: 1,
                ..area
            },
        );
    }

    fn footer_control_style(&self, control: FooterControl, compact: bool) -> (String, Style) {
        let selected = match control {
            FooterControl::Profiles => self.focus == Focus::Profiles,
            FooterControl::Models => self.focus == Focus::Models,
            FooterControl::Details => self.focus == Focus::Details,
            FooterControl::Resume => self.launch_mode == LaunchMode::Resume,
            FooterControl::New => self.launch_mode == LaunchMode::New,
            _ => false,
        };
        let dot = if selected { '●' } else { '○' };
        let label = match (control, compact) {
            (FooterControl::Profiles, _) => format!("{dot} Routes"),
            (FooterControl::Models, _) => format!("{dot} Models"),
            (FooterControl::Details, _) => format!("{dot} Details"),
            (FooterControl::Launch, true) => "▶ Run".into(),
            (FooterControl::Launch, false) => "▶ Launch".into(),
            (FooterControl::Resume, true) => format!("R{dot}"),
            (FooterControl::Resume, false) => format!("{dot} Resume"),
            (FooterControl::New, true) => format!("N{dot}"),
            (FooterControl::New, false) => format!("{dot} New"),
            (FooterControl::Sync, true) => "⇄ Sync".into(),
            (FooterControl::Sync, false) => "⇄ Sync all".into(),
            (FooterControl::Test, _) => "Test".into(),
            (FooterControl::Help, true) => "?".into(),
            (FooterControl::Help, false) => "Help".into(),
            (FooterControl::Quit, true) => "×".into(),
            (FooterControl::Quit, false) => "Quit".into(),
        };
        let style = if matches!(control, FooterControl::Launch | FooterControl::Sync) {
            Style::default()
                .fg(Color::Black)
                .bg(ROUTE)
                .add_modifier(Modifier::BOLD)
        } else if selected {
            Style::default()
                .fg(Color::Black)
                .bg(CONNECTED)
                .add_modifier(Modifier::BOLD)
        } else if control == FooterControl::Resume && self.current_session.is_none() {
            Style::default().fg(MUTED).add_modifier(Modifier::DIM)
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
                draw_form(
                    frame,
                    area,
                    " Manual model · Space toggles 1M ",
                    &form.fields,
                    form.selected,
                );
                draw_modal_buttons(frame, area, &["Save", "Cancel"]);
            }
            Modal::Route(editor) => {
                let profile = &self.config.profiles[&editor.profile_id];
                draw_route_editor(frame, area, profile, editor);
            }
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
            Modal::Help => {
                let text = vec![
                    Line::styled(
                        "Route",
                        Style::default().fg(ROUTE).add_modifier(Modifier::BOLD),
                    ),
                    Line::raw("Enter  route → models → launch; details → manage"),
                    Line::raw("N      start a separate Claude session"),
                    Line::raw("/model shows enabled models from every provider"),
                    Line::raw(""),
                    Line::raw("n/e/d  create, manage, delete route"),
                    Line::raw("E      edit API format, endpoint and credential"),
                    Line::raw("r/t    fetch provider models / test connection"),
                    Line::raw("a/x    add / delete a manual model"),
                    Line::raw("/      search models; Enter/Space enables"),
                    Line::raw("1/d    toggle [1m] context / set default"),
                    Line::raw("A      enable every model in the selected profile"),
                    Line::raw("p      sync every profile to direct Claude /model"),
                    Line::raw("Tab    switch routes/models/details panel"),
                    Line::raw("m      toggle Resume/New launch mode"),
                    Line::raw("Mouse  click controls; drag scrollbars to navigate"),
                ];
                frame.render_widget(
                    Paragraph::new(text)
                        .wrap(Wrap { trim: false })
                        .block(panel(" Help ", true)),
                    area,
                );
                draw_modal_buttons(frame, area, &["Close"]);
            }
        }
    }
}

impl ProfileForm {
    fn new() -> Self {
        Self::from_values(
            None,
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
        form.original_profile = Some(profile.clone());
        form
    }

    #[allow(clippy::too_many_arguments)]
    fn from_values(
        original_id: Option<String>,
        models: Vec<ModelEntry>,
        enabled_models: Vec<String>,
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
            models,
            enabled_models,
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
            models: self.models.clone(),
        };
        profile.validate()?;
        Ok((id, profile))
    }
}

impl ModelForm {
    fn new() -> Self {
        Self {
            fields: vec![
                field("Model ID", ""),
                field("Label", ""),
                field("Description", ""),
                toggle_field("1M context", false),
            ],
            selected: 0,
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
        id == self.default_model || self.locked.contains(id)
    }

    fn is_enabled(&self, id: &str) -> bool {
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

    fn move_selection(&mut self, down: bool) {
        let len = self.filtered_indices().len();
        if len == 0 {
            self.selected = 0;
        } else if down {
            self.selected = (self.selected + 1) % len;
        } else {
            self.selected = self.selected.checked_sub(1).unwrap_or(len - 1);
        }
    }

    fn toggle_selected(&mut self) {
        let Some(id) = self.selected_model().map(|model| model.id.clone()) else {
            self.status = "No model matches this search".into();
            return;
        };
        if self.is_required(&id) {
            self.status = format!("{id} is required by the current route and stays enabled");
        } else if self.enabled.remove(&id) {
            self.status = format!("Disabled {id}");
        } else {
            self.enabled.insert(id.clone());
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
        self.enabled.remove(&id);
        self.default_model = id.clone();
        self.status = format!("Default model set to {id}");
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
    profile.default_model = editor.effective_id(&editor.default_model);
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
        *model = editor.effective_id(model);
    }
    profile.fallback_models = profile
        .fallback_models
        .iter()
        .map(|id| editor.effective_id(id))
        .collect();
    profile.enabled_models = editor
        .enabled
        .iter()
        .filter(|id| !editor.is_required(id))
        .map(|id| editor.effective_id(id))
        .collect();
}

fn handle_form_key(fields: &mut [FormField], selected: &mut usize, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => return true,
        KeyCode::Tab | KeyCode::Down => *selected = (*selected + 1) % fields.len(),
        KeyCode::BackTab | KeyCode::Up => {
            *selected = selected.checked_sub(1).unwrap_or(fields.len() - 1)
        }
        KeyCode::Enter | KeyCode::Char(' ') if fields[*selected].toggle => {
            toggle_form_field(&mut fields[*selected]);
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char(' ')
            if !fields[*selected].choices.is_empty() =>
        {
            cycle_choice(&mut fields[*selected], true);
        }
        KeyCode::Left if !fields[*selected].choices.is_empty() => {
            cycle_choice(&mut fields[*selected], false);
        }
        KeyCode::Backspace if !fields[*selected].toggle && fields[*selected].choices.is_empty() => {
            fields[*selected].value.pop();
        }
        KeyCode::Char(ch)
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !fields[*selected].toggle
                && fields[*selected].choices.is_empty() =>
        {
            fields[*selected].value.push(ch)
        }
        _ => {}
    }
    false
}

fn field(label: &'static str, value: &str) -> FormField {
    FormField {
        label,
        value: value.into(),
        secret: false,
        toggle: false,
        choices: &[],
    }
}

fn secret_field(label: &'static str, value: &str) -> FormField {
    FormField {
        label,
        value: value.into(),
        secret: true,
        toggle: false,
        choices: &[],
    }
}

fn toggle_field(label: &'static str, enabled: bool) -> FormField {
    FormField {
        label,
        value: enabled.to_string(),
        secret: false,
        toggle: true,
        choices: &[],
    }
}

fn choice_field(label: &'static str, value: &str, choices: &'static [&'static str]) -> FormField {
    FormField {
        label,
        value: value.into(),
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
}

fn toggle_form_field(field: &mut FormField) {
    field.value = (field.value != "true").to_string();
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

fn draw_route_editor(
    frame: &mut ratatui::Frame,
    area: Rect,
    profile: &Profile,
    editor: &RouteEditor,
) {
    frame.render_widget(Clear, area);
    frame.render_widget(panel(" Manage route · models ", true), area);
    let inner = panel_inner(area);
    let filtered = editor.filtered_indices();
    let enabled_count = editor
        .catalog
        .iter()
        .filter(|model| editor.is_enabled(&model.id))
        .count();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                profile.name.clone(),
                Style::default().fg(ROUTE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {enabled_count}/{} enabled", editor.catalog.len()),
                Style::default().fg(CONNECTED),
            ),
        ])),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Provider  ", Style::default().fg(MUTED)),
            Span::styled(
                format!("{}  ", profile.api_format.label()),
                Style::default().fg(ROUTE),
            ),
            Span::raw(profile.base_url.clone()),
        ])),
        Rect::new(inner.x, inner.y + 1, inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Credential  ", Style::default().fg(MUTED)),
            Span::raw(profile.credential.masked()),
            Span::styled("  fixed · click to edit", Style::default().fg(WARNING)),
        ])),
        Rect::new(inner.x, inner.y + 2, inner.width, 1),
    );

    let search = if editor.query.is_empty() {
        "type / to search models".to_owned()
    } else {
        editor.query.clone()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Search  ", Style::default().fg(MUTED)),
            Span::styled(
                format!("{search}{}", if editor.search_active { "▌" } else { "" }),
                if editor.search_active {
                    Style::default().fg(Color::Black).bg(ROUTE)
                } else {
                    Style::default().fg(ROUTE)
                },
            ),
            Span::styled(
                format!("  {} matches", filtered.len()),
                Style::default().fg(MUTED),
            ),
        ])),
        route_search_area(area),
    );

    let items = filtered
        .iter()
        .map(|index| {
            let model = &editor.catalog[*index];
            let marker = if editor.is_required(&model.id) {
                "◆"
            } else if editor.enabled.contains(&model.id) {
                "●"
            } else {
                "○"
            };
            let marker_color = if editor.is_required(&model.id) {
                WARNING
            } else if editor.enabled.contains(&model.id) {
                CONNECTED
            } else {
                MUTED
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{marker}  "), Style::default().fg(marker_color)),
                Span::raw(model.label().to_owned()),
                Span::styled(format!("  {}", model.id), Style::default().fg(MUTED)),
                Span::styled(
                    format!(
                        "  1M {}",
                        if editor.one_m.contains(&model.id) {
                            "●"
                        } else {
                            "○"
                        }
                    ),
                    Style::default().fg(if editor.one_m.contains(&model.id) {
                        CONNECTED
                    } else {
                        MUTED
                    }),
                ),
                if model.id == editor.default_model {
                    Span::styled("  default", Style::default().fg(WARNING))
                } else {
                    Span::raw("")
                },
            ]))
        })
        .collect::<Vec<_>>();
    let list_area = route_model_list_area(area);
    let offset = route_editor_offset(editor, list_area.height.saturating_sub(2));
    let mut state = ListState::default()
        .with_offset(offset)
        .with_selected((!items.is_empty()).then_some(editor.selected));
    frame.render_stateful_widget(
        List::new(items)
            .block(panel(" Models · ◆ required  ● enabled  ○ disabled ", true))
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(ROUTE)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(" "),
        list_area,
        &mut state,
    );
    draw_scrollbar(
        frame,
        list_area,
        filtered.len(),
        editor.selected,
        usize::from(list_area.height.saturating_sub(2)),
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(editor.status.clone()),
            Span::styled(
                "  · a add manual · p sync all to Claude",
                Style::default().fg(ROUTE),
            ),
        ]))
        .style(Style::default().fg(MUTED)),
        Rect::new(
            inner.x,
            area.y + area.height.saturating_sub(5),
            inner.width,
            1,
        ),
    );
    draw_route_controls(frame, area);
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

fn ui_areas(area: Rect, focus: Focus) -> UiAreas {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(area);
    if area.width >= 110 {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(22),
                Constraint::Percentage(46),
                Constraint::Percentage(32),
            ])
            .split(rows[1]);
        UiAreas {
            profiles: Some(cols[0]),
            models: Some(cols[1]),
            details: cols[2],
            footer: rows[2],
        }
    } else {
        let inner = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(56), Constraint::Percentage(44)])
            .split(rows[1]);
        UiAreas {
            profiles: (focus == Focus::Profiles).then_some(inner[0]),
            models: (focus != Focus::Profiles).then_some(inner[0]),
            details: inner[1],
            footer: rows[2],
        }
    }
}

fn footer_controls(area: Rect, compact: bool) -> Vec<(FooterControl, Rect)> {
    const WIDE: &[(FooterControl, u16)] = &[
        (FooterControl::Launch, 11),
        (FooterControl::Sync, 14),
        (FooterControl::Resume, 11),
        (FooterControl::New, 8),
        (FooterControl::Test, 8),
        (FooterControl::Help, 8),
        (FooterControl::Quit, 8),
    ];
    const COMPACT: &[(FooterControl, u16)] = &[
        (FooterControl::Profiles, 8),
        (FooterControl::Models, 8),
        (FooterControl::Details, 8),
        (FooterControl::Launch, 7),
        (FooterControl::Sync, 8),
        (FooterControl::Help, 5),
        (FooterControl::Quit, 5),
    ];
    let specs = if compact { COMPACT } else { WIDE };
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

fn detail_controls(area: Rect) -> Vec<(DetailControl, Rect)> {
    let inner = panel_inner(area);
    let first_y = inner.y.saturating_add(inner.height.saturating_sub(2));
    let second_y = inner.y.saturating_add(inner.height.saturating_sub(1));
    let first_width = 14_u16.min(inner.width);
    let first_rest = inner.width.saturating_sub(first_width.saturating_add(1));
    let second_width = 22_u16.min(inner.width);
    let second_rest = inner.width.saturating_sub(second_width.saturating_add(1));
    let mut controls = vec![
        (
            DetailControl::EnableAll,
            Rect::new(inner.x, first_y, first_width, 1),
        ),
        (
            DetailControl::SyncAll,
            Rect::new(inner.x, second_y, second_width, 1),
        ),
    ];
    if first_rest > 0 {
        controls.push((
            DetailControl::Manage,
            Rect::new(
                inner.x.saturating_add(first_width).saturating_add(1),
                first_y,
                first_rest,
                1,
            ),
        ));
    }
    if second_rest > 0 {
        controls.push((
            DetailControl::Edit,
            Rect::new(
                inner.x.saturating_add(second_width).saturating_add(1),
                second_y,
                second_rest,
                1,
            ),
        ));
    }
    controls
}

fn draw_detail_controls(frame: &mut ratatui::Frame, area: Rect) {
    for (control, rect) in detail_controls(area) {
        let (label, style) = match control {
            DetailControl::EnableAll => (
                "[Enable all]",
                Style::default().fg(CONNECTED).add_modifier(Modifier::BOLD),
            ),
            DetailControl::SyncAll => (
                "[Sync all → Claude]",
                Style::default()
                    .fg(Color::Black)
                    .bg(ROUTE)
                    .add_modifier(Modifier::BOLD),
            ),
            DetailControl::Manage => ("[Manage models]", Style::default().fg(ROUTE)),
            DetailControl::Edit => ("[Edit route]", Style::default().fg(WARNING)),
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
    if matches!(modal, Modal::Route(_)) {
        centered_rect(
            96.min(screen.width.saturating_sub(2)),
            30.min(screen.height.saturating_sub(2)),
            screen,
        )
    } else {
        modal_area(screen)
    }
}

fn route_search_area(area: Rect) -> Rect {
    let inner = panel_inner(area);
    Rect::new(inner.x, inner.y.saturating_add(4), inner.width, 1)
}

fn route_model_list_area(area: Rect) -> Rect {
    let inner = panel_inner(area);
    let y = inner.y.saturating_add(6);
    let bottom = area.y.saturating_add(area.height).saturating_sub(6);
    Rect::new(inner.x, y, inner.width, bottom.saturating_sub(y))
}

fn route_edit_area(area: Rect) -> Rect {
    let inner = panel_inner(area);
    Rect::new(inner.x, inner.y.saturating_add(1), inner.width, 2)
}

fn route_editor_offset(editor: &RouteEditor, viewport_height: u16) -> usize {
    let visible = usize::from(viewport_height.max(1));
    editor.selected.saturating_add(1).saturating_sub(visible)
}

fn route_controls(area: Rect) -> Vec<(RouteControl, Rect)> {
    let bottom = area.y.saturating_add(area.height);
    let first = [
        (RouteControl::Enable, "Enable"),
        (RouteControl::OneM, "1M"),
        (RouteControl::Default, "Default"),
        (RouteControl::Fetch, "Fetch"),
        (RouteControl::Add, "Add"),
    ];
    let second = [
        (RouteControl::Save, "Save"),
        (RouteControl::Apply, "Sync all"),
        (RouteControl::Edit, "Edit route"),
        (RouteControl::Cancel, "Cancel"),
    ];
    button_row_rects(area, bottom.saturating_sub(3), &first)
        .into_iter()
        .chain(button_row_rects(area, bottom.saturating_sub(2), &second))
        .collect()
}

fn button_row_rects(
    area: Rect,
    y: u16,
    buttons: &[(RouteControl, &str)],
) -> Vec<(RouteControl, Rect)> {
    let gap = 2_u16;
    let widths = buttons
        .iter()
        .map(|(_, label)| {
            u16::try_from(label.len())
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
            let rect = Rect::new(x, y, width, 1);
            x = x.saturating_add(width).saturating_add(gap);
            (*control, rect)
        })
        .collect()
}

fn route_control_key(control: RouteControl) -> KeyEvent {
    match control {
        RouteControl::Enable => KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        RouteControl::OneM => KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE),
        RouteControl::Default => KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
        RouteControl::Fetch => KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
        RouteControl::Add => KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        RouteControl::Save => KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
        RouteControl::Apply => KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
        RouteControl::Edit => KeyEvent::new(KeyCode::Char('E'), KeyModifiers::NONE),
        RouteControl::Cancel => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    }
}

fn draw_route_controls(frame: &mut ratatui::Frame, area: Rect) {
    for (control, rect) in route_controls(area) {
        let label = match control {
            RouteControl::Enable => "Enable",
            RouteControl::OneM => "1M",
            RouteControl::Default => "Default",
            RouteControl::Fetch => "Fetch",
            RouteControl::Add => "Add",
            RouteControl::Save => "Save",
            RouteControl::Apply => "Sync all",
            RouteControl::Edit => "Edit route",
            RouteControl::Cancel => "Cancel",
        };
        let style = match control {
            RouteControl::Enable => Style::default()
                .fg(Color::Black)
                .bg(ROUTE)
                .add_modifier(Modifier::BOLD),
            RouteControl::Apply => Style::default()
                .fg(Color::Black)
                .bg(CONNECTED)
                .add_modifier(Modifier::BOLD),
            RouteControl::Edit => Style::default().fg(WARNING),
            _ => Style::default().fg(MUTED),
        };
        frame.render_widget(
            Paragraph::new(format!("[{label}]"))
                .alignment(Alignment::Center)
                .style(style),
            rect,
        );
    }
}

fn modal_button_rects(area: Rect, count: usize) -> Vec<Rect> {
    if count == 0 {
        return vec![];
    }
    let count = u16::try_from(count).unwrap_or(u16::MAX);
    let gap = if count >= 5 { 1_u16 } else { 2_u16 };
    let available = area.width.saturating_sub(4);
    let width = 12_u16
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
        assert!(rendered.contains("No routes yet"));
        assert!(rendered.contains("no profile"));
        assert!(rendered.contains("Routes"));
        assert!(rendered.contains("Run"));
    }

    #[test]
    fn mouse_wheel_and_click_navigate_lists() {
        let mut app = interactive_test_app();
        let screen = Rect::new(0, 0, 120, 30);
        let areas = ui_areas(screen, app.focus);
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

        app.focus = Focus::Models;
        let models = ui_areas(screen, app.focus).models.unwrap();
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: models.x + 1,
                row: models.y + 3,
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
        let screen = Rect::new(0, 0, 120, 30);
        let footer = ui_areas(screen, app.focus).footer;
        let new_button = footer_controls(footer, false)
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
        let panel = ui_areas(screen, app.focus).profiles.unwrap();
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
        let screen = Rect::new(0, 0, 120, 30);
        let details = ui_areas(screen, app.focus).details;
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
    fn homepage_exposes_enable_all_and_sync_controls() {
        let mut app = interactive_test_app();
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
        assert!(rendered.contains("Enable all"));
        assert!(rendered.contains("Sync all → Claude"));
        assert!(rendered.contains("Claude /model"));
        assert!(
            footer_controls(Rect::new(0, 27, 120, 3), false)
                .iter()
                .any(|(control, _)| *control == FooterControl::Sync)
        );
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
            catalog: models,
            enabled: BTreeSet::new(),
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
            catalog: profile.models.clone(),
            enabled: BTreeSet::from(["model-b".into()]),
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

    fn interactive_test_app() -> App {
        let model = |id: &str| ModelEntry {
            id: id.into(),
            label: None,
            description: None,
        };
        let profile = |name: &str| Profile {
            name: name.into(),
            base_url: "https://example.com".into(),
            api_format: ApiFormat::Anthropic,
            credential: Credential::None,
            default_model: "model-a".into(),
            aliases: RoleModels::default(),
            subagent_model: None,
            fallback_models: vec![],
            enabled_models: vec!["model-b".into()],
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
        }
    }
}
