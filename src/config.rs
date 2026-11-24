//! Persistent Quottyall settings, stored in %APPDATA%\Quottyall\settings.json.
//!
//! The directory is the fork's own, not `%APPDATA%\Quotty`: an installed
//! original Quotty keeps its settings untouched, and in particular never tries
//! to parse a `family` value it does not know (`MiniMax`/`DeepSeek`), which
//! would make it fall back to defaults and lose the user's preferences.

use crate::providers::Family;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// Per-user application directory. Shared by the settings file, the DPAPI key
/// store and the DeepSeek baselines.
const APP_DIR: &str = "Quottyall";

/// Whether the MiniMax key is expected at the mainland-China platform. Kept in
/// a static because the provider runs on the poll thread, which must not read
/// `settings.json` once per poll just to learn the region.
static MINIMAX_CN: AtomicBool = AtomicBool::new(false);

/// Region the MiniMax fetch tries first: `"global"` or `"cn"`.
pub fn minimax_region() -> &'static str {
    if MINIMAX_CN.load(Ordering::Relaxed) {
        "cn"
    } else {
        "global"
    }
}

pub fn set_minimax_region(region: &str) {
    MINIMAX_CN.store(region.eq_ignore_ascii_case("cn"), Ordering::Relaxed);
}

/// What the strip's header line shows.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderMode {
    /// Environment + plan, e.g. "Claude Max 20×".
    Full,
    /// Family only, e.g. "Claude".
    FamilyOnly,
    Hidden,
}

/// How much room the strip takes. Smaller designs lose height and width: the
/// text moves next to a shorter bar instead of sitting on a line of its own,
/// and each step drops the least useful part of it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripSize {
    /// The original design: title + reset line over a full-width bar.
    Normal,
    /// One line per limit: short title · bar · percent + countdown.
    Mini,
    /// Bar + percent + countdown only, no titles.
    Nano,
}

/// Which edge of the strip stays put when its height changes (a row appears or
/// goes, or the design changes).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// The strip grows and shrinks downwards from its top edge.
    Top,
    /// The strip grows and shrinks upwards from its bottom edge.
    Bottom,
}

/// Which family the strip shows.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveMode {
    /// Follow the foreground window — whichever tool was used last.
    Auto,
    /// Always show `Settings::family`.
    Pinned,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    /// Window opacity, 0.15..=1.0
    pub opacity: f32,
    /// Last window position (screen points, top-left). None = let the OS place it.
    pub pos: Option<(f32, f32)>,
    /// Bottom edge of the window at the last drag, for `Anchor::Bottom`.
    pub pos_bottom: Option<f32>,
    /// Edge that stays put when the height changes.
    pub anchor: Anchor,
    /// Poll interval in seconds.
    pub poll_secs: u64,
    /// Bubble animation on/off (off = lowest CPU).
    pub animate: bool,
    /// Header line content.
    pub header_mode: HeaderMode,
    /// How compact the strip is drawn.
    pub strip_size: StripSize,
    /// Keep the strip off screen while none of the enabled tools is running —
    /// there is nothing to watch when nothing is spending quota.
    pub hide_when_idle: bool,
    pub claude_enabled: bool,
    pub codex_enabled: bool,
    pub antigravity_enabled: bool,
    pub minimax_enabled: bool,
    pub deepseek_enabled: bool,
    /// Keep this family on the strip whatever the foreground window is.
    /// MiniMax and DeepSeek have no desktop app for the auto mode to
    /// recognise, so this is how their strip gets on screen: pinned manually,
    /// independent of the active application.
    pub pinned_family: Option<Family>,
    /// Which MiniMax platform the key belongs to: "global" (minimax.io) or
    /// "cn" (api.minimaxi.com). Keys are region-bound; the fetch falls back to
    /// the other host once before giving up.
    pub minimax_region: String,
    pub active_mode: ActiveMode,
    /// Pinned family, and the one restored at startup.
    pub family: Family,
    /// Verbose, anonymised logging next to the exe. Off unless asked for.
    pub diagnostics: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            opacity: 0.8,
            pos: None,
            pos_bottom: None,
            anchor: Anchor::Bottom,
            poll_secs: 60,
            animate: true,
            header_mode: HeaderMode::Full,
            strip_size: StripSize::Normal,
            hide_when_idle: true,
            claude_enabled: true,
            codex_enabled: true,
            antigravity_enabled: true,
            // Off until a key is entered: an enabled source that cannot fetch
            // would otherwise report "ключ не задан" on first start.
            minimax_enabled: false,
            deepseek_enabled: false,
            pinned_family: None,
            minimax_region: "global".into(),
            active_mode: ActiveMode::Auto,
            family: Family::Claude,
            diagnostics: false,
        }
    }
}

