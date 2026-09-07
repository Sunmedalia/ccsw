use super::*;

impl RouteEditor {
    pub(super) fn filtered_indices(&self) -> Vec<usize> {
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

    pub(super) fn selected_catalog_index(&self) -> Option<usize> {
        self.filtered_indices().get(self.selected).copied()
    }

    pub(super) fn selected_model(&self) -> Option<&ModelEntry> {
        self.selected_catalog_index()
            .and_then(|index| self.catalog.get(index))
    }

    pub(super) fn is_required(&self, id: &str) -> bool {
        if !self.provider_enabled {
            return false;
        }
        if self.disabled.contains(id) {
            return false;
        }
        id == self.default_model || self.locked.contains(id)
    }

    pub(super) fn is_enabled(&self, id: &str) -> bool {
        if !self.provider_enabled {
            return false;
        }
        if self.disabled.contains(id) {
            return false;
        }
        self.is_required(id) || self.enabled.contains(id)
    }

    pub(super) fn effective_id(&self, id: &str) -> String {
        let base = canonical_model_id(id);
        if self.one_m.contains(&base) {
            format!("{base}[1m]")
        } else {
            base
        }
    }

    pub(super) fn toggle_selected(&mut self) {
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

    pub(super) fn toggle_selected_1m(&mut self) {
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

    pub(super) fn set_selected_default(&mut self) {
        let Some(id) = self.selected_model().map(|model| model.id.clone()) else {
            self.status = "No model matches this search".into();
            return;
        };
        self.disabled.remove(&id);
        self.enabled.remove(&id);
        self.default_model = id.clone();
        self.status = format!("Default model set to {id}");
    }

    pub(super) fn enable_all_filtered(&mut self) {
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

    pub(super) fn disable_all_filtered(&mut self) {
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

pub(super) fn has_1m_suffix(id: &str) -> bool {
    id.to_ascii_lowercase().ends_with("[1m]")
}

pub(super) fn canonical_model_id(id: &str) -> String {
    if has_1m_suffix(id) {
        id[..id.len().saturating_sub(4)].to_owned()
    } else {
        id.to_owned()
    }
}

pub(super) fn normalize_model_catalog(models: Vec<ModelEntry>) -> Vec<ModelEntry> {
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

pub(super) fn apply_route_editor(profile: &mut Profile, editor: &RouteEditor) {
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
        *model = editor.effective_id(model);
    }
    profile.fallback_models = profile
        .fallback_models
        .iter()
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
