//! The Orbital Laser mod, installed and kept up to date from its own releases
//! (`malik05051/orbital-laser`), so players of this fork get each new version without a new launcher.
//!
//! The release holds `orbital-laser-mod.zip` (one top folder, `orbital/`: what goes in the game folder's
//! `horizon-mods/orbital/`) and a `SHA256SUMS` listing it. Nothing is pinned here: the zip is checked
//! against that list and against GitHub's own digest of the asset when the API gives one.
//!
//! Before the game starts, the newest release is installed when no Orbital Laser is there or the one
//! there is older. An equal or newer copy is left alone (a player's own build of the mod), and so is one
//! whose version can't be read. The mod's saved settings (`horizon-mods/.horizon-mod-data/`) are never
//! touched. A failure (offline, a bad checksum) never stops the game: it starts with what is installed.
use std::{
    cmp::Ordering,
    fmt, fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use sha2::{Digest, Sha256};

/// Where the mod's releases are.
pub const MOD_REPO: &str = "malik05051/orbital-laser";
/// The mod's id: its folder in `horizon-mods/` and its key in `horizon-mods.txt`.
pub const MOD_ID: &str = "orbital";
/// The release asset holding the mod.
pub const ASSET: &str = "orbital-laser-mod.zip";
const SUMS: &str = "SHA256SUMS";
/// The game's folder of mods and its list of the ones turned on (horizon-client's `mod_list.rs`).
const MODS_DIR: &str = "horizon-mods";
const MOD_LIST: &str = "horizon-mods.txt";
const MANIFEST: &str = "horizon-mod.toml";
/// Where a download is unpacked and checked before it replaces the installed mod: in the game
/// folder, so the final move is a rename on the same disk, and outside `horizon-mods/`, so the game
/// never sees a half-written mod.
const STAGING: &str = ".orbital-mod-update";
/// A mod's `mod.wasm` is capped at 32 MiB; the zip and what it unpacks to stay well under these.
const MAX_ZIP: u64 = 64 << 20;
const MAX_UNPACKED: u64 = 256 << 20;

/// A release version: `1.2.3`, with an optional pre-release (`1.2.3-rc.1`, older than `1.2.3`).
/// Build metadata (`+…`) is ignored, as semver says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    core: [u64; 3],
    pre: Option<String>,
}

impl Version {
    /// `1.2.3` or a tag `v1.2.3`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let text = text.split('+').next()?;
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) if !pre.is_empty() => (core, Some(pre.to_string())),
            Some(_) => return None,
            None => (text, None),
        };
        let mut parts = core.split('.');
        let mut number = || -> Option<u64> {
            let part = parts.next()?;
            (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
                .then(|| part.parse().ok())
                .flatten()
        };
        let core = [number()?, number()?, number()?];
        parts.next().is_none().then_some(Self { core, pre })
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core
            .cmp(&other.core)
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => pre_release_order(a, b),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Semver's pre-release order: dot-separated, numbers by value and below words, words by ASCII.
fn pre_release_order(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.split('.'), b.split('.'));
    loop {
        return match (a.next(), b.next()) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
            (Some(x), Some(y)) => {
                let order = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if order == Ordering::Equal {
                    continue;
                }
                order
            }
        };
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let [major, minor, patch] = self.core;
        write!(f, "{major}.{minor}.{patch}")?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

/// What is in the game folder's `horizon-mods/orbital/`.
#[derive(Debug, PartialEq)]
pub enum Installed {
    Nothing,
    Version(Version),
    /// A folder whose version can't be read: left alone, as a player's own copy might be.
    Unreadable(String),
}

pub fn installed(home: &Path) -> Installed {
    let dir = home.join(MODS_DIR).join(MOD_ID);
    if fs::symlink_metadata(&dir).is_err() {
        return Installed::Nothing;
    }
    let Ok(text) = fs::read_to_string(dir.join(MANIFEST)) else {
        return Installed::Unreadable(format!("{} has no readable {MANIFEST}", dir.display()));
    };
    match manifest_field(&text, "version").and_then(|v| Version::parse(&v)) {
        Some(v) => Installed::Version(v),
        None => Installed::Unreadable(format!("{} has no version", dir.join(MANIFEST).display())),
    }
}

/// A top-level `key = "value"` of a `horizon-mod.toml` (before its first table). Read by hand so a
/// manifest with fields this launcher doesn't know still gives its version.
fn manifest_field(text: &str, key: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .take_while(|l| !l.starts_with('['))
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| {
            let v = v.split(" #").next().unwrap_or(v).trim();
            v.trim_matches(|c| c == '"' || c == '\'').to_string()
        })
}

