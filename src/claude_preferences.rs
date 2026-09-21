//! Client preferences and per-field ownership. Values are never interpreted as shell code.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const PRESETS: [(&str, &str); 5] = [
    ("Teammates", "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS"),
    ("Tool Search", "ENABLE_TOOL_SEARCH"),
    ("Effort", "CLAUDE_CODE_EFFORT_LEVEL"),
    ("Disable updates", "DISABLE_AUTOUPDATER"),
    ("Disable Artifact", "CLAUDE_CODE_DISABLE_ARTIFACT"),
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hide_attribution: Option<bool>,
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        for (key, value) in &self.env {
            if key.is_empty()
                || !key.bytes().enumerate().all(|(i, b)| {
                    b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit())
                })
            {
                bail!("Invalid environment variable name: {key}");
            }
            if crate::claude_config::MANAGED_ENV_KEYS.contains(&key.as_str())
                || key == "CLAUDE_CONFIG_DIR"
                || key.starts_with("CLAUDE_CODE_USE_")
                || key.starts_with("ANTHROPIC_")
            {
                bail!("{key} controls routing, credentials or models; configure it in Providers");
            }
            if value.contains('\0') {
                bail!("{key} contains a NUL character");
            }
            if PRESETS.iter().any(|(_, preset)| *preset == key)
                && key != "CLAUDE_CODE_EFFORT_LEVEL"
                && key != "ENABLE_TOOL_SEARCH"
                && !["0", "1", "true", "false"].contains(&value.to_ascii_lowercase().as_str())
            {
                bail!("{key} requires 1/0 or true/false");
            }
            if key == "ENABLE_TOOL_SEARCH"
                && !["true", "false", "auto"].contains(&value.as_str())
                && value
                    .strip_prefix("auto:")
                    .and_then(|v| v.parse::<u8>().ok())
                    .is_none_or(|n| n > 100)
            {
                bail!("{key} requires true, false, auto, or auto:0 through auto:100");
            }
            if key == "CLAUDE_CODE_EFFORT_LEVEL"
                && !["auto", "low", "medium", "high", "xhigh", "max"].contains(&value.as_str())
            {
                bail!("Invalid effort level for {key}");
            }
        }
        Ok(())
    }
    pub fn desired(&self) -> BTreeMap<String, Option<Value>> {
        let mut values: BTreeMap<_, _> = self
            .env
            .iter()
            .map(|(k, v)| (format!("/env/{k}"), Some(json!(v))))
            .collect();
        if let Some(hide) = self.hide_attribution {
            values.insert("/includeCoAuthoredBy".into(), Some(json!(!hide)));
            for key in ["commit", "pr"] {
                values.insert(format!("/attribution/{key}"), hide.then(|| json!("")));
            }
        }
        values
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Owned {
    // Option distinguishes missing from explicit JSON null.
    #[serde(with = "optional_value")]
    pub before: Option<Value>,
    #[serde(with = "optional_value")]
    pub after: Option<Value>,
}
pub type Ownership = BTreeMap<String, Owned>;

fn put(root: &mut Value, path: &str, value: Option<Value>) -> Result<()> {
    let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
    let mut node = root;
    for part in &parts[..parts.len() - 1] {
        if node.get(*part).is_none() {
            if value.is_none() {
                return Ok(());
            }
            node[*part] = json!({});
        }
        node = node.get_mut(*part).unwrap();
        if !node.is_object() {
            bail!("Claude settings {part} must be an object");
        }
    }
    let object = node
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Claude settings must be an object"))?;
    if let Some(value) = value {
        object.insert(parts.last().unwrap().to_string(), value);
    } else {
        object.remove(*parts.last().unwrap());
    }
    Ok(())
}
pub fn conflicts(owned: &Ownership, root: &Value) -> Vec<String> {
    owned
        .iter()
        .filter(|(p, o)| root.pointer(p).cloned() != o.after)
        .map(|(p, _)| p.clone())
        .collect()
}
pub fn restore(root: &mut Value, owned: &Ownership) -> Result<()> {
    for (path, saved) in owned {
        if root.pointer(path).cloned() == saved.after {
            put(root, path, saved.before.clone())?;
        }
    }
    Ok(())
}
pub fn apply(root: &mut Value, settings: &Settings, previous: &Ownership) -> Result<Ownership> {
    settings.validate()?;
    let desired = settings.desired();
    let mut next = Ownership::new();
    for (path, saved) in previous {
        if !desired.contains_key(path) && root.pointer(path).cloned() == saved.after {
            put(root, path, saved.before.clone())?;
        }
    }
    for (path, mut after) in desired {
        let current = root.pointer(&path).cloned();
        let before = previous
            .get(&path)
            .filter(|o| o.after == current)
            .map(|o| o.before.clone())
            .unwrap_or(current);
        if settings.hide_attribution == Some(false) && path.starts_with("/attribution/") {
            after = before.clone();
        }
        put(root, &path, after.clone())?;
        next.insert(path, Owned { before, after });
    }
    Ok(next)
}
pub fn from_snapshot(snapshot: Option<&Value>) -> Ownership {
    snapshot
        .and_then(|s| s.get("client_preferences"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ownership_preserves_missing_null_and_external_changes() {
        let mut document = json!({"env":{"KEEP":"yes","A":null,"B":"original"},"attribution":{"commit":"custom","extra":true}});
        let original = document.clone();
        let settings = Settings {
            env: [
                ("A".into(), "".into()),
                ("B".into(), "new".into()),
                ("C".into(), "value".into()),
            ]
            .into(),
            hide_attribution: Some(true),
        };
        let owned = apply(&mut document, &settings, &Ownership::new()).unwrap();
        let owned = apply(&mut document, &settings, &owned).unwrap();
        assert_eq!(document["attribution"]["extra"], true);
        assert_eq!(document["env"]["A"], "");
        let encoded = serde_json::to_value(&owned).unwrap();
        let owned: Ownership = serde_json::from_value(encoded).unwrap();
        restore(&mut document, &owned).unwrap();
        assert_eq!(document, original);
        let owned = apply(&mut document, &settings, &Ownership::new()).unwrap();
        document["env"]["B"] = json!("external");
        assert_eq!(conflicts(&owned, &document), ["/env/B"]);
        restore(&mut document, &owned).unwrap();
        assert_eq!(document["env"]["B"], "external");
        assert_eq!(document["env"]["A"], Value::Null);
        assert!(document["env"].get("C").is_none());
    }
    #[test]
    fn explicit_retake_uses_external_baseline_and_removal_restores() {
        let settings = Settings {
            env: [("A".into(), "ours".into())].into(),
            ..Settings::default()
        };
        let mut document = json!({"env":{"A":"before"}});
        let owned = apply(&mut document, &settings, &Ownership::new()).unwrap();
        document["env"]["A"] = json!("external");
        let owned = apply(&mut document, &settings, &owned).unwrap();
        apply(&mut document, &Settings::default(), &owned).unwrap();
        assert_eq!(document["env"]["A"], "external");
    }
    #[test]
    fn show_restores_custom_attribution_and_default_does_not_take_ownership() {
        let mut document = json!({"attribution":{"commit":"custom","pr":"custom-pr"}});
        let hide = Settings {
            hide_attribution: Some(true),
            ..Settings::default()
        };
        let owned = apply(&mut document, &hide, &Ownership::new()).unwrap();
        let show = Settings {
            hide_attribution: Some(false),
            ..Settings::default()
        };
        let owned = apply(&mut document, &show, &owned).unwrap();
        assert_eq!(document["attribution"]["commit"], "custom");
        assert_eq!(document["includeCoAuthoredBy"], true);
        assert!(!owned.is_empty());
        let owned = apply(&mut document, &Settings::default(), &owned).unwrap();
        assert!(owned.is_empty());
        assert!(document.get("includeCoAuthoredBy").is_none());
    }
    #[test]
    fn rejects_routing_and_invalid_names_without_exposing_values() {
        for key in [
            "ANTHROPIC_BASE_URL",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CONFIG_DIR",
            "1BAD",
            "A/B",
            "",
        ] {
            let settings = Settings {
                env: [(key.into(), "private-value".into())].into(),
                ..Settings::default()
            };
            let error = settings.validate().unwrap_err().to_string();
            assert!(!error.contains("private-value"));
        }
        Settings {
            env: [("CUSTOM_1".into(), "$(literal)\nvalue".into())].into(),
            ..Settings::default()
        }
        .validate()
        .unwrap();
    }
}

// JSON null and a missing field must survive serialization distinctly.
mod optional_value {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        value: &Option<Value>,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        value.iter().collect::<Vec<_>>().serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Option<Value>, D::Error> {
        let mut values = Vec::<Value>::deserialize(deserializer)?;
        if values.len() > 1 {
            return Err(serde::de::Error::custom("invalid owned field value"));
        }
        Ok(values.pop())
    }
}