/// Where `settings.json` may live. `dirs::config_dir()` alone is not enough:
/// depending on how the exe was started (shortcut, Startup folder, shell) it has
/// been seen to resolve to something other than the real Roaming directory, and
/// the app would then silently start from defaults every login.
fn candidate_paths() -> Vec<PathBuf> {
    let mut dirs_list: Vec<PathBuf> = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        dirs_list.push(PathBuf::from(appdata));
    }
    if let Some(cfg) = dirs::config_dir() {
        dirs_list.push(cfg);
    }
    if let Ok(profile) = std::env::var("USERPROFILE") {
        dirs_list.push(PathBuf::from(profile).join("AppData").join("Roaming"));
    }
    if let Some(home) = dirs::home_dir() {
        dirs_list.push(home.join("AppData").join("Roaming"));
    }
    dirs_list.dedup();
    dirs_list
        .into_iter()
        .map(|d| d.join(APP_DIR).join("settings.json"))
        .collect()
}

impl Settings {
    /// Directory holding `settings.json` and `minimax.key`. Resolved from the
    /// settings file when one exists, so the encrypted key always sits next to
    /// the settings that describe it.
    pub(crate) fn data_dir() -> Option<PathBuf> {
        if let Some(p) = Self::path() {
            if let Some(dir) = p.parent() {
                return Some(dir.to_path_buf());
            }
        }
        candidate_paths()
            .into_iter()
            .next()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    }

    /// Resolved once: an existing file wins wherever it is, so load and save
    /// can never end up on different paths.
    fn path() -> Option<PathBuf> {
        static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
        PATH.get_or_init(|| {
            let candidates = candidate_paths();
            candidates
                .iter()
                .find(|p| std::fs::metadata(p).is_ok())
                .cloned()
                .or_else(|| candidates.into_iter().next())
        })
        .clone()
    }

    pub fn load() -> Self {
        let path = Self::path();
        let raw = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok());
        // Falling back to defaults silently would look like "the app forgot
        // everything", so say why in the debug log.
        let mut s: Settings = match &raw {
            None => {
                crate::providers::dbg_log(&format!(
                    "settings: nothing readable among {:?}",
                    candidate_paths()
                ));
                Settings::default()
            }
            // Windows editors and PowerShell's `Set-Content -Encoding UTF8`
            // put a BOM in front, which JSON parsers reject outright.
            Some(text) => match serde_json::from_str(text.trim_start_matches('\u{feff}')) {
                Ok(parsed) => parsed,
                Err(e) => {
                    crate::providers::dbg_log(&format!("settings: {path:?} unparseable: {e}"));
                    // Keep the unreadable file instead of letting the next save
                    // silently overwrite settings this build cannot read (a
                    // newer version's file, or a hand-edited one).
                    if let Some(p) = &path {
                        let backup = p.with_extension("json.broken");
                        let _ = std::fs::copy(p, backup);
                    }
                    Settings::default()
                }
            },
        };
        s.opacity = s.opacity.clamp(0.15, 1.0);
        if s.poll_secs < 15 {
            s.poll_secs = 15;
        }
        // Never leave the user with nothing to show.
        if s.enabled_mask() == 0 {
            s.claude_enabled = true;
        }
        if !s.enabled(s.family) {
            s.family = s.first_enabled();
        }
        // A pin on a switched-off source would leave an empty strip; the pin
        // only means something while its provider is enabled.
        if s.pinned_family.is_some_and(|f| !s.enabled(f)) {
            s.pinned_family = None;
        }
        set_minimax_region(&s.minimax_region);
        s
    }

    pub fn save(&self) {
        set_minimax_region(&self.minimax_region);
        if let Some(p) = Self::path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(raw) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(p, raw);
            }
        }
    }

    pub fn enabled(&self, f: Family) -> bool {
        match f {
            Family::Claude => self.claude_enabled,
            Family::Codex => self.codex_enabled,
            Family::Antigravity => self.antigravity_enabled,
            Family::MiniMax => self.minimax_enabled,
            Family::DeepSeek => self.deepseek_enabled,
        }
    }

    pub fn set_enabled(&mut self, f: Family, on: bool) {
        match f {
            Family::Claude => self.claude_enabled = on,
            Family::Codex => self.codex_enabled = on,
            Family::Antigravity => self.antigravity_enabled = on,
            Family::MiniMax => self.minimax_enabled = on,
            Family::DeepSeek => self.deepseek_enabled = on,
        }
        if self.enabled_mask() == 0 {
            // Refuse to turn the last one off.
            self.set_enabled(f, true);
        }
        if self.pinned_family == Some(f) && !on {
            self.pinned_family = None;
        }
        if !self.enabled(self.family) {
            self.family = self.first_enabled();
        }
    }

    pub fn first_enabled(&self) -> Family {
        Family::ALL
            .into_iter()
            .find(|f| self.enabled(*f))
            .unwrap_or(Family::Claude)
    }

    /// Bitmask of enabled families, as handed to the poller thread.
    pub fn enabled_mask(&self) -> u8 {
        Family::ALL
            .into_iter()
            .filter(|f| self.enabled(*f))
            .fold(0u8, |m, f| m | (1 << f.idx()))
    }
}
