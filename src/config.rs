use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlayerKind {
    Mpv,
    Afplay,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    pub source_dir: PathBuf,
    pub filtered_dir: PathBuf,
    pub player: PlayerKind,
    pub mpv_socket: PathBuf,
    /// Switch the macOS default output device's sample rate + bit depth to
    /// match each track before playback (CoreAudio, in-process; no external
    /// app required). No-op on non-macOS. Defaults to true.
    #[serde(default = "default_true")]
    pub switch_sample_rate: bool,
}

fn default_true() -> bool { true }

/// Resolve the config file path.
/// Honors `$XDG_CONFIG_HOME`, else falls back to `~/.config` (via `dirs`).
pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::config_dir())
        .unwrap_or_else(|| PathBuf::from("/tmp/.config"));
    base.join("jukebox").join("config.yml")
}

impl Config {
    /// Derive a Config from a source dir: filtered_dir is the `filtered_lossless`
    /// sibling of `source_dir`.
    pub fn default_for(source_dir: PathBuf) -> Self {
        let filtered_dir = source_dir
            .parent()
            .map(|p| p.join("filtered_lossless"))
            .unwrap_or_else(|| PathBuf::from("filtered_lossless"));
        Config {
            version: 1,
            source_dir,
            filtered_dir,
            player: PlayerKind::Mpv,
            mpv_socket: PathBuf::from("/tmp/jukebox-mpv.sock"),
            switch_sample_rate: true,
        }
    }

    pub fn load() -> Result<Option<Self>> {
        let p = config_path();
        if !p.exists() {
            return Ok(None);
        }
        let text = fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
        let cfg: Config = serde_yaml_compat(&text)
            .with_context(|| format!("parsing {}", p.display()))?;
        Ok(Some(cfg))
    }

    pub fn save(&self) -> Result<()> {
        let p = config_path();
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
            // 0700 on the dir
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).ok();
        }
        let text = format_yaml(self)?;
        fs::write(&p, text).with_context(|| format!("writing {}", p.display()))?;
        fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).ok();
        Ok(())
    }
}

/// Validate that a source dir exists and contains at least one .flac file.
pub fn validate_source_dir(p: &Path) -> Result<()> {
    if !p.is_dir() {
        return Err(anyhow!("source dir does not exist: {}", p.display()));
    }
    let has_flac = walkdir_has_flac(p);
    if !has_flac {
        return Err(anyhow!("source dir contains no .flac files: {}", p.display()));
    }
    Ok(())
}

fn walkdir_has_flac(root: &Path) -> bool {
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = match fs::read_dir(&d) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|s| s.to_str()).map(|s| s.eq_ignore_ascii_case("flac")).unwrap_or(false) {
                return true;
            }
        }
    }
    false
}

// ---- minimal YAML (de)serialization without pulling serde_yaml ----
// We hand-roll a tiny reader/writer for our 5-key flat config to avoid an extra
// dependency that is in maintenance mode. Format is fixed and simple.

fn serde_yaml_compat(text: &str) -> Result<Config> {
    let mut version = 1u32;
    let mut source_dir = PathBuf::new();
    let mut filtered_dir = PathBuf::new();
    let mut player = PlayerKind::Mpv;
    let mut mpv_socket = PathBuf::from("/tmp/jukebox-mpv.sock");
    let mut switch_sample_rate = true;
    for line in text.lines() {
        let line = line.split('#').next().unwrap().trim();
        if line.is_empty() { continue; }
        let (k, v) = match line.split_once(':') { Some(kv) => kv, None => continue };
        let k = k.trim();
        let v = v.trim().trim_matches('"');
        match k {
            "version" => version = v.parse().unwrap_or(1),
            "source_dir" => source_dir = PathBuf::from(v),
            "filtered_dir" => filtered_dir = PathBuf::from(v),
            "player" => player = if v == "afplay" { PlayerKind::Afplay } else { PlayerKind::Mpv },
            "mpv_socket" => mpv_socket = PathBuf::from(v),
            "switch_sample_rate" => switch_sample_rate = matches!(v, "true" | "yes" | "1"),
            _ => {}
        }
    }
    Ok(Config { version, source_dir, filtered_dir, player, mpv_socket, switch_sample_rate })
}

fn format_yaml(c: &Config) -> Result<String> {
    let player = match c.player { PlayerKind::Mpv => "mpv", PlayerKind::Afplay => "afplay" };
    Ok(format!(
        "# jukebox config — written by `jukebox`\n\
         version: {v}\n\
         source_dir: \"{s}\"\n\
         filtered_dir: \"{f}\"\n\
         player: {p}\n\
         mpv_socket: \"{m}\"\n\
         switch_sample_rate: {ssr}\n",
        v = c.version,
        s = c.source_dir.display(),
        f = c.filtered_dir.display(),
        p = player,
        m = c.mpv_socket.display(),
        ssr = c.switch_sample_rate,
    ))
}
