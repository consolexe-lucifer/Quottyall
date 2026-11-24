//! Which tool family the user is working in right now.
//!
//! The rule is "whatever was in the foreground last": we look at the foreground
//! window's process, and — when that process is only a host (a terminal, or an
//! editor with an embedded terminal) — at what it is running underneath, so a
//! CLI session counts as its own family.
//!
//! Harnesses (an app that can host sessions of several providers) are handled
//! separately: the process, not the window title, decides. A window title is
//! free text — it is whatever the user called the session, or the name of the
//! project they are working on — so it is never read for provider selection.
//! Only structured metadata is trusted: an explicit `--model`/`--provider`
//! argument, then the harness's own brand. When neither names exactly one
//! provider, the detector stays ambiguous and the caller keeps the last family
//! (manual pinning is the explicit escape hatch).

use crate::providers::Family;
use crate::winproc;

/// Exe name → family, for windows that *are* the tool.
fn direct(exe: &str) -> Option<Family> {
    match exe {
        // Linux names come from the kernel's `comm`: no extension, and no space.
        "claude.exe" | "claude" => Some(Family::Claude),
        "codex.exe" | "chatgpt.exe" | "codex" => Some(Family::Codex),
        "antigravity.exe"
        | "antigravity.exe.real"
        | "antigravity ide.exe"
        | "antigravity"
        | "antigravity-ide" => Some(Family::Antigravity),
        // MiniMax Code is MiniMax's own desktop app, and `mmx` is its CLI:
        // both identify the provider unambiguously.
        "minimax.exe" | "minimax code.exe" | "minimax-code.exe" | "minimax code" | "mmx.exe"
        | "mmx" | "minimax" => Some(Family::MiniMax),
        _ => None,
    }
}

/// Does the executable host sessions of several providers instead of being one
/// tool? The DeepSeek Harness desktop app is the case that matters here: its
/// name says DeepSeek, but a session inside it may be running another model.
fn is_harness(exe: &str) -> bool {
    exe.contains("harness")
}

/// A provider named by a piece of *structured* metadata (a flag value or a
/// model id), never by prose. Returns `None` when the text names no provider or
/// names more than one — both mean "cannot tell".
fn provider_named(text: &str) -> Option<Family> {
    let t = text.to_lowercase();
    let mut hits: Vec<Family> = Vec::new();
    let mut add = |f: Family| {
        if !hits.contains(&f) {
            hits.push(f);
        }
    };
    if t.contains("minimax") {
        add(Family::MiniMax);
    }
    if t.contains("deepseek") {
        add(Family::DeepSeek);
    }
    if t.contains("claude") || t.contains("anthropic") {
        add(Family::Claude);
    }
    if t.contains("codex") || t.contains("chatgpt") || t.contains("openai") {
        add(Family::Codex);
    }
    if t.contains("antigravity") {
        add(Family::Antigravity);
    }
    match hits.len() {
        1 => Some(hits[0]),
        _ => None,
    }
}

/// Provider flags a harness may be started with, in `--flag=value` or
/// `--flag value` form.
const HARNESS_FLAGS: [&str; 6] = [
    "--model",
    "--provider",
    "--provider-id",
    "-m",
    "model",
    "provider",
];