/// Whether the release replaces what is installed: only when nothing is there or it is older.
pub fn should_install(installed: &Installed, release: &Version) -> bool {
    match installed {
        Installed::Nothing => true,
        Installed::Version(local) => local < release,
        Installed::Unreadable(_) => false,
    }
}

/// What a check before playing did.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// The player turned the updater off in Settings.
    Off,
    /// The mod has no release yet.
    NoRelease,
    UpToDate(Version),
    /// An equal or newer copy of their own, left alone.
    KeptNewer {
        local: Version,
        release: Version,
    },
    KeptUnreadable(String),
    Installed {
        from: Option<Version>,
        to: Version,
    },
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Outcome::Off => write!(f, "Orbital Laser updates are off"),
            Outcome::NoRelease => write!(f, "Orbital Laser has no release yet"),
            Outcome::UpToDate(v) => write!(f, "Orbital Laser {v} is up to date"),
            Outcome::KeptNewer { local, release } => write!(
                f,
                "Orbital Laser {local} kept (the newest release is {release})"
            ),
            Outcome::KeptUnreadable(why) => write!(f, "Orbital Laser left as it is: {why}"),
            Outcome::Installed { from: None, to } => write!(f, "Orbital Laser {to} installed"),
            Outcome::Installed { from: Some(v), to } => {
                write!(f, "Orbital Laser updated from {v} to {to}")
            }
        }
    }
}

/// The check run before the game starts: nothing when the player turned it off, otherwise
/// [`update`]. Its error says why the mod wasn't updated; the game starts anyway.
pub fn before_play(home: &Path) -> Result<Outcome, String> {
    if !crate::backend::Settings::load().orbital_mod_updates() {
        return Ok(Outcome::Off);
    }
    update(home).map_err(|e| format!("Orbital Laser not updated: {e}"))
}

/// Installs the newest release of the mod into `home`'s `horizon-mods/` when it should.
pub fn update(home: &Path) -> Result<Outcome, String> {
    let installed = installed(home);
    if let Installed::Unreadable(why) = installed {
        return Ok(Outcome::KeptUnreadable(why));
    }
    let Some(release) = latest_release()? else {
        return Ok(Outcome::NoRelease);
    };
    if !should_install(&installed, &release.version) {
        return Ok(match installed {
            Installed::Version(local) if local == release.version => Outcome::UpToDate(local),
            Installed::Version(local) => Outcome::KeptNewer {
                local,
                release: release.version,
            },
            _ => unreachable!("only a readable copy is kept"),
        });
    }
    install_release(home, installed, release)
}

/// Downloads, checks and installs `release` in place of what is `installed`.
fn install_release(home: &Path, installed: Installed, release: Release) -> Result<Outcome, String> {
    let sums = String::from_utf8(read_capped(&release.sums_url, 64 << 10)?)
        .map_err(|_| format!("{SUMS} is not text"))?;
    let listed =
        listed_sha256(&sums, ASSET).ok_or_else(|| format!("{SUMS} doesn't list {ASSET}"))?;
    let staging = home.join(STAGING);
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let result = (|| {
        let zip = staging.join(ASSET);
        let actual = download(&release.zip_url, &zip)?;
        verify(&actual, &listed, release.digest.as_deref())?;
        let unpacked = staging.join("unpacked");
        let version = unpack_checked(&zip, &unpacked)?;
        if version != release.version {
            return Err(format!(
                "the zip holds Orbital Laser {version}, not {}",
                release.version
            ));
        }
        replace(home, &unpacked.join(MOD_ID), &staging.join("old"))?;
        turn_on(home)?;
        Ok(())
    })();
    let _ = fs::remove_dir_all(&staging);
    result?;
    Ok(Outcome::Installed {
        from: match installed {
            Installed::Version(v) => Some(v),
            _ => None,
        },
        to: release.version,
    })
}

struct Release {
    version: Version,
    zip_url: String,
    /// GitHub's `sha256:…` digest of the zip, when the API gives one.
    digest: Option<String>,
    sums_url: String,
}

/// The newest release that isn't a draft or a pre-release; `None` when there is none yet.
fn latest_release() -> Result<Option<Release>, String> {
    let url = format!("https://api.github.com/repos/{MOD_REPO}/releases/latest");
    let Some(response) = crate::backend::get_unless_missing(&url)? else {
        return Ok(None);
    };
    let json: serde_json::Value = response.into_json().map_err(|e| format!("{url}: {e}"))?;
    release_from(&json).map(Some)
}

