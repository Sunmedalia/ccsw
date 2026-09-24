//! Local Herdr plugin setup. Preserve unrelated config, credentials and panes.
use anyhow::{Context, Result, bail, ensure};
use std::{fs, io::Write, path::Path, process::Command};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

fn shortcut(text: &str, key: &str) -> Result<(String, String)> {
    ensure!(
        !key.trim().is_empty() && !key.contains(['\n', '\r']),
        "Invalid shortcut"
    );
    let mut doc: DocumentMut = text
        .parse()
        .context("Cannot parse Herdr config; no changes made")?;
    if let Some(commands) = doc.get("keys").and_then(|v| v.get("command")) {
        let commands = commands
            .as_array_of_tables()
            .context("keys.command must be an array of tables")?;
        // Preserve an existing user-selected shortcut for this plugin.
        if let Some(existing) = commands.iter().find(|t| {
            t.get("type").and_then(Item::as_str) == Some("plugin_action")
                && t.get("command").and_then(Item::as_str) == Some("ccsw.open")
                && t.get("key").and_then(Item::as_str).is_some()
        }) {
            return Ok((text.into(), existing["key"].as_str().unwrap().into()));
        }
        ensure!(
            !commands
                .iter()
                .any(|t| t.get("key").and_then(Item::as_str) == Some(key)),
            "Shortcut {key} is already bound. Re-run with --key prefix+shift+u (or another unused key)"
        );
    }
    if doc.get("keys").is_none() {
        doc["keys"] = Item::Table(Table::new());
    }
    let keys = doc["keys"]
        .as_table_mut()
        .context("keys must be a TOML table")?;
    // Also protect ordinary key settings, not only command bindings.
    ensure!(
        !keys
            .iter()
            .any(|(name, item)| name != "command" && item.as_str() == Some(key)),
        "Shortcut {key} is already used by a Herdr key setting; choose another --key"
    );
    if keys.get("command").is_none() {
        keys["command"] = Item::ArrayOfTables(ArrayOfTables::new());
    }
    let mut binding = Table::new();
    binding["key"] = value(key);
    binding["type"] = value("plugin_action");
    binding["command"] = value("ccsw.open");
    binding["description"] = value("Toggle CCSW Pulse usage monitor");
    keys["command"]
        .as_array_of_tables_mut()
        .unwrap()
        .push(binding);
    Ok((doc.to_string(), key.into()))
}

fn herdr(args: &[&std::ffi::OsStr]) -> Result<()> {
    let status = Command::new("herdr")
        .args(args)
        .status()
        .context("Cannot run herdr")?;
    ensure!(
        status.success(),
        "Herdr command failed; fix the error above and re-run the installer"
    );
    Ok(())
}

fn backup(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut copy = tempfile::Builder::new()
        .prefix("ccsw-install-backup-")
        .tempfile_in(path.parent().unwrap())?;
    std::io::copy(&mut fs::File::open(path)?, &mut copy)?;
    copy.as_file()
        .set_permissions(fs::metadata(path)?.permissions())?;
    copy.as_file().sync_all()?;
    let (_, saved) = copy.keep()?;
    println!("Backup: {}", saved.display());
    Ok(())
}

fn config_path() -> Result<std::path::PathBuf> {
    crate::platform::override_path("HERDR_CONFIG_PATH", || {
        Ok(crate::platform::home()?.join(".config/herdr/config.toml"))
    })
}

fn read_config(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.into()),
    }
}

fn write_config(path: &Path, original: &str, updated: &str) -> Result<()> {
    if updated == original {
        return Ok(());
    }
    fs::create_dir_all(path.parent().unwrap())?;
    // Do not replace edits made by another process while preparing the shortcut.
    ensure!(
        read_config(path)? == original,
        "Herdr config changed during installation; re-run to merge the shortcut"
    );
    backup(path)?;
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    temp.write_all(updated.as_bytes())?;
    if let Ok(meta) = fs::metadata(path) {
        temp.as_file().set_permissions(meta.permissions())?;
    }
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}

/// Called by the GitHub install build hook, before Herdr registers the plugin.
pub fn bind_default() -> Result<()> {
    let config = config_path()?;
    let original = read_config(&config)?;
    let (updated, key) = match shortcut(&original, "prefix+u") {
        Ok(binding) => binding,
        Err(error) => {
            eprintln!("CCSW shortcut skipped: {error:#}");
            return Ok(());
        }
    };
    write_config(&config, &original, &updated)?;
    println!("CCSW shortcut: {key}");
    if updated != original {
        // A server may not be running during plugin install. It will read the
        // config on startup; a live server can pick it up immediately.
        if !Command::new("herdr")
            .args(["server", "reload-config"])
            .status()
            .is_ok_and(|status| status.success())
        {
            eprintln!("CCSW shortcut saved; restart Herdr to activate it");
        }
    }
    Ok(())
}