/// Which family a harness window belongs to.
///
/// `cmdline` is the process command line (structured metadata). The window title
/// is deliberately not a parameter: it is user free text, and guessing from it
/// is exactly what must not happen.
///
/// Order: an explicit flag wins; without one, the harness brand does. Several
/// different providers named by flags is ambiguous — no guess.
fn harness_family(exe: &str, cmdline: &str) -> Option<Family> {
    let mut found: Vec<Family> = Vec::new();
    let mut push = |f: Family| {
        if !found.contains(&f) {
            found.push(f);
        }
    };

    // Split on whitespace and take each flag together with its value, in both
    // `--flag=value` and `--flag value` shapes.
    let args: Vec<&str> = cmdline.split_whitespace().collect();
    for (i, arg) in args.iter().enumerate() {
        let lower = arg.to_lowercase();
        for flag in HARNESS_FLAGS {
            if let Some(value) = lower.strip_prefix(flag).and_then(|r| r.strip_prefix('=')) {
                if let Some(f) = provider_named(value) {
                    push(f);
                }
            } else if lower == flag {
                if let Some(next) = args.get(i + 1) {
                    if let Some(f) = provider_named(next) {
                        push(f);
                    }
                }
            }
        }
    }

    match found.len() {
        1 => return found.into_iter().next(),
        // Contradictory metadata: keep the last family instead of flipping.
        n if n > 1 => return None,
        _ => {}
    }

    // No flag: the harness's own brand, e.g. "DeepSeek Harness.exe" or
    // "MiniMax Harness.exe". This is the application's identity, not a window
    // title, and a generic "harness.exe" yields `None` (→ keep the last family).
    provider_named(&exe.replace("harness", " "))
}

/// Console hosts own the window but not the CLI: `conhost.exe` is a *child* of
/// the program whose console it draws, so the search has to start one level up.
fn is_console_host(exe: &str) -> bool {
    matches!(exe, "conhost.exe" | "openconsole.exe")
}

/// Windows that merely *host* a CLI: look at their process tree instead.
fn is_host(exe: &str) -> bool {
    matches!(
        exe,
        "windowsterminal.exe"
            | "windowsterminalpreview.exe"
            | "openconsole.exe"
            | "conhost.exe"
            | "cmd.exe"
            | "powershell.exe"
            | "pwsh.exe"
            | "bash.exe"
            | "sh.exe"
            | "mintty.exe"
            | "alacritty.exe"
            | "wezterm-gui.exe"
            | "hyper.exe"
            | "tabby.exe"
            | "kitty.exe"
            | "warp.exe"
            | "conemu64.exe"
            | "cmder.exe"
            | "code.exe"
            | "cursor.exe"
            | "windsurf.exe"
    )
}

/// Remembers the last answer so we only walk the process list when the
/// foreground window changes (or a host window may have started a new CLI).
pub struct Detector {
    last_pid: u32,
    last: Option<Family>,
    /// Set when the answer came from a process-tree walk — those go stale
    /// without the foreground window ever changing (`claude` started in the
    /// terminal you are already looking at), so they are re-checked on a timer.
    from_tree: bool,
    checked_at: f64,
    /// Cache behind `running()`.
    running: u8,
    running_at: f64,
}

/// How long the "which tools are running" answer is reused. Walking the process
/// list is cheap but not free, and a strip that appears a couple of seconds
/// after the tool did costs nothing.
const RUNNING_TTL: f64 = 3.0;

impl Default for Detector {
    fn default() -> Self {
        Self {
            last_pid: 0,
            last: None,
            from_tree: false,
            checked_at: f64::MIN,
            running: 0,
            running_at: f64::MIN,
        }
    }
}

impl Detector {
    /// `now` is any monotonically growing clock in seconds (egui's frame time).
    /// Returns `None` when the foreground window belongs to nothing we track —
    /// the caller then keeps showing the previous family.
    pub fn poll(&mut self, now: f64) -> Option<Family> {
        let pid = foreground_pid()?;
        let stale = self.from_tree && now - self.checked_at > 2.0;
        if pid == self.last_pid && !stale {
            return self.last;
        }

        let exe = process_name(pid)?.to_lowercase();
        let (family, from_tree) = match direct(&exe) {
            Some(f) => (Some(f), false),
            // A harness can host any provider, so its identity alone is not an
            // answer: ask the structured metadata, and stay undecided when that
            // does not name exactly one tool.
            None if is_harness(&exe) => {
                let cmdline = winproc::command_line(pid).unwrap_or_default();
                (harness_family(&exe, &cmdline), false)
            }
            None if is_host(&exe) => (family_in_tree(pid, is_console_host(&exe)), true),
            None => (None, false),
        };

        self.last_pid = pid;
        self.last = family;
        self.from_tree = from_tree;
        self.checked_at = now;
        family
    }