fn release_from(json: &serde_json::Value) -> Result<Release, String> {
    let tag = json["tag_name"].as_str().unwrap_or_default();
    let version =
        Version::parse(tag).ok_or_else(|| format!("its newest release {tag:?} isn't a version"))?;
    let asset = |name: &str| {
        json["assets"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|a| a["name"].as_str() == Some(name))
            .ok_or_else(|| format!("release {tag} has no {name}"))
    };
    let zip = asset(ASSET)?;
    let url = |a: &serde_json::Value| a["browser_download_url"].as_str().map(str::to_string);
    Ok(Release {
        version,
        zip_url: url(zip).ok_or_else(|| format!("release {tag}: {ASSET} has no address"))?,
        digest: zip["digest"].as_str().map(str::to_string),
        sums_url: url(asset(SUMS)?)
            .ok_or_else(|| format!("release {tag}: {SUMS} has no address"))?,
    })
}

/// The SHA-256 `sums` (as `sha256sum` writes it) gives for `name`.
fn listed_sha256(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        let file = file.trim_start().trim_start_matches('*');
        (file == name && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

/// The zip's SHA-256 must be the listed one, and GitHub's digest when there is one.
fn verify(actual: &str, listed: &str, digest: Option<&str>) -> Result<(), String> {
    if !actual.eq_ignore_ascii_case(listed) {
        return Err(format!(
            "{ASSET} doesn't match {SUMS} (got {actual}, listed {listed})"
        ));
    }
    if let Some(hash) = digest.and_then(|d| d.strip_prefix("sha256:")) {
        if !actual.eq_ignore_ascii_case(hash) {
            return Err(format!(
                "{ASSET} doesn't match GitHub's digest (got {actual}, GitHub says {hash})"
            ));
        }
    }
    Ok(())
}

fn read_capped(url: &str, cap: u64) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    crate::backend::get_unless_missing(url)?
        .ok_or_else(|| format!("{url} is missing"))?
        .into_reader()
        .take(cap + 1)
        .read_to_end(&mut body)
        .map_err(|e| format!("Download interrupted: {e}"))?;
    if body.len() as u64 > cap {
        return Err(format!("{url} is bigger than expected"));
    }
    Ok(body)
}

/// Downloads `url` to `to`, at most [`MAX_ZIP`]; returns its SHA-256.
fn download(url: &str, to: &Path) -> Result<String, String> {
    let mut reader = crate::backend::get_unless_missing(url)?
        .ok_or_else(|| format!("{url} is missing"))?
        .into_reader()
        .take(MAX_ZIP + 1);
    let mut file = fs::File::create(to).map_err(|e| e.to_string())?;
    let (mut hash, mut done, mut buf) = (Sha256::new(), 0u64, vec![0u8; 1 << 16]);
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("Download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        done += n as u64;
        if done > MAX_ZIP {
            return Err(format!("{ASSET} is bigger than {} MiB", MAX_ZIP >> 20));
        }
        hash.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// Where an entry of the zip may go: a relative path under `orbital/`, no `..`, nothing absolute.
fn entry_path(name: &str) -> Result<PathBuf, String> {
    let bad = || format!("{ASSET} holds {name:?}, outside {MOD_ID}/");
    if name.contains('\\') || name.contains('\0') {
        return Err(bad());
    }
    let path = Path::new(name);
    let mut parts = path.components();
    match parts.next() {
        Some(Component::Normal(first)) if first == MOD_ID => {}
        _ => return Err(bad()),
    }
    if parts.all(|c| matches!(c, Component::Normal(_))) {
        Ok(path.to_path_buf())
    } else {
        Err(bad())
    }
}

/// Unpacks the zip into `to` after checking every entry ([`entry_path`], no links), and returns the
/// version its `orbital/horizon-mod.toml` gives (its id must be `orbital`).
fn unpack_checked(zip: &Path, to: &Path) -> Result<Version, String> {
    let file = fs::File::open(zip).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("{ASSET}: {e}"))?;
    let mut paths = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| format!("{ASSET}: {e}"))?;
        let path = entry_path(entry.name())?;
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err(format!("{ASSET} holds a link, {}", entry.name()));
        }
        paths.push((path, entry.is_dir()));
    }
    let mut total = 0u64;
    for (i, (path, is_dir)) in paths.into_iter().enumerate() {
        let target = to.join(&path);
        if is_dir {
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let entry = archive.by_index(i).map_err(|e| format!("{ASSET}: {e}"))?;
        let mut out = fs::File::create(&target).map_err(|e| e.to_string())?;
        let copied = std::io::copy(&mut entry.take(MAX_UNPACKED - total + 1), &mut out)
            .map_err(|e| format!("{ASSET}: {e}"))?;
        total += copied;
        if total > MAX_UNPACKED {
            return Err(format!(
                "{ASSET} unpacks to more than {} MiB",
                MAX_UNPACKED >> 20
            ));
        }
    }
    let manifest = fs::read_to_string(to.join(MOD_ID).join(MANIFEST))
        .map_err(|_| format!("{ASSET} has no {MOD_ID}/{MANIFEST}"))?;
    let id = manifest_field(&manifest, "id").unwrap_or_default();
    if id != MOD_ID {
        return Err(format!("{ASSET} holds the mod {id:?}, not {MOD_ID}"));
    }
    manifest_field(&manifest, "version")
        .and_then(|v| Version::parse(&v))
        .ok_or_else(|| format!("{ASSET}'s {MANIFEST} has no version"))
}