pub fn run(source: &Path, key: &str) -> Result<()> {
    ensure!(
        cfg!(any(target_os = "macos", target_os = "linux")),
        "Herdr plugin supports macOS / Linux"
    );
    ensure!(
        std::env::var("HERDR_ENV").as_deref() == Ok("1"),
        "Run inside a Herdr terminal"
    );
    let source = fs::canonicalize(source).context("Cannot find the source checkout")?;
    let manifest: toml::Value = fs::read_to_string(source.join("herdr-plugin.toml"))?.parse()?;
    ensure!(
        manifest.get("id").and_then(toml::Value::as_str) == Some("ccsw"),
        "Expected the ccsw plugin manifest"
    );
    let binary = source.join("target/release/ccsw");
    ensure!(
        binary.is_file(),
        "Build first: bash scripts/install-herdr.sh"
    );
    let config = config_path()?;
    let original = read_config(&config)?;
    let (updated, bound_key) = shortcut(&original, key)?;
    // Validate the live plugin API before writing configuration or binaries.
    herdr(&[
        "plugin".as_ref(),
        "list".as_ref(),
        "--plugin".as_ref(),
        "ccsw".as_ref(),
        "--json".as_ref(),
    ])?;
    let destination = crate::platform::home()?.join(".local/bin/ccsw");
    fs::create_dir_all(destination.parent().unwrap())?;
    if destination
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        bail!(
            "{} is a symlink; choose a regular user installation before running this installer",
            destination.display()
        );
    }
    if !destination.exists() || fs::read(&destination)? != fs::read(&binary)? {
        backup(&destination)?;
        let mut temp = tempfile::NamedTempFile::new_in(destination.parent().unwrap())?;
        std::io::copy(&mut fs::File::open(&binary)?, &mut temp)?;
        temp.as_file()
            .set_permissions(fs::metadata(&binary)?.permissions())?;
        temp.as_file().sync_all()?;
        temp.persist(&destination)?; // Atomic replacement; running binaries remain valid.
    }
    herdr(&["plugin".as_ref(), "link".as_ref(), source.as_os_str()])?;
    herdr(&["plugin".as_ref(), "enable".as_ref(), "ccsw".as_ref()])?;
    write_config(&config, &original, &updated)?;
    herdr(&["server".as_ref(), "reload-config".as_ref()])?;
    println!("Installed CCSW from {}", source.display());
    println!("Binary: {}", destination.display());
    println!("Shortcut: {bound_key} (default prefix is Ctrl+B)");
    println!("Keep this checkout in place. Re-run the same script after updating the source.");
    println!("Reopen existing CCSW panes to use the new version.");
    println!("For new usage metrics, when API requests are idle, run:");
    println!("  {} proxy stop", destination.display());
    println!("  {} proxy start", destination.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merge_is_repeatable_and_preserves_other_plugins_and_comments() {
        let text = "# user settings\nonboarding = false\n[keys]\nswitch_workspace = 'alt+shift+1..9'\n[[keys.command]]\nkey = 'prefix+f'\ntype = 'plugin_action'\ncommand = 'files.open'\n";
        let (merged, key) = shortcut(text, "prefix+u").unwrap();
        assert_eq!(key, "prefix+u");
        assert!(merged.contains("# user settings"));
        assert!(merged.contains("files.open"));
        assert!(merged.contains("switch_workspace"));
        assert_eq!(shortcut(&merged, "prefix+u").unwrap().0, merged);
        assert_eq!(merged.matches("ccsw.open").count(), 1);
    }
    #[test]
    fn existing_shortcut_is_preserved_and_conflicts_do_not_overwrite() {
        let text = "[[keys.command]]\nkey='prefix+x'\ntype='plugin_action'\ncommand='ccsw.open'\n";
        assert_eq!(
            shortcut(text, "prefix+u").unwrap(),
            (text.into(), "prefix+x".into())
        );
        let conflict =
            "[[keys.command]]\nkey='prefix+u'\ntype='plugin_action'\ncommand='files.open'\n";
        assert!(shortcut(conflict, "prefix+u").is_err());
        assert!(shortcut(conflict, "prefix+shift+u").is_ok());
        assert!(shortcut("[keys]\nother='prefix+u'", "prefix+u").is_err());
        assert!(shortcut("broken = [", "prefix+u").is_err());
    }
}
