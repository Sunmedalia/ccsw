//! Verified GitHub Release updates and safe fast-forward updates for a Herdr checkout.
use anyhow::{Context, Result, bail, ensure};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Read, Write},
    path::Path,
    process::Command,
    time::Duration,
};

const RELEASE_API: &str = "https://api.github.com/repos/Sunmedalia/ccsw/releases/latest";
const ASSET_PREFIX: &str = "https://github.com/Sunmedalia/ccsw/releases/download/";
const MAX_ARCHIVE: u64 = 100 * 1024 * 1024;
const MAX_BINARY: u64 = 150 * 1024 * 1024;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
    size: u64,
}

fn asset_name() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("ccsw-macos-arm64.tar.gz"),
        ("linux", "x86_64") => Ok("ccsw-linux-x86_64.tar.gz"),
        ("linux", "aarch64") => Ok("ccsw-linux-arm64.tar.gz"),
        ("windows", "x86_64") => Ok("ccsw-windows-x86_64.zip"),
        _ => bail!("No CCSW release package for this operating system and architecture"),
    }
}

fn release_version(release: &Release) -> Result<Version> {
    Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name),
    )
    .context("Latest release has an invalid version")
}

fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(format!("ccsw/{}", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(120))
        .build()?)
}

fn fetch_release(client: &reqwest::blocking::Client) -> Result<Release> {
    Ok(client.get(RELEASE_API).send()?.error_for_status()?.json()?)
}

fn download(client: &reqwest::blocking::Client, asset: &Asset) -> Result<Vec<u8>> {
    ensure!(
        asset.size > 0 && asset.size <= MAX_ARCHIVE,
        "Release archive is too large"
    );
    ensure!(
        asset.browser_download_url.starts_with(ASSET_PREFIX),
        "Unexpected release download URL"
    );
    let expected = asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .context("Release asset has no SHA-256 digest; update refused")?;
    ensure!(
        expected.len() == 64 && expected.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid release SHA-256 digest"
    );
    let mut response = client
        .get(&asset.browser_download_url)
        .send()?
        .error_for_status()?;
    let mut archive = Vec::new();
    response
        .by_ref()
        .take(MAX_ARCHIVE + 1)
        .read_to_end(&mut archive)?;
    ensure!(
        archive.len() as u64 <= MAX_ARCHIVE,
        "Release archive exceeds size limit"
    );
    verify_digest(&archive, expected)?;
    Ok(archive)
}

fn verify_digest(bytes: &[u8], expected: &str) -> Result<()> {
    ensure!(
        format!("{:x}", Sha256::digest(bytes)).eq_ignore_ascii_case(expected),
        "Release SHA-256 mismatch; installed binary unchanged"
    );
    Ok(())
}

fn binary_from_archive(bytes: &[u8]) -> Result<Vec<u8>> {
    #[cfg(windows)]
    {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;
        let mut file = zip
            .by_name("ccsw.exe")
            .context("Release ZIP has no ccsw.exe")?;
        ensure!(
            file.is_file() && file.size() > 0 && file.size() <= MAX_BINARY,
            "Invalid release executable"
        );
        let mut binary = Vec::new();
        file.by_ref()
            .take(MAX_BINARY + 1)
            .read_to_end(&mut binary)?;
        ensure!(
            binary.len() as u64 <= MAX_BINARY,
            "Release executable exceeds size limit"
        );
        Ok(binary)
    }
    #[cfg(not(windows))]
    {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(Cursor::new(bytes)));
        let mut binary = None;
        for entry in tar.entries()? {
            let mut entry = entry?;
            if entry.path()?.as_ref() != Path::new("ccsw") {
                continue;
            }
            ensure!(
                binary.is_none()
                    && entry.header().entry_type().is_file()
                    && entry.size() > 0
                    && entry.size() <= MAX_BINARY,
                "Invalid release executable"
            );
            let mut contents = Vec::new();
            entry
                .by_ref()
                .take(MAX_BINARY + 1)
                .read_to_end(&mut contents)?;
            ensure!(
                contents.len() as u64 <= MAX_BINARY,
                "Release executable exceeds size limit"
            );
            binary = Some(contents);
        }
        binary.context("Release archive has no ccsw executable")
    }
}