/// Puts `new` in place of `horizon-mods/orbital` as a whole: the old folder is moved aside first
/// (to `old`) and moved back if the new one can't take its place.
fn replace(home: &Path, new: &Path, old: &Path) -> Result<(), String> {
    let mods = home.join(MODS_DIR);
    fs::create_dir_all(&mods).map_err(|e| e.to_string())?;
    let target = mods.join(MOD_ID);
    let had_one = fs::symlink_metadata(&target).is_ok();
    if had_one {
        fs::rename(&target, old).map_err(|e| format!("couldn't move the old mod aside: {e}"))?;
    }
    if let Err(e) = fs::rename(new, &target) {
        if had_one {
            let _ = fs::rename(old, &target);
        }
        return Err(format!("couldn't put the new mod in place: {e}"));
    }
    if had_one {
        let _ = fs::remove_dir_all(old);
    }
    Ok(())
}

/// `horizon-mods.txt` with `mod.orbital:true`, every other line kept as it was.
fn mod_list_with_orbital(text: &str) -> String {
    let key = format!("mod.{MOD_ID}");
    let mut out: String = text
        .lines()
        .filter(|l| l.split_once(':').is_none_or(|(k, _)| k.trim() != key))
        .map(|l| format!("{l}\n"))
        .collect();
    out.push_str(&format!("{key}:true\n"));
    out
}