    /// Bitmask over `Family::idx` of the families whose program is running at
    /// all — app, IDE or CLI, foreground or not. Unlike `poll`, this asks "is
    /// there anything to watch on this machine right now".
    pub fn running(&mut self, now: f64) -> u8 {
        if now - self.running_at <= RUNNING_TTL {
            return self.running;
        }
        self.running_at = now;
        self.running = winproc::snapshot()
            .iter()
            .filter_map(|p| {
                direct(&p.name).or_else(|| {
                    // A running harness keeps its provider's strip on screen in
                    // auto mode; a generic one that cannot be identified does
                    // not count as "running" for anyone.
                    is_harness(&p.name)
                        .then(|| harness_family(&p.name, ""))
                        .flatten()
                })
            })
            .fold(0u8, |mask, f| mask | 1 << f.idx());
        self.running
    }
}

// ---------------------------------------------------------------------------
// Win32
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn foreground_pid() -> Option<u32> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        (pid != 0).then_some(pid)
    }
}

fn process_name(pid: u32) -> Option<String> {
    winproc::name_of(pid)
}

/// Walk everything started under `root` and return the family of the most
/// recently started match (highest pid — good enough to prefer the CLI you just
/// launched over one sitting in another tab). With `hop_parent`, the walk starts
/// at the root's parent instead: a console host's CLI is its sibling, not its
/// child.
fn family_in_tree(root: u32, hop_parent: bool) -> Option<Family> {
    let procs = winproc::snapshot();
    let root = if hop_parent {
        procs
            .iter()
            .find(|p| p.pid == root)
            .map(|p| p.parent)
            .filter(|p| *p != 0)
            .unwrap_or(root)
    } else {
        root
    };

    // Breadth-first over the descendants of `root`.
    let mut frontier = vec![root];
    let mut seen = vec![root];
    let mut best: Option<(u32, Family)> = None;
    while let Some(parent) = frontier.pop() {
        for p in &procs {
            if p.parent != parent || seen.contains(&p.pid) {
                continue;
            }
            seen.push(p.pid);
            frontier.push(p.pid);
            if let Some(f) = direct(&p.name) {
                if best.map_or(true, |(bp, _)| p.pid > bp) {
                    best = Some((p.pid, f));
                }
            }
        }
        // A runaway tree would only cost us time; the process list is finite.
        if seen.len() > 4096 {
            break;
        }
    }
    best.map(|(_, f)| f)
}

