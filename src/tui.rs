mod app;
mod background;
mod events;
mod forms;
mod help;
mod layout;
mod models;
mod state;
#[cfg(test)]
mod tests;
mod views;

use background::Background;
use forms::*;
use help::*;
use layout::*;
use models::*;
use state::*;

use std::{
    collections::{BTreeMap, BTreeSet},
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
    proxy, sync,
};

const ROUTE: Color = Color::Rgb(95, 215, 215);
const SELECTION: Color = Color::Rgb(34, 58, 66);
const CONNECTED: Color = Color::Rgb(135, 215, 135);
const WARNING: Color = Color::Rgb(255, 215, 95);
const ERROR: Color = Color::Rgb(255, 107, 107);
const MUTED: Color = Color::Rgb(128, 138, 148);

pub struct App {
    paths: AppPaths,
    config: Config,
    cache: ModelCache,
    view_mode: ViewMode,
    home_all_selected: bool,
    profile_idx: usize,
    model_idx: usize,
    profile_offset: usize,
    model_offset: usize,
    focus: Focus,
    status: String,
    status_error: bool,
    modal: Option<Modal>,
    proxy_status: Option<proxy::ProxyStatus>,
    provider_editor: Option<RouteEditor>,
    background: Background,
    screen: Rect,
}

pub fn run(paths: AppPaths, config: Config, import: Option<ImportCandidate>) -> Result<()> {
    let proxy_status = None;
    let mut app = App {
        cache: discovery::load_cache(&paths.cache),
        paths,
        config,
        view_mode: ViewMode::Home,
        home_all_selected: false,
        profile_idx: 0,
        model_idx: 0,
        profile_offset: 0,
        model_offset: 0,
        focus: Focus::Profiles,
        status: "↑↓ Select · Space toggle provider · Enter open".into(),
        status_error: false,
        modal: None,
        proxy_status,
        provider_editor: None,
        background: Background::default(),
        screen: Rect::new(0, 0, 80, 24),
    };
    if app.config.profiles.is_empty()
        && let Some(candidate) = import
    {
        app.modal = Some(Modal::Import(Box::new(candidate)));
    }

    let (mut terminal, _guard) = setup_terminal()?;
    app.event_loop(&mut terminal)
}

type TuiTerminal = Terminal<CrosstermBackend<io::Stdout>>;

struct TerminalGuard;

fn reset_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        DisableMouseCapture,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    );
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        reset_terminal();
    }
}

fn setup_terminal() -> Result<(TuiTerminal, TerminalGuard)> {
    let guard = TerminalGuard;
    enable_raw_mode()?;
    let main_thread = std::thread::current().id();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().id() == main_thread {
            reset_terminal();
        }
        previous(info);
    }));
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    terminal.clear()?;
    Ok((terminal, guard))
}
