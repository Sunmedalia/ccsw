use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum ViewMode {
    #[default]
    Home,
    Provider,
    AllEnabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Focus {
    Profiles,
    Models,
    Details,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MouseAction {
    None,
    Quit,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct UiAreas {
    pub(super) profiles: Option<Rect>,
    pub(super) models: Option<Rect>,
    pub(super) details: Option<Rect>,
    pub(super) footer: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FooterControl {
    Back,
    Models,
    Details,
    AddProfile,
    Sync,
    Proxy,
    Help,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DetailControl {
    FetchModels,
    Edit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ShowcaseControl {
    Toggle,
    Default,
    OneM,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProxyControl {
    Port,
    Start,
    Stop,
    Refresh,
    EnableAtLogin,
    DisableAtLogin,
    Close,
}

#[derive(Clone)]
pub(super) enum Modal {
    Import(Box<ImportCandidate>),
    Profile(Box<ProfileForm>),
    Model(ModelForm),
    DeleteProfile,
    DeleteModel,
    Proxy(ProxyManager),
    Help(HelpModal),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HelpSection {
    Home,
    AllEnabled,
    Provider,
    Forms,
}

impl HelpSection {
    pub(super) const ALL: [Self; 4] = [Self::Home, Self::AllEnabled, Self::Provider, Self::Forms];

    pub(super) fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|section| *section == self)
            .unwrap_or_default()
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::AllEnabled => "All Models",
            Self::Provider => "Provider",
            Self::Forms => "Forms",
        }
    }
}

#[derive(Clone)]
pub(super) struct HelpModal {
    pub(super) pi: bool,
    pub(super) codex: bool,
    pub(super) section: HelpSection,
    pub(super) scroll: u16,
}

impl HelpModal {
    pub(super) fn for_view(view_mode: ViewMode) -> Self {
        let section = match view_mode {
            ViewMode::Home => HelpSection::Home,
            ViewMode::AllEnabled => HelpSection::AllEnabled,
            ViewMode::Provider => HelpSection::Provider,
        };
        Self {
            pi: false,
            codex: false,
            section,
            scroll: 0,
        }
    }

    pub(super) fn move_section(&mut self, forward: bool) {
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

    pub(super) fn select(&mut self, index: usize) {
        if let Some(section) = HelpSection::ALL.get(index) {
            self.section = *section;
            self.scroll = 0;
        }
    }

    pub(super) fn scroll(&mut self, down: bool) {
        if down {
            self.scroll = self.scroll.saturating_add(1).min(32);
        } else {
            self.scroll = self.scroll.saturating_sub(1);
        }
    }
}

#[derive(Clone)]
pub(super) struct ProxyManager {
    pub(super) port_field: Option<FormField>,
    pub(super) port_changed: bool,
    pub(super) instance: uuid::Uuid,
    pub(super) runtime: Option<proxy::ProxyStatus>,
    pub(super) service: Option<proxy::ProxyServiceStatus>,
    pub(super) selected: usize,
    pub(super) message: String,
    pub(super) error: bool,
}

#[derive(Clone)]
pub(super) struct ProfileForm {
    pub(super) template_selected: Option<usize>,
    pub(super) instance: uuid::Uuid,
    pub(super) picker: Option<ModelForm>,
    pub(super) picker_search: bool,
    pub(super) original_id: Option<String>,
    pub(super) original_profile: Option<Profile>,
    pub(super) provider_enabled: bool,
    pub(super) models: Vec<ModelEntry>,
    pub(super) enabled_models: Vec<String>,
    pub(super) disabled_models: Vec<String>,
    pub(super) fields: Vec<FormField>,
    pub(super) selected: usize,
}

#[derive(Clone)]
pub(super) struct ModelForm {
    pub(super) original_model_id: Option<String>,
    pub(super) instance: uuid::Uuid,
    pub(super) original_profile: Option<Box<Profile>>,
    pub(super) fields: Vec<FormField>,
    pub(super) selected: usize,
    pub(super) api_models: Vec<ModelEntry>,
    pub(super) api_query: String,
    pub(super) api_query_cursor: usize,
    pub(super) api_scroll: usize,
    pub(super) api_selected: usize,
    pub(super) api_clicked: Option<String>,
    pub(super) focus_api_search: bool,
    pub(super) api_status: String,
}

#[derive(Clone)]
pub(super) struct RouteEditor {
    pub(super) profile_id: String,
    pub(super) original_profile: Profile,
    pub(super) provider_enabled: bool,
    pub(super) catalog: Vec<ModelEntry>,
    pub(super) enabled: BTreeSet<String>,
    pub(super) disabled: BTreeSet<String>,
    pub(super) locked: BTreeSet<String>,
    pub(super) default_model: String,
    pub(super) one_m: BTreeSet<String>,
    pub(super) query: String,
    pub(super) selected: usize,
    pub(super) search_active: bool,
    pub(super) status: String,
}

#[derive(Debug, Clone)]
pub(super) struct GlobalModelRef {
    pub(super) profile_id: String,
    pub(super) profile_name: String,
    pub(super) model: ModelEntry,
    pub(super) enabled: bool,
}

#[derive(Clone)]
pub(super) struct FormField {
    pub(super) label: &'static str,
    pub(super) value: String,
    pub(super) cursor: usize,
    pub(super) secret: bool,
    pub(super) toggle: bool,
    pub(super) choices: &'static [&'static str],
}