/// Turns the mod on in the game's `horizon-mods.txt`, as Options > Mods would.
fn turn_on(home: &Path) -> Result<(), String> {
    let path = home.join(MOD_LIST);
    let text = fs::read_to_string(&path).unwrap_or_default();
    let merged = mod_list_with_orbital(&text);
    if merged != text {
        fs::write(&path, merged).map_err(|e| format!("{MOD_LIST}: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("orbital-mod-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn manifest(version: &str) -> String {
        format!("id = \"orbital\"\nname = \"Orbital Laser\"\nversion = \"{version}\"\napi = \"0.2\"\n\n[extra]\nversion = \"9.9.9\"\n")
    }

    fn zip_of(path: &Path, files: &[(&str, &str)]) {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, body) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn versions_compare_as_semver() {
        assert!(v("0.6.0") > v("0.5.9"));
        assert!(v("v0.10.0") > v("0.9.0"));
        assert!(v("1.0.0") > v("1.0.0-rc.1"));
        assert!(v("1.0.0-rc.10") > v("1.0.0-rc.2"));
        assert!(v("1.0.0-beta") > v("1.0.0-alpha.1"));
        assert_eq!(v("v0.6.0"), v("0.6.0+local"));
        assert_eq!(v("0.6.0").to_string(), "0.6.0");
        for bad in ["", "0.6", "0.6.0.1", "x.6.0", "0.6.0-", "0..1"] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn an_equal_or_newer_copy_is_left_alone() {
        let release = v("0.6.0");
        assert!(should_install(&Installed::Nothing, &release));
        assert!(should_install(&Installed::Version(v("0.5.2")), &release));
        assert!(!should_install(&Installed::Version(v("0.6.0")), &release));
        assert!(!should_install(&Installed::Version(v("0.7.0")), &release));
        assert!(!should_install(
            &Installed::Unreadable("?".into()),
            &release
        ));

        let home = temp("installed");
        assert_eq!(installed(&home), Installed::Nothing);
        let dir = home.join(MODS_DIR).join(MOD_ID);
        fs::create_dir_all(&dir).unwrap();
        assert!(matches!(installed(&home), Installed::Unreadable(_)));
        // The top-level version, not a table's.
        fs::write(dir.join(MANIFEST), manifest("0.6.1")).unwrap();
        assert_eq!(installed(&home), Installed::Version(v("0.6.1")));
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn zip_entries_stay_inside_the_mod_folder() {
        for good in ["orbital/", "orbital/mod.wasm", "orbital/assets/a/b.png"] {
            assert!(entry_path(good).is_ok(), "{good}");
        }
        for bad in [
            "mod.wasm",
            "other/mod.wasm",
            "orbital/../escape",
            "orbital/a/../../escape",
            "/orbital/mod.wasm",
            "../orbital/mod.wasm",
            "orbital\\..\\escape",
            "./orbital/mod.wasm",
            "orbitalx/mod.wasm",
        ] {
            assert!(entry_path(bad).is_err(), "{bad}");
        }

        let dir = temp("zip");
        let zip = dir.join("bad.zip");
        zip_of(
            &zip,
            &[
                ("orbital/horizon-mod.toml", &manifest("0.6.0")),
                ("../evil", "x"),
            ],
        );
        assert!(unpack_checked(&zip, &dir.join("out")).is_err());
        assert!(!dir.join("evil").exists());

        zip_of(
            &zip,
            &[
                ("orbital/horizon-mod.toml", &manifest("0.6.0")),
                ("orbital/mod.wasm", "w"),
            ],
        );
        assert_eq!(unpack_checked(&zip, &dir.join("ok")).unwrap(), v("0.6.0"));
        assert!(dir.join("ok/orbital/mod.wasm").is_file());

        let other = manifest("0.6.0").replace("\"orbital\"", "\"other\"");
        zip_of(&zip, &[("orbital/horizon-mod.toml", &other)]);
        assert!(unpack_checked(&zip, &dir.join("other")).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_checksum_mismatch_is_refused() {
        let sums = "0b1e  other.zip\nABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789  orbital-laser-mod.zip\n";
        let listed = listed_sha256(sums, ASSET).unwrap();
        assert_eq!(
            listed,
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        );
        assert_eq!(listed_sha256(sums, "missing.zip"), None);
        let good = listed.clone();
        let bad = "0".repeat(64);
        assert!(verify(&good, &listed, None).is_ok());
        assert!(verify(&good, &listed, Some(&format!("sha256:{good}"))).is_ok());
        assert!(verify(&bad, &listed, None).is_err());
        assert!(verify(&good, &listed, Some(&format!("sha256:{bad}"))).is_err());
    }

    #[test]
    fn horizon_mods_txt_gets_orbital_and_keeps_the_rest() {
        assert_eq!(mod_list_with_orbital(""), "mod.orbital:true\n");
        assert_eq!(
            mod_list_with_orbital("mod.coords_hud:true\nmod.orbital:false"),
            "mod.coords_hud:true\nmod.orbital:true\n"
        );
        let on = "mod.coords_hud:true\nmod.orbital:true\n";
        assert_eq!(mod_list_with_orbital(on), on);
    }

    #[test]
    fn replacing_keeps_the_mods_saved_settings() {
        let home = temp("replace");
        let mods = home.join(MODS_DIR);
        fs::create_dir_all(mods.join(MOD_ID)).unwrap();
        fs::write(mods.join(MOD_ID).join("stale.txt"), "old").unwrap();
        fs::create_dir_all(mods.join(".horizon-mod-data/orbital")).unwrap();
        fs::write(mods.join(".horizon-mod-data/orbital/settings"), "mine").unwrap();
        let new = home.join("new");
        fs::create_dir_all(&new).unwrap();
        fs::write(new.join(MANIFEST), manifest("0.6.0")).unwrap();
        replace(&home, &new, &home.join("old")).unwrap();
        assert!(!mods.join(MOD_ID).join("stale.txt").exists());
        assert_eq!(installed(&home), Installed::Version(v("0.6.0")));
        assert_eq!(
            fs::read_to_string(mods.join(".horizon-mod-data/orbital/settings")).unwrap(),
            "mine"
        );
        assert!(!home.join("old").exists());
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_release_needs_the_zip_and_its_checksums() {
        let json = serde_json::json!({
            "tag_name": "v0.6.0",
            "assets": [
                {"name": ASSET, "browser_download_url": "https://x/zip", "digest": "sha256:ab"},
                {"name": SUMS, "browser_download_url": "https://x/sums"},
            ],
        });
        let release = release_from(&json).unwrap();
        assert_eq!(release.version, v("0.6.0"));
        assert_eq!(release.digest.as_deref(), Some("sha256:ab"));
        let no_sums = serde_json::json!({"tag_name": "v0.6.0", "assets": [json["assets"][0]]});
        assert!(release_from(&no_sums).is_err());
        assert!(release_from(&serde_json::json!({"tag_name": "nightly", "assets": []})).is_err());
    }
}
