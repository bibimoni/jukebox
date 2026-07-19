use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    #[serde(rename = "pane.prefix")]
    PanePrefix,
    #[serde(rename = "pane.edit.toggle")]
    PaneEditToggle,
    #[serde(rename = "pane.focus_left")]
    PaneFocusLeft,
    #[serde(rename = "pane.focus_down")]
    PaneFocusDown,
    #[serde(rename = "pane.focus_up")]
    PaneFocusUp,
    #[serde(rename = "pane.focus_right")]
    PaneFocusRight,
    #[serde(rename = "pane.cycle_next")]
    PaneCycleNext,
    #[serde(rename = "pane.cycle_prev")]
    PaneCyclePrev,
    #[serde(rename = "navigation.view_artists")]
    NavigationViewArtists,
    #[serde(rename = "navigation.view_playlists")]
    NavigationViewPlaylists,
    #[serde(rename = "navigation.view_queue")]
    NavigationViewQueue,
    #[serde(rename = "navigation.view_youtube")]
    NavigationViewYoutube,
    #[serde(rename = "navigation.view_cycle_next")]
    NavigationViewCycleNext,
    #[serde(rename = "navigation.view_cycle_prev")]
    NavigationViewCyclePrev,
    #[serde(rename = "playback.resume_or_toggle")]
    PlaybackResumeOrToggle,
    #[serde(rename = "playback.play_selected")]
    PlaybackPlaySelected,
    #[serde(rename = "playback.next")]
    PlaybackNext,
    #[serde(rename = "playback.previous")]
    PlaybackPrevious,
    #[serde(rename = "playback.seek_back")]
    PlaybackSeekBack,
    #[serde(rename = "playback.seek_forward")]
    PlaybackSeekForward,
    #[serde(rename = "volume.up")]
    VolumeUp,
    #[serde(rename = "volume.down")]
    VolumeDown,
    #[serde(rename = "volume.mute_toggle")]
    VolumeMuteToggle,
    #[serde(rename = "shuffle.cycle")]
    ShuffleCycle,
    #[serde(rename = "shuffle.reshuffle")]
    ShuffleReshuffle,
    #[serde(rename = "repeat.cycle")]
    RepeatCycle,
    #[serde(rename = "continue.cycle")]
    ContinueCycle,
    #[serde(rename = "source_mode.cycle")]
    SourceModeCycle,
    #[serde(rename = "youtube.refresh_surface")]
    YoutubeRefreshSurface,
    #[serde(rename = "keybindings.open")]
    KeybindingsOpen,
    #[serde(rename = "help.open")]
    HelpOpen,
    #[serde(rename = "search.open")]
    SearchOpen,
    #[serde(rename = "command.open")]
    CommandOpen,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::PanePrefix => "pane.prefix",
            Action::PaneEditToggle => "pane.edit.toggle",
            Action::PaneFocusLeft => "pane.focus_left",
            Action::PaneFocusDown => "pane.focus_down",
            Action::PaneFocusUp => "pane.focus_up",
            Action::PaneFocusRight => "pane.focus_right",
            Action::PaneCycleNext => "pane.cycle_next",
            Action::PaneCyclePrev => "pane.cycle_prev",
            Action::NavigationViewArtists => "navigation.view_artists",
            Action::NavigationViewPlaylists => "navigation.view_playlists",
            Action::NavigationViewQueue => "navigation.view_queue",
            Action::NavigationViewYoutube => "navigation.view_youtube",
            Action::NavigationViewCycleNext => "navigation.view_cycle_next",
            Action::NavigationViewCyclePrev => "navigation.view_cycle_prev",
            Action::PlaybackResumeOrToggle => "playback.resume_or_toggle",
            Action::PlaybackPlaySelected => "playback.play_selected",
            Action::PlaybackNext => "playback.next",
            Action::PlaybackPrevious => "playback.previous",
            Action::PlaybackSeekBack => "playback.seek_back",
            Action::PlaybackSeekForward => "playback.seek_forward",
            Action::VolumeUp => "volume.up",
            Action::VolumeDown => "volume.down",
            Action::VolumeMuteToggle => "volume.mute_toggle",
            Action::ShuffleCycle => "shuffle.cycle",
            Action::ShuffleReshuffle => "shuffle.reshuffle",
            Action::RepeatCycle => "repeat.cycle",
            Action::ContinueCycle => "continue.cycle",
            Action::SourceModeCycle => "source_mode.cycle",
            Action::YoutubeRefreshSurface => "youtube.refresh_surface",
            Action::KeybindingsOpen => "keybindings.open",
            Action::HelpOpen => "help.open",
            Action::SearchOpen => "search.open",
            Action::CommandOpen => "command.open",
        }
    }

    pub fn editable_actions() -> &'static [Action] {
        &[
            Action::PaneEditToggle,
            Action::PaneFocusLeft,
            Action::PaneFocusDown,
            Action::PaneFocusUp,
            Action::PaneFocusRight,
            Action::PaneCycleNext,
            Action::PaneCyclePrev,
            Action::NavigationViewArtists,
            Action::NavigationViewPlaylists,
            Action::NavigationViewQueue,
            Action::NavigationViewYoutube,
            Action::NavigationViewCycleNext,
            Action::NavigationViewCyclePrev,
            Action::PlaybackPlaySelected,
            Action::PlaybackResumeOrToggle,
            Action::PlaybackNext,
            Action::PlaybackPrevious,
            Action::PlaybackSeekBack,
            Action::PlaybackSeekForward,
            Action::VolumeUp,
            Action::VolumeDown,
            Action::VolumeMuteToggle,
            Action::ShuffleCycle,
            Action::ShuffleReshuffle,
            Action::RepeatCycle,
            Action::ContinueCycle,
            Action::SourceModeCycle,
            Action::YoutubeRefreshSurface,
            Action::KeybindingsOpen,
            Action::HelpOpen,
            Action::SearchOpen,
            Action::CommandOpen,
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeySpec {
    pub code: KeyCodeSpec,
    #[serde(default)]
    pub modifiers: KeyModifierSpec,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyCodeSpec {
    Char(char),
    Space,
    Enter,
    Esc,
    Tab,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    F(u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyModifierSpec {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TuiPreferences {
    pub sidebar_visible: bool,
    pub player_bar_hidden: bool,
    pub pane_status_line_visible: bool,
}

impl Default for TuiPreferences {
    fn default() -> Self {
        Self {
            sidebar_visible: true,
            player_bar_hidden: false,
            pane_status_line_visible: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TuiConfig {
    pub version: u32,
    #[serde(default)]
    pub ui: TuiPreferences,
    #[serde(default)]
    pub keybindings: BTreeMap<String, Vec<String>>,
}

impl Default for TuiConfig {
    fn default() -> Self {
        serde_yaml::from_str(DEFAULT_TUI_YAML).expect("embedded tui.yaml parses")
    }
}

#[derive(Clone, Debug)]
pub struct Keymap {
    pub config: TuiConfig,
    bindings: HashMap<KeySpec, Action>,
    warnings: Vec<String>,
}

const DEFAULT_TUI_YAML: &str = include_str!("../../config/tui.yaml");
const DEFAULT_CONFIG_YAML: &str = include_str!("../../config/config.yaml");

pub fn config_base() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
        .unwrap_or_else(|| PathBuf::from("/tmp/.config"))
        .join("jukebox")
}

pub fn tui_config_path() -> PathBuf {
    config_base().join("tui.yaml")
}

pub fn default_config_yaml_path() -> PathBuf {
    config_base().join("config.yaml")
}

pub fn ensure_default_files() -> Result<()> {
    let dir = config_base();
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).ok();
    let config_path = default_config_yaml_path();
    if !config_path.exists() {
        fs::write(&config_path, DEFAULT_CONFIG_YAML)
            .with_context(|| format!("writing {}", config_path.display()))?;
    }
    let tui_path = tui_config_path();
    if !tui_path.exists() {
        fs::write(&tui_path, DEFAULT_TUI_YAML)
            .with_context(|| format!("writing {}", tui_path.display()))?;
    }
    Ok(())
}

impl Keymap {
    pub fn load_or_default() -> Self {
        let mut warnings = Vec::new();
        if let Err(err) = ensure_default_files() {
            warnings.push(format!("config init warning: {err}"));
        }
        let config = match fs::read_to_string(tui_config_path()) {
            Ok(text) => match serde_yaml::from_str::<TuiConfig>(&text) {
                Ok(config) => config,
                Err(err) => {
                    warnings.push(format!("tui.yaml malformed: {err}; using defaults"));
                    TuiConfig::default()
                }
            },
            Err(err) => {
                warnings.push(format!("could not read tui.yaml: {err}; using defaults"));
                TuiConfig::default()
            }
        };
        Self::from_config(config, warnings)
    }

    pub fn from_config(config: TuiConfig, mut warnings: Vec<String>) -> Self {
        let mut bindings = HashMap::new();
        let default_config = TuiConfig::default();
        for action in [
            Action::PanePrefix,
            Action::PaneEditToggle,
            Action::PaneFocusLeft,
            Action::PaneFocusDown,
            Action::PaneFocusUp,
            Action::PaneFocusRight,
            Action::PaneCycleNext,
            Action::PaneCyclePrev,
            Action::NavigationViewArtists,
            Action::NavigationViewPlaylists,
            Action::NavigationViewQueue,
            Action::NavigationViewYoutube,
            Action::NavigationViewCycleNext,
            Action::NavigationViewCyclePrev,
            Action::PlaybackResumeOrToggle,
            Action::PlaybackPlaySelected,
            Action::PlaybackNext,
            Action::PlaybackPrevious,
            Action::PlaybackSeekBack,
            Action::PlaybackSeekForward,
            Action::VolumeUp,
            Action::VolumeDown,
            Action::VolumeMuteToggle,
            Action::ShuffleCycle,
            Action::ShuffleReshuffle,
            Action::RepeatCycle,
            Action::ContinueCycle,
            Action::SourceModeCycle,
            Action::YoutubeRefreshSurface,
            Action::KeybindingsOpen,
            Action::HelpOpen,
            Action::SearchOpen,
            Action::CommandOpen,
        ] {
            let keys = if let Some(keys) = config.keybindings.get(action.label()) {
                keys
            } else if let Some(default_keys) = default_config.keybindings.get(action.label()) {
                warnings.push(format!("missing binding for {}", action.label()));
                default_keys
            } else {
                warnings.push(format!("missing default binding for {}", action.label()));
                continue;
            };
            for raw in keys {
                match parse_key_spec(raw) {
                    Ok(spec) => {
                        if let Some(previous) = bindings.insert(spec.clone(), action) {
                            warnings.push(format!(
                                "key {raw} rebinds {} over {}",
                                action.label(),
                                previous.label()
                            ));
                        }
                    }
                    Err(err) => warnings.push(format!("invalid key {raw:?}: {err}")),
                }
            }
        }
        Self {
            config,
            bindings,
            warnings,
        }
    }

    pub fn action_for(&self, key: KeyEvent) -> Option<Action> {
        self.bindings.get(&KeySpec::from(key)).copied()
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn bindings_for(&self, action: Action) -> Vec<String> {
        self.config
            .keybindings
            .get(action.label())
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_primary_binding(&mut self, action: Action, spec: KeySpec) {
        let label = action.label().to_string();
        self.config
            .keybindings
            .insert(label, vec![spec.to_string()]);
        let config = self.config.clone();
        *self = Self::from_config(config, Vec::new());
    }

    pub fn save(&self) -> Result<()> {
        ensure_default_files()?;
        let text = serde_yaml::to_string(&self.config)?;
        fs::write(tui_config_path(), text).with_context(|| "writing tui.yaml")
    }
}

impl From<KeyEvent> for KeySpec {
    fn from(key: KeyEvent) -> Self {
        let explicit_ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let control_char = match key.code {
            KeyCode::Char(c) if ('\u{1}'..='\u{1a}').contains(&c) => {
                Some(((c as u8 - 1) + b'a') as char)
            }
            _ => None,
        };
        let ctrl_letter = match key.code {
            KeyCode::Char(c) if explicit_ctrl && c.is_ascii_alphabetic() => {
                Some(c.to_ascii_lowercase())
            }
            _ => None,
        };
        let uppercase_char = matches!(key.code, KeyCode::Char(c) if c.is_ascii_uppercase());
        let back_tab = matches!(key.code, KeyCode::BackTab);
        Self {
            code: match key.code {
                KeyCode::Char(' ') => KeyCodeSpec::Space,
                KeyCode::Char(_) if control_char.is_some() => {
                    KeyCodeSpec::Char(control_char.unwrap())
                }
                KeyCode::Char(_) if ctrl_letter.is_some() => {
                    KeyCodeSpec::Char(ctrl_letter.unwrap())
                }
                KeyCode::Char(c) if uppercase_char => KeyCodeSpec::Char(c.to_ascii_lowercase()),
                KeyCode::Char(c) => KeyCodeSpec::Char(c),
                KeyCode::Enter => KeyCodeSpec::Enter,
                KeyCode::Esc => KeyCodeSpec::Esc,
                KeyCode::Tab | KeyCode::BackTab => KeyCodeSpec::Tab,
                KeyCode::Backspace => KeyCodeSpec::Backspace,
                KeyCode::Up => KeyCodeSpec::Up,
                KeyCode::Down => KeyCodeSpec::Down,
                KeyCode::Left => KeyCodeSpec::Left,
                KeyCode::Right => KeyCodeSpec::Right,
                KeyCode::F(n) => KeyCodeSpec::F(n),
                _ => KeyCodeSpec::Char('\0'),
            },
            modifiers: KeyModifierSpec {
                ctrl: explicit_ctrl || control_char.is_some(),
                alt: key.modifiers.contains(KeyModifiers::ALT),
                shift: (key.modifiers.contains(KeyModifiers::SHIFT) || uppercase_char || back_tab)
                    && !explicit_ctrl
                    && control_char.is_none(),
            },
        }
    }
}

impl std::fmt::Display for KeySpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();
        if self.modifiers.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.modifiers.alt {
            parts.push("Alt".to_string());
        }
        if self.modifiers.shift {
            parts.push("Shift".to_string());
        }
        parts.push(match self.code {
            KeyCodeSpec::Char(c) => c.to_string(),
            KeyCodeSpec::Space => "Space".to_string(),
            KeyCodeSpec::Enter => "Enter".to_string(),
            KeyCodeSpec::Esc => "Esc".to_string(),
            KeyCodeSpec::Tab => "Tab".to_string(),
            KeyCodeSpec::Backspace => "Backspace".to_string(),
            KeyCodeSpec::Up => "Up".to_string(),
            KeyCodeSpec::Down => "Down".to_string(),
            KeyCodeSpec::Left => "Left".to_string(),
            KeyCodeSpec::Right => "Right".to_string(),
            KeyCodeSpec::F(n) => format!("F{n}"),
        });
        write!(f, "{}", parts.join("+"))
    }
}

pub fn parse_key_spec(raw: &str) -> Result<KeySpec, String> {
    let normalized = raw.trim().replace(' ', "+");
    let mut modifiers = KeyModifierSpec::default();
    let mut code: Option<KeyCodeSpec> = None;
    for part in normalized.split('+').filter(|p| !p.is_empty()) {
        let lower = part.to_ascii_lowercase();
        match lower.as_str() {
            "ctrl" | "control" => modifiers.ctrl = true,
            "alt" | "option" | "opt" => modifiers.alt = true,
            "shift" => modifiers.shift = true,
            "space" => code = Some(KeyCodeSpec::Space),
            "enter" | "return" => code = Some(KeyCodeSpec::Enter),
            "esc" | "escape" => code = Some(KeyCodeSpec::Esc),
            "tab" => code = Some(KeyCodeSpec::Tab),
            "backspace" => code = Some(KeyCodeSpec::Backspace),
            "up" => code = Some(KeyCodeSpec::Up),
            "down" => code = Some(KeyCodeSpec::Down),
            "left" => code = Some(KeyCodeSpec::Left),
            "right" => code = Some(KeyCodeSpec::Right),
            fkey if fkey.starts_with('f') && fkey.len() > 1 => {
                let n = fkey[1..]
                    .parse::<u8>()
                    .map_err(|_| format!("unknown token {part}"))?;
                if n == 0 || n > 24 {
                    return Err(format!("function key out of range F{n}"));
                }
                code = Some(KeyCodeSpec::F(n));
            }
            _ if part.chars().count() == 1 => {
                let c = part.chars().next().unwrap();
                if c.is_ascii_uppercase() {
                    modifiers.shift = true;
                    code = Some(KeyCodeSpec::Char(c.to_ascii_lowercase()))
                } else {
                    code = Some(KeyCodeSpec::Char(c))
                }
            }
            other => return Err(format!("unknown token {other}")),
        }
    }
    Ok(KeySpec {
        code: code.ok_or_else(|| "missing key code".to_string())?,
        modifiers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_alt_h() {
        let spec = parse_key_spec("Alt+h").unwrap();
        assert!(spec.modifiers.alt);
        assert_eq!(spec.code, KeyCodeSpec::Char('h'));
    }

    #[test]
    fn default_r_maps_to_youtube_refresh() {
        let keymap = Keymap::from_config(TuiConfig::default(), Vec::new());
        let event = KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT);
        assert_eq!(
            keymap.action_for(event),
            Some(Action::YoutubeRefreshSurface)
        );
    }

    #[test]
    fn shifted_letters_normalize_to_lowercase_code() {
        let parsed = parse_key_spec("Shift+r").unwrap();
        let event = KeySpec::from(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT));

        assert_eq!(parsed, event);
        assert_eq!(parsed.code, KeyCodeSpec::Char('r'));
        assert!(parsed.modifiers.shift);
    }

    #[test]
    fn backtab_normalizes_to_shift_tab() {
        let parsed = parse_key_spec("Shift+Tab").unwrap();
        let event = KeySpec::from(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));

        assert_eq!(parsed, event);
        assert_eq!(parsed.code, KeyCodeSpec::Tab);
        assert!(parsed.modifiers.shift);
    }

    #[test]
    fn missing_new_binding_falls_back_to_default() {
        let mut config = TuiConfig::default();
        config.keybindings.remove(Action::HelpOpen.label());
        let keymap = Keymap::from_config(config, Vec::new());

        assert_eq!(
            keymap.action_for(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)),
            Some(Action::HelpOpen)
        );
        assert!(
            keymap
                .warnings()
                .iter()
                .any(|warning| warning.contains("missing binding for help.open")),
            "expected missing-binding warning, got {:?}",
            keymap.warnings()
        );
    }
}