/// X11: the window manager names the active window in `_NET_ACTIVE_WINDOW`, and
/// `_NET_WM_PID` gives its process. Wayland exposes no such thing to a client, so
/// there the answer stays `None` and the caller keeps the previous family.
#[cfg(target_os = "linux")]
fn foreground_pid() -> Option<u32> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    let atom = |name: &str| -> Option<u32> {
        Some(
            conn.intern_atom(false, name.as_bytes())
                .ok()?
                .reply()
                .ok()?
                .atom,
        )
    };
    let active = atom("_NET_ACTIVE_WINDOW")?;
    let wm_pid = atom("_NET_WM_PID")?;

    let win = conn
        .get_property(false, root, active, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()?
        .next()?;
    let pid = conn
        .get_property(false, win, wm_pid, AtomEnum::CARDINAL, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()?
        .next()?;
    (pid != 0).then_some(pid)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn foreground_pid() -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_known_processes() {
        assert_eq!(direct("claude.exe"), Some(Family::Claude));
        assert_eq!(direct("codex.exe"), Some(Family::Codex));
        assert_eq!(direct("chatgpt.exe"), Some(Family::Codex));
        assert_eq!(direct("antigravity.exe"), Some(Family::Antigravity));
        assert_eq!(direct("antigravity.exe.real"), Some(Family::Antigravity));
        assert_eq!(direct("antigravity ide.exe"), Some(Family::Antigravity));
        // MiniMax's own desktop app and CLI.
        assert_eq!(direct("minimax code.exe"), Some(Family::MiniMax));
        assert_eq!(direct("minimax.exe"), Some(Family::MiniMax));
        assert_eq!(direct("mmx.exe"), Some(Family::MiniMax));
        // The agy CLI keeps its CSRF token in memory only, so its quota is unreadable.
        assert_eq!(direct("agy.exe"), None);
        assert_eq!(direct("notepad.exe"), None);
        assert_eq!(direct("explorer.exe"), None);
        // A harness is not a tool: it must be classified by metadata instead.
        assert_eq!(direct("deepseek harness.exe"), None);
        assert_eq!(direct("harness.exe"), None);
    }

    #[test]
    fn an_unknown_process_is_not_guessed() {
        // Requirement: an unrecognised foreground window changes nothing.
        assert_eq!(direct("sublime_text.exe"), None);
        assert_eq!(harness_family("notepad.exe", ""), None);
    }

    #[test]
    fn a_branded_harness_keeps_its_provider() {
        // The application's identity, not a window title.
        assert_eq!(
            harness_family("deepseek harness.exe", ""),
            Some(Family::DeepSeek)
        );
        assert_eq!(
            harness_family("minimax harness.exe", ""),
            Some(Family::MiniMax)
        );
        // A generic harness names no provider: the caller must keep the last
        // family rather than flip to something arbitrary.
        assert_eq!(harness_family("harness.exe", ""), None);
        assert_eq!(harness_family("dsh.exe", ""), None);
    }

    #[test]
    fn structured_metadata_decides_when_present() {
        // Second harness running MiniMax under a DeepSeek-branded launcher: the
        // explicit flag wins over the brand.
        assert_eq!(
            harness_family("deepseek harness.exe", "--model MiniMax-M3"),
            Some(Family::MiniMax)
        );
        assert_eq!(
            harness_family("harness.exe", "--provider=deepseek"),
            Some(Family::DeepSeek)
        );
        assert_eq!(
            harness_family("harness.exe", "--model=claude-opus-4"),
            Some(Family::Claude)
        );
        assert_eq!(
            harness_family("harness.exe", "--provider codex"),
            Some(Family::Codex)
        );
        assert_eq!(
            harness_family("harness.exe", "--model=antigravity"),
            Some(Family::Antigravity)
        );
    }

    #[test]
    fn contradictory_metadata_stays_undecided() {
        // Both providers in one harness and the flags name both: never pick one.
        assert_eq!(
            harness_family("harness.exe", "--model minimax --provider deepseek"),
            None
        );
        assert_eq!(
            harness_family(
                "deepseek harness.exe",
                "--model MiniMax-M3 --model DeepSeek-V4"
            ),
            None
        );
    }

    #[test]
    fn a_window_title_is_never_used_to_pick_a_provider() {
        // `harness_family` takes the *process command line* only. Free text that
        // merely mentions a provider (a session name, a project folder) cannot
        // reach it, which is what keeps a project named "MiniMax migration"
        // from silently switching the strip.
        assert_eq!(harness_family("harness.exe", "MiniMax"), None);
        assert_eq!(
            harness_family("harness.exe", "D:\\work\\minimax-gateway\\session"),
            None
        );
        // A generic harness with no flags stays undecided even though the
        // launcher's own name mentions nothing either.
        assert_eq!(
            harness_family("harness.exe", "--user-data-dir=C:\\tmp"),
            None
        );
    }

    #[test]
    fn provider_names_are_matched_by_family_not_by_brand_only() {
        assert_eq!(provider_named("MiniMax-M3"), Some(Family::MiniMax));
        assert_eq!(provider_named("deepseek-v4-flash"), Some(Family::DeepSeek));
        assert_eq!(provider_named("gpt-5-codex"), Some(Family::Codex));
        assert_eq!(
            provider_named("gemini-3-pro"),
            None,
            "gemini alone is not a family"
        );
        assert_eq!(provider_named("unknown-model"), None);
        // A string naming two providers is ambiguous.
        assert_eq!(provider_named("minimax/deepseek"), None);
    }
}