fn install_binary(binary: &[u8]) -> Result<()> {
    let target = std::env::current_exe()?;
    ensure!(
        !fs::symlink_metadata(&target)?.file_type().is_symlink(),
        "Refusing to replace a symlinked executable"
    );
    ensure!(
        target
            .file_name()
            .is_some_and(|name| name == "ccsw" || name == "ccsw.exe"),
        "Run the installed ccsw binary to update it"
    );
    let parent = target
        .parent()
        .context("Executable has no parent directory")?;
    ensure!(
        !parent.ends_with("target/release") && !parent.ends_with("target/debug"),
        "This is a source build; use ccsw update --source <checkout>"
    );
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    staged.write_all(binary)?;
    staged
        .as_file()
        .set_permissions(fs::metadata(&target)?.permissions())?;
    staged.as_file().sync_all()?;
    #[cfg(windows)]
    {
        match staged.persist(&target) {
            Ok(_) => println!(
                "Updated {}. Restart running CCSW windows and the proxy when idle.",
                target.display()
            ),
            Err(error) => {
                let path = error.file.into_temp_path().keep()?;
                println!(
                    "Windows has locked the running executable. Verified update staged at {}. Close CCSW, replace {} with that file, then reopen.",
                    path.display(),
                    target.display()
                );
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        staged.persist(&target).map_err(|e| e.error)?;
        println!(
            "Updated {}. Restart running CCSW windows and the proxy when idle.",
            target.display()
        );
        Ok(())
    }
}

fn command(source: &Path, program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .current_dir(source)
        .status()
        .with_context(|| format!("Cannot run {program}"))?;
    ensure!(
        status.success(),
        "{program} failed; installed CCSW was not replaced"
    );
    Ok(())
}

fn update_source(source: &Path) -> Result<()> {
    ensure!(
        std::env::var("HERDR_ENV").as_deref() == Ok("1"),
        "Run source updates in a Herdr terminal"
    );
    let source = fs::canonicalize(source).context("Cannot find source checkout")?;
    let manifest: toml::Value = fs::read_to_string(source.join("herdr-plugin.toml"))?.parse()?;
    ensure!(
        manifest.get("id").and_then(toml::Value::as_str) == Some("ccsw"),
        "Not a CCSW Herdr checkout"
    );
    let status = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .current_dir(&source)
        .output()?;
    ensure!(
        status.status.success() && status.stdout.is_empty(),
        "Checkout has local tracked changes; commit or stash them first"
    );
    command(&source, "git", &["pull", "--ff-only"])?;
    command(
        &source,
        "cargo",
        &[
            "build",
            "--locked",
            "--release",
            "--bin",
            "ccsw",
            "--target-dir",
            "target",
        ],
    )?;
    let status = Command::new(source.join("target/release/ccsw"))
        .arg("herdr-install")
        .arg("--source")
        .arg(&source)
        .status()
        .context("Cannot run updated CCSW installer")?;
    ensure!(status.success(), "Updated CCSW plugin installer failed");
    println!("Source checkout updated. Reopen existing CCSW panes to load the new version.");
    Ok(())
}

pub fn run(check: bool, source: Option<&Path>) -> Result<()> {
    if let Some(source) = source {
        ensure!(!check, "--check and --source cannot be combined");
        return update_source(source);
    }
    let client = client()?;
    let release = fetch_release(&client)?;
    let latest = release_version(&release)?;
    let current = Version::parse(env!("CARGO_PKG_VERSION"))?;
    println!("Installed: {current} · Latest release: {latest}");
    if latest <= current {
        println!("Already up to date.");
        return Ok(());
    }
    if check {
        return Ok(());
    }
    let name = asset_name()?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .with_context(|| format!("Latest release has no {name} package"))?;
    println!("Downloading and verifying {}…", asset.name);
    let archive = download(&client, asset)?;
    install_binary(&binary_from_archive(&archive)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_version_accepts_tag_and_rejects_invalid_version() {
        let release = Release {
            tag_name: "v0.1.14".into(),
            assets: Vec::new(),
        };
        assert_eq!(release_version(&release).unwrap(), Version::new(0, 1, 14));
        let invalid = Release {
            tag_name: "latest".into(),
            assets: Vec::new(),
        };
        assert!(release_version(&invalid).is_err());
    }

    #[test]
    fn release_asset_matches_target() {
        let name = asset_name().unwrap();
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "aarch64") => assert_eq!(name, "ccsw-linux-arm64.tar.gz"),
            ("linux", "x86_64") => assert_eq!(name, "ccsw-linux-x86_64.tar.gz"),
            ("macos", "aarch64") => assert_eq!(name, "ccsw-macos-arm64.tar.gz"),
            ("windows", "x86_64") => assert_eq!(name, "ccsw-windows-x86_64.zip"),
            _ => unreachable!(),
        }
    }

    #[test]
    fn release_digest_rejects_modified_archive() {
        let expected = format!("{:x}", Sha256::digest(b"archive"));
        verify_digest(b"archive", &expected).unwrap();
        assert!(verify_digest(b"changed", &expected).is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn release_archive_extracts_only_the_named_regular_executable() {
        let mut compressed = Vec::new();
        {
            let encoder =
                flate2::write::GzEncoder::new(&mut compressed, flate2::Compression::default());
            let mut archive = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(6);
            header.set_mode(0o755);
            header.set_cksum();
            archive
                .append_data(&mut header, "ccsw", &b"binary"[..])
                .unwrap();
            archive.into_inner().unwrap().finish().unwrap();
        }
        assert_eq!(binary_from_archive(&compressed).unwrap(), b"binary");
        assert!(binary_from_archive(b"not gzip").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn release_zip_extracts_only_the_named_executable() {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        archive
            .start_file("README.md", zip::write::FileOptions::default())
            .unwrap();
        archive.write_all(b"readme").unwrap();
        archive
            .start_file("ccsw.exe", zip::write::FileOptions::default())
            .unwrap();
        archive.write_all(b"binary").unwrap();
        let bytes = archive.finish().unwrap().into_inner();
        assert_eq!(binary_from_archive(&bytes).unwrap(), b"binary");
        assert!(binary_from_archive(b"not zip").is_err());
    }
}
