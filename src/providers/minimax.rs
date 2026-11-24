//! MiniMax module: reads the MiniMax **M Plan** subscription quota — the plan
//! the app calls "MiniMax Go", after its entry tier — straight from the
//! official Token Plan endpoint the MiniMax console itself uses.
//!
//! Everything here talks to a *documented* endpoint over HTTPS with the user's
//! own M Plan API key:
//!
//! ```text
//! GET https://www.minimax.io/v1/token_plan/remains
//! Authorization: Bearer <M Plan API key>
//! ```
//!
//! confirmed by MiniMax's M Plan FAQ ("Method 3: Use the API endpoint"), which
//! also fixes the meaning of the two windows: text models are limited by a
//! 5-hour window *and* a weekly one, image/video models by the weekly window
//! only. Both windows start with the first use and reset independently.
//!
//! Four traps the parser handles explicitly, because getting any of them wrong
//! shows the user a confidently false number (see `docs/minimax-api.md` for the
//! evidence behind each one):
//!
//! 1. `*_remaining_percent` is **remaining**, so the bar needs `100 - percent`.
//! 2. `current_*_usage_count` is also **remaining**, despite the word "usage"
//!    (MiniMax-AI/MiniMax-M2#99; OpenClaw and OpenTokenUsage ship the same
//!    correction). Only used when the percent field is missing.
//! 3. Timestamps are **epoch milliseconds**, not seconds — and `remains_time`
//!    is milliseconds too.
//! 4. Auth failures arrive as **HTTP 200** with a non-zero `base_resp`; the
//!    status line proves nothing.
//!
//! No key, no subscription, an unreachable host: each maps to a specific
//! message. Nothing is ever filled in with a made-up value — a window whose
//! numbers cannot be established is left out of the snapshot.

use super::{dbg_log, diag, Family, FetchError, Limit, LimitWindow, Snapshot};
use crate::{config, secret};
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use serde_json::Value;

/// Global (minimax.io) host — where M Plan API keys from the international
/// platform answer.
const GLOBAL_URL: &str = "https://www.minimax.io/v1/token_plan/remains";
/// Mainland-China host. MiniMax keys are region-bound: a CN key answers
/// `2049`/`1004` on the global host, so the fetch retries here before reporting
/// an auth failure.
const CN_URL: &str = "https://api.minimaxi.com/v1/token_plan/remains";

/// The 5-hour window, in seconds. Fallback only: the response carries its own
/// `start_time`/`end_time`, which are used when both are present.
const SESSION_WINDOW_SECS: i64 = 5 * 3600;
/// The weekly window: seven days from first use. Same fallback rule.
const WEEKLY_WINDOW_SECS: i64 = 7 * 24 * 3600;

/// Rows on screen, named exactly as the user sees them in the MiniMax console.
const SESSION_TITLE: &str = "5 Hours";
const WEEKLY_TITLE: &str = "Weekly";

/// Static label. The API response carries **no** plan/tier field, so the tier
/// must come from the app rather than be guessed ("Go" is what this app is
/// asked to show; `docs/minimax-api.md` records the check).
const PLAN_LABEL: &str = "MiniMax Go";

/// `current_interval_status` / `current_weekly_status`: the weekly quota is
/// used up. Still a real window — show it at 100 %.
const STATUS_EXHAUSTED: f64 = 2.0;
/// The plan simply has no weekly limit (`∞`); there is no window to draw.
const STATUS_NO_WEEKLY_LIMIT: f64 = 3.0;

/// MiniMax timestamps are epoch milliseconds. Accept seconds as well: the
/// legacy `coding_plan` responses and third-party mirrors have shipped both,
/// and a ten-digit value cannot be mistaken for a millisecond one.
fn epoch_to_utc(v: f64) -> Option<DateTime<Utc>> {
    if !v.is_finite() || v <= 0.0 {
        return None;
    }
    let secs = if v > 1e11 { v / 1000.0 } else { v };
    Utc.timestamp_opt(secs as i64, 0).single()
}

/// A number that may arrive as a JSON number or as a string. Absent, null,
/// booleans and junk all mean "not reported".
fn as_f64(v: Option<&Value>) -> Option<f64> {
    let v = v?;
    let n = match v {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    n.is_finite().then_some(n)
}

fn as_text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        }
        _ => None,
    }
}

/// One entry of `model_remains[]`.
///
/// `general` is the bucket that covers text/speech — the one that carries both
/// windows. `video` is a weekly-only product bucket (its own interval is 24 h),
/// so it is never shown as "the" MiniMax quota: that would be wrong for anyone
/// not generating video.
#[derive(Deserialize, Debug, Default)]
#[serde(default)]
struct ModelRemains {
    model_name: Option<Value>,

    current_interval_total_count: Option<Value>,
    current_interval_usage_count: Option<Value>,
    current_interval_remaining_count: Option<Value>,
    current_interval_remains_count: Option<Value>,
    current_interval_remaining_percent: Option<Value>,
    current_interval_status: Option<Value>,

    current_weekly_total_count: Option<Value>,
    current_weekly_usage_count: Option<Value>,
    current_weekly_remaining_count: Option<Value>,
    current_weekly_remaining_percent: Option<Value>,
    current_weekly_status: Option<Value>,

    start_time: Option<Value>,
    end_time: Option<Value>,
    weekly_start_time: Option<Value>,
    weekly_end_time: Option<Value>,
    /// Milliseconds left in the interval window (fallback for `end_time`).
    remains_time: Option<Value>,
    /// Milliseconds left in the weekly window (fallback for `weekly_end_time`).
    weekly_remains_time: Option<Value>,

    /// Plan hints that some deployments return. Never required, never guessed.
    plan_name: Option<Value>,
    current_subscribe_title: Option<Value>,
    plan: Option<Value>,
}

impl ModelRemains {
    fn name(&self) -> String {
        as_text(self.model_name.as_ref()).unwrap_or_default()
    }

    fn is_video(&self) -> bool {
        self.name().eq_ignore_ascii_case("video")
    }

    fn status(&self, which: Option<&Value>) -> Option<f64> {
        as_f64(which)
    }

    fn plan_label(&self) -> Option<String> {
        as_text(self.plan_name.as_ref())
            .or_else(|| as_text(self.current_subscribe_title.as_ref()))
            .or_else(|| as_text(self.plan.as_ref()))
    }

    /// How many windows this entry can actually fill — used to pick the best
    /// entry when there is no `general` bucket.
    fn completeness(&self, now: DateTime<Utc>) -> usize {
        usize::from(self.session_window(now).is_some())
            + usize::from(self.weekly_window(now).is_some())
    }

    fn session_window(&self, now: DateTime<Utc>) -> Option<Limit> {
        let exhausted =
            self.status(self.current_interval_status.as_ref()) == Some(STATUS_EXHAUSTED);
        let used = used_percent(
            as_f64(self.current_interval_remaining_percent.as_ref()),
            as_f64(self.current_interval_usage_count.as_ref()),
            as_f64(self.current_interval_remaining_count.as_ref())
                .or_else(|| as_f64(self.current_interval_remains_count.as_ref())),
            as_f64(self.current_interval_total_count.as_ref()),
        );
        let used = match (used, exhausted) {
            (_, true) => 100.0,
            (Some(u), false) => u,
            (None, false) => return None,
        };
        Some(Limit {
            title: SESSION_TITLE.to_string(),
            used_percent: used,
            window: reset_window(
                as_f64(self.end_time.as_ref()),
                as_f64(self.start_time.as_ref()),
                as_f64(self.remains_time.as_ref()),
                SESSION_WINDOW_SECS,
                now,
            ),
        })
    }

    fn weekly_window(&self, now: DateTime<Utc>) -> Option<Limit> {
        let status = self.status(self.current_weekly_status.as_ref());
        // `3` means the plan has no weekly limit at all: there is no window to
        // draw, and a permanent 100 % bar would read as "quota left".
        if status == Some(STATUS_NO_WEEKLY_LIMIT) {
            return None;
        }
        let used = used_percent(
            as_f64(self.current_weekly_remaining_percent.as_ref()),
            as_f64(self.current_weekly_usage_count.as_ref()),
            as_f64(self.current_weekly_remaining_count.as_ref()),
            as_f64(self.current_weekly_total_count.as_ref()),
        );
        let used = match (used, status == Some(STATUS_EXHAUSTED)) {
            (_, true) => 100.0,
            (Some(u), false) => u,
            (None, false) => return None,
        };
        Some(Limit {
            title: WEEKLY_TITLE.to_string(),
            used_percent: used,
            window: reset_window(
                as_f64(self.weekly_end_time.as_ref()),
                as_f64(self.weekly_start_time.as_ref()),
                as_f64(self.weekly_remains_time.as_ref()),
                WEEKLY_WINDOW_SECS,
                now,
            ),
        })
    }
}

/// Percent of the window consumed, 0..=100.
///
/// `remaining_percent` is authoritative when present. Otherwise the counts are
/// used, with `usage_count` read as **remaining** (see module docs) and
/// `remaining_count` as the explicitly named alias. If the "counts" are
/// implausible as remaining — a value above the total — they are read as
/// consumed instead, which keeps a differently-implemented server honest
/// instead of pinning the bar at "nothing used".
fn used_percent(
    remaining_percent: Option<f64>,
    usage_count: Option<f64>,
    remaining_count: Option<f64>,
    total: Option<f64>,
) -> Option<f32> {
    if let Some(p) = remaining_percent {
        return Some((100.0 - p).clamp(0.0, 100.0) as f32);
    }
    let total = total.filter(|t| *t > 0.0)?;
    if let Some(remaining) = remaining_count.or(usage_count) {
        let used = if remaining > total {
            // Not a remaining value: the server meant "consumed".
            remaining
        } else {
            total - remaining
        };
        return Some(((used / total) * 100.0).clamp(0.0, 100.0) as f32);
    }
    None
}

/// A window ending at the reported reset.
///
/// The length comes from the server's own `start_time`/`end_time` pair when it
/// gives both — that is what places the time marker honestly — and from the
/// product constant otherwise. `remains_time` (milliseconds from now) is the
/// fallback when `end_time` is missing.
fn reset_window(
    end_time: Option<f64>,
    start_time: Option<f64>,
    remains_ms: Option<f64>,
    default_len_secs: i64,
    now: DateTime<Utc>,
) -> Option<LimitWindow> {
    let resets_at = end_time
        .and_then(epoch_to_utc)
        .or_else(|| remains_ms.map(|ms| now + chrono::Duration::milliseconds(ms as i64)))?;
    let len = match (
        start_time.and_then(epoch_to_utc),
        end_time.and_then(epoch_to_utc),
    ) {
        (Some(start), Some(end)) if end > start => end - start,
        _ => chrono::Duration::seconds(default_len_secs),
    };
    Some(LimitWindow::ending_at(resets_at, len, now))
}

#[derive(Deserialize, Debug, Default)]
#[serde(default)]
struct BaseResp {
    status_code: Option<Value>,
    status_msg: Option<Value>,
}

/// What the endpoint returns: an envelope with one record per model bucket.
#[derive(Deserialize, Debug, Default)]
#[serde(default)]
struct Envelope {
    model_remains: Option<Vec<ModelRemains>>,
    base_resp: Option<BaseResp>,
    /// The documented token-plan route has no wrapper, but the console/cookie
    /// route of the same family does; descended into when present.
    data: Option<Value>,
}

/// Map a non-zero `base_resp` to a message the user can act on.
fn base_error(code: f64, msg: &str) -> FetchError {
    let low = msg.to_lowercase();
    // 1004 missing/invalid key, 2049 wrong-region key, 2057 not a token-plan
    // key. The status text is the only reliable discriminator.
    if low.contains("login")
        || low.contains("auth")
        || low.contains("secret key")
        || low.contains("invalid api key")
    {
        return FetchError::from("ключ MiniMax не принят (проверьте регион ключа)");
    }
    if code == 2062.0 && low.contains("rate") {
        return FetchError::rate_limited("лимит запросов MiniMax");
    }
    if code == 2057.0 || code == 2062.0 || low.contains("plan") || low.contains("subscript") {
        return FetchError::from(format!(
            "у ключа нет активной подписки M Plan{}",
            if msg.is_empty() {
                String::new()
            } else {
                format!(" ({msg})")
            }
        ));
    }
    FetchError::from(format!(
        "MiniMax: {}",
        if msg.is_empty() {
            format!("код {}", code as i64)
        } else {
            msg.to_string()
        }
    ))
}

/// Parse the endpoint's JSON. Pure, so every branch below is unit-tested
/// against the field names the live API uses (see the `tests` module).
pub fn parse(payload: &Value, now: DateTime<Utc>) -> Result<Snapshot, FetchError> {
    let env: Envelope =
        serde_json::from_value(payload.clone()).map_err(|e| format!("ответ MiniMax: {e}"))?;

    let mut entries = env.model_remains;
    let mut base = env.base_resp;

    // Descend one level when the payload is wrapped (`data`), as the console
    // route does. The token-plan route returns the fields at the top level, so
    // this is only a compatibility path.
    if entries.is_none() {
        if let Some(data) = &env.data {
            if let Ok(nested) = serde_json::from_value::<Envelope>(data.clone()) {
                entries = nested.model_remains;
                if base.is_none() {
                    base = nested.base_resp;
                }
            } else if let Ok(list) = serde_json::from_value::<Vec<ModelRemains>>(data.clone()) {
                entries = Some(list);
            }
        }
    }

    let (status_code, status_msg) = base
        .as_ref()
        .map(|b| {
            (
                as_f64(b.status_code.as_ref()).unwrap_or(0.0),
                as_text(b.status_msg.as_ref()).unwrap_or_default(),
            )
        })
        .unwrap_or((0.0, String::new()));

    // HTTP 200 with a non-zero status is the normal failure shape here.
    if status_code != 0.0 {
        return Err(base_error(status_code, &status_msg));
    }

    let entries = entries.filter(|e| !e.is_empty()).ok_or_else(|| {
        FetchError::from("MiniMax не вернул ни одной записи о квоте (ключ без подписки?)")
    })?;

    // The `general` bucket is the aggregate text/speech pool and the only one
    // with a real 5-hour window. Without it, take the best non-video entry; a
    // video-only response is an error rather than a misleading single bar.
    let chosen = entries
        .iter()
        .find(|e| e.name().eq_ignore_ascii_case("general"))
        .or_else(|| {
            entries
                .iter()
                .filter(|e| !e.is_video())
                .max_by_key(|e| e.completeness(now))
        })
        .ok_or_else(|| {
            FetchError::from("MiniMax вернул только видео-пул — нет данных о текстовой квоте")
        })?;

    let mut limits = Vec::new();
    if let Some(l) = chosen.session_window(now) {
        limits.push(l);
    }
    if let Some(l) = chosen.weekly_window(now) {
        limits.push(l);
    }
    if limits.is_empty() {
        return Err(
            "MiniMax вернул запись о квоте без пригодных окон (ни процентов, ни счётчиков)".into(),
        );
    }

    let plan = match chosen.plan_label() {
        Some(p) if p.to_lowercase().starts_with("minimax") => p,
        Some(p) => format!("MiniMax {p}"),
        // The tier is not in the response, and inventing "Go" for a Build
        // subscriber would be wrong. The app is about MiniMax Go, so that is
        // the label it shows; the rows themselves are exact either way.
        None => PLAN_LABEL.to_string(),
    };

    Ok(Snapshot {
        family: Family::MiniMax,
        plan,
        limits,
    })
}

/// One GET against a MiniMax host. Returns the parsed body or an error that
/// keeps the distinction the UI needs (auth vs throttled vs transport).
fn get_json(url: &str, key: &str) -> Result<Value, FetchError> {
    let started = std::time::Instant::now();
    let resp = ureq::get(url)
        .set("Authorization", &format!("Bearer {key}"))
        .set("Content-Type", "application/json")
        .set("Accept", "application/json")
        .set("User-Agent", "Quotty/0.1")
        .timeout(std::time::Duration::from_secs(20))
        .call();

    match resp {
        Ok(r) => {
            diag(&format!(
                "minimax: {} in {} ms",
                r.status(),
                started.elapsed().as_millis()
            ));
            r.into_json::<Value>()
                .map_err(|e| FetchError::from(format!("ответ MiniMax: {e}")))
        }
        // MiniMax reports auth problems in the body, but a gateway may still
        // produce a plain 401/403 — treat it the same way.
        Err(ureq::Error::Status(401 | 403, _)) => Err(FetchError::from(
            "ключ MiniMax не принят (проверьте регион ключа)",
        )),
        Err(ureq::Error::Status(404, _)) => Err(FetchError::from("endpoint MiniMax не найден")),
        Err(ureq::Error::Status(429, _)) => {
            diag("minimax: status 429 (rate limited)");
            Err(FetchError::rate_limited("лимит запросов MiniMax"))
        }
        Err(ureq::Error::Status(code, _)) => {
            Err(FetchError::from(format!("MiniMax: status {code}")))
        }
        Err(e) => Err(FetchError::from(format!("сеть: {e}"))),
    }
}

pub fn fetch() -> Result<Snapshot, FetchError> {
    let key = secret::load(secret::Slot::MINIMAX).map_err(|e| {
        // Missing key is the normal state before the user enters one, not a
        // failure worth a log line.
        if !secret::exists(secret::Slot::MINIMAX) {
            FetchError::from("ключ MiniMax не задан")
        } else {
            dbg_log(&format!("minimax: {e}"));
            FetchError::from(e)
        }
    })?;

    let region = config::minimax_region();
    let hosts: [&str; 2] = if region.eq_ignore_ascii_case("cn") {
        [CN_URL, GLOBAL_URL]
    } else {
        [GLOBAL_URL, CN_URL]
    };

    let first = match get_json(hosts[0], &key) {
        Ok(v) => return parse(&v, Utc::now()),
        Err(e) => e,
    };

    // A key is bound to one platform: the other host is the likeliest reason a
    // correct key is rejected. Retry only for auth/not-found, not for
    // throttling or transport errors.
    let retryable = first.msg.contains("не принят") || first.msg.contains("не найден");
    if !retryable {
        return Err(first);
    }
    diag("minimax: retrying the other region host");
    match get_json(hosts[1], &key) {
        Ok(v) => parse(&v, Utc::now()),
        Err(second) => Err(FetchError {
            msg: format!("{} (другой регион: {})", first.msg, second.msg),
            rate_limited: false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse, used_percent};
    use chrono::{TimeZone, Utc};

    fn at(secs: i64) -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).single().unwrap()
    }

    /// Epoch milliseconds, as the live endpoint reports them.
    fn ms(secs: i64) -> i64 {
        secs * 1000
    }

    /// Shape of a real `general` record: both windows, epoch-millisecond
    /// bounds, percent fields, statuses.
    fn general_json() -> String {
        format!(
            r#"{{
            "model_remains": [
                {{
                    "start_time": {},
                    "end_time": {},
                    "remains_time": 1877492,
                    "current_interval_total_count": 0,
                    "current_interval_usage_count": 0,
                    "model_name": "general",
                    "current_weekly_total_count": 0,
                    "current_weekly_usage_count": 0,
                    "weekly_start_time": {},
                    "weekly_end_time": {},
                    "weekly_remains_time": 534677492,
                    "current_interval_status": 1,
                    "current_interval_remaining_percent": 62,
                    "current_weekly_status": 1,
                    "current_weekly_remaining_percent": 25
                }}
            ],
            "base_resp": {{ "status_code": 0, "status_msg": "success" }}
        }}"#,
            ms(1773000000),
            ms(1773018000),
            ms(1773000000),
            ms(1773604800),
        )
    }

    #[test]
    fn parses_both_windows_from_a_live_shaped_payload() {
        let now = at(1773010000);
        let snap = parse(&serde_json::from_str(&general_json()).unwrap(), now).unwrap();
        assert_eq!(snap.family, super::Family::MiniMax);
        assert_eq!(snap.limits.len(), 2);

        let session = &snap.limits[0];
        assert_eq!(session.title, "5 Hours");
        // remaining 62 % -> 38 % used.
        assert!(
            (session.used_percent - 38.0).abs() < 0.01,
            "{}",
            session.used_percent
        );
        assert_eq!(session.window.unwrap().resets_at, at(1773018000));
        // The window length comes from the server's own bounds: 5 hours.
        let w = session.window.unwrap();
        assert_eq!(
            w.resets_at - w.start.unwrap(),
            chrono::Duration::seconds(5 * 3600)
        );

        let weekly = &snap.limits[1];
        assert_eq!(weekly.title, "Weekly");
        // remaining 25 % -> 75 % used.
        assert!(
            (weekly.used_percent - 75.0).abs() < 0.01,
            "{}",
            weekly.used_percent
        );
        assert_eq!(weekly.window.unwrap().resets_at, at(1773604800));

        assert_eq!(snap.plan, "MiniMax Go");
    }

    #[test]
    fn counts_are_read_as_remaining_when_percent_is_absent() {
        let json = r#"{"model_remains":[{
            "model_name":"general",
            "current_interval_total_count":1000,
            "current_interval_usage_count":620,
            "current_weekly_total_count":10000,
            "current_weekly_usage_count":2500,
            "end_time":1773018000000,
            "weekly_end_time":1773600000000
        }]}"#;
        let snap = parse(&serde_json::from_str(json).unwrap(), at(1773010000)).unwrap();
        assert!((snap.limits[0].used_percent - 38.0).abs() < 0.01);
        assert!((snap.limits[1].used_percent - 75.0).abs() < 0.01);
    }

    #[test]
    fn the_server_percent_wins_over_the_count_fields() {
        // remaining 12 % => 88 % used, regardless of what the counts suggest.
        let json = r#"{"model_remains":[{
            "model_name":"general",
            "current_interval_total_count":1000,
            "current_interval_usage_count":500,
            "current_interval_remaining_percent":12,
            "current_weekly_remaining_percent":40,
            "end_time":1773018000,
            "weekly_end_time":1773600000
        }]}"#;
        let snap = parse(&serde_json::from_str(json).unwrap(), at(1773010000)).unwrap();
        assert!((snap.limits[0].used_percent - 88.0).abs() < 0.01);
        assert!((snap.limits[1].used_percent - 60.0).abs() < 0.01);
    }

    #[test]
    fn counts_reported_as_consumed_are_detected() {
        // usage_count above the total cannot be "remaining": read as consumed.
        let json = r#"{"model_remains":[{
            "model_name":"general",
            "current_interval_total_count":100,
            "current_interval_usage_count":140
        }]}"#;
        let snap = parse(&serde_json::from_str(json).unwrap(), at(1773010000)).unwrap();
        assert_eq!(snap.limits.len(), 1, "only the 5-hour window is present");
        assert!((snap.limits[0].used_percent - 100.0).abs() < 0.01);
    }

    #[test]
    fn explicit_remaining_count_alias_is_understood() {
        let json = r#"{"model_remains":[{
            "model_name":"general",
            "current_interval_total_count":200,
            "current_interval_remaining_count":50,
            "current_weekly_total_count":400,
            "current_weekly_usage_count":300
        }]}"#;
        let snap = parse(&serde_json::from_str(json).unwrap(), at(1773010000)).unwrap();
        assert!((snap.limits[0].used_percent - 75.0).abs() < 0.01);
        assert!((snap.limits[1].used_percent - 25.0).abs() < 0.01);
    }

    #[test]
    fn string_numbers_are_accepted() {
        let json = r#"{"model_remains":[{
            "model_name":"general",
            "current_interval_remaining_percent":"2"
        }]}"#;
        let snap = parse(&serde_json::from_str(json).unwrap(), at(1773010000)).unwrap();
        assert!((snap.limits[0].used_percent - 98.0).abs() < 0.01);
    }

    #[test]
    fn millisecond_and_second_resets_both_land_on_the_same_instant() {
        let ms = r#"{"model_remains":[{"model_name":"general",
            "current_interval_remaining_percent":10,"end_time":1773018000000}]}"#;
        let secs = r#"{"model_remains":[{"model_name":"general",
            "current_interval_remaining_percent":10,"end_time":1773018000}]}"#;
        let now = at(1773010000);
        let a = parse(&serde_json::from_str(ms).unwrap(), now).unwrap();
        let b = parse(&serde_json::from_str(secs).unwrap(), now).unwrap();
        assert_eq!(
            a.limits[0].window.unwrap().resets_at,
            b.limits[0].window.unwrap().resets_at
        );
    }

    #[test]
    fn remains_time_is_milliseconds() {
        // A real `remains_time` of 1_877_492 ms is ~31 minutes, not 21 days.
        let json = r#"{"model_remains":[{"model_name":"general",
            "current_interval_remaining_percent":10,"remains_time":1877492}]}"#;
        let now = at(1773010000);
        let snap = parse(&serde_json::from_str(json).unwrap(), now).unwrap();
        let reset = snap.limits[0].window.unwrap().resets_at;
        let left = (reset - now).num_seconds();
        assert!(
            (left - 1877).abs() <= 1,
            "remains_time must be ms, got {left}s left"
        );
    }

    #[test]
    fn an_exhausted_weekly_status_reads_as_one_hundred_percent() {
        let json = r#"{"model_remains":[{"model_name":"general",
            "current_interval_remaining_percent":80,
            "current_weekly_remaining_percent":80,
            "current_weekly_status":2,
            "weekly_end_time":1773600000000}]}"#;
        let snap = parse(&serde_json::from_str(json).unwrap(), at(1773010000)).unwrap();
        assert!((snap.limits[1].used_percent - 100.0).abs() < 0.01);
    }

    #[test]
    fn a_plan_without_a_weekly_limit_draws_only_the_five_hour_row() {
        let json = r#"{"model_remains":[{"model_name":"general",
            "current_interval_remaining_percent":80,
            "current_weekly_remaining_percent":100,
            "current_weekly_status":3}]}"#;
        let snap = parse(&serde_json::from_str(json).unwrap(), at(1773010000)).unwrap();
        assert_eq!(snap.limits.len(), 1);
        assert_eq!(snap.limits[0].title, "5 Hours");
    }

    #[test]
    fn the_general_bucket_is_preferred_over_video() {
        let json = r#"{"model_remains":[
            {"model_name":"video","current_weekly_remaining_percent":90},
            {"model_name":"general","current_interval_remaining_percent":50,
             "current_weekly_remaining_percent":20}
        ]}"#;
        let snap = parse(&serde_json::from_str(json).unwrap(), at(1773010000)).unwrap();
        assert_eq!(snap.limits.len(), 2);
        assert!((snap.limits[1].used_percent - 80.0).abs() < 0.01);
    }

    #[test]
    fn a_video_only_response_is_an_error_not_a_fake_text_bar() {
        let json = r#"{"model_remains":[
            {"model_name":"video","current_weekly_remaining_percent":90,
             "weekly_end_time":1773600000}
        ]}"#;
        let err = parse(&serde_json::from_str(json).unwrap(), at(1773010000)).unwrap_err();
        assert!(err.msg.contains("видео"), "{}", err.msg);
    }

    #[test]
    fn fields_wrapped_in_data_are_descended_into() {
        let json = r#"{"data":{"model_remains":[{"model_name":"general",
            "current_interval_remaining_percent":70}]}}"#;
        let snap = parse(&serde_json::from_str(json).unwrap(), at(1773010000)).unwrap();
        assert!((snap.limits[0].used_percent - 30.0).abs() < 0.01);
    }

    #[test]
    fn plan_tier_is_taken_from_the_payload_when_present() {
        let json = r#"{"model_remains":[{"model_name":"general",
            "plan_name":"Explore","current_interval_remaining_percent":10}]}"#;
        let snap = parse(&serde_json::from_str(json).unwrap(), at(1773010000)).unwrap();
        assert_eq!(snap.plan, "MiniMax Explore");
    }

    #[test]
    fn an_auth_failure_in_a_200_body_is_an_error() {
        // The live endpoint reports a missing key exactly like this.
        let json = r#"{"base_resp":{"status_code":1004,"status_msg":
            "login fail: Please carry the API secret key in the 'Authorization' field"}}"#;
        let err = parse(&serde_json::from_str(json).unwrap(), at(1773010000)).unwrap_err();
        assert!(err.msg.contains("не принят"), "{}", err.msg);
        assert!(!err.rate_limited);
    }

    #[test]
    fn a_pay_as_you_go_key_reports_no_subscription() {
        let json = r#"{"base_resp":{"status_code":2057,"status_msg":"no active M Plan subscription"},
                       "model_remains":[]}"#;
        let err = parse(&serde_json::from_str(json).unwrap(), at(1773010000)).unwrap_err();
        assert!(err.msg.contains("M Plan"), "{}", err.msg);
        assert!(!err.rate_limited);
    }

    #[test]
    fn an_empty_envelope_is_an_error() {
        for json in [
            r#"{"model_remains":[],"base_resp":{"status_code":0,"status_msg":"success"}}"#,
            r#"{"base_resp":{"status_code":0,"status_msg":"success"}}"#,
        ] {
            let err = parse(&serde_json::from_str(json).unwrap(), at(1773010000)).unwrap_err();
            assert!(!err.rate_limited);
        }
    }

    #[test]
    fn a_record_without_usable_numbers_is_not_shown_as_zero() {
        let json = r#"{"model_remains":[{"model_name":"general","end_time":1773018000}]}"#;
        let err = parse(&serde_json::from_str(json).unwrap(), at(1773010000)).unwrap_err();
        assert!(err.msg.contains("без пригодных окон"), "{}", err.msg);
    }

    #[test]
    fn used_percent_never_leaves_its_range() {
        assert_eq!(used_percent(Some(-40.0), None, None, None), Some(100.0));
        assert_eq!(used_percent(Some(140.0), None, None, None), Some(0.0));
        assert_eq!(used_percent(None, None, None, None), None);
        // A zero total is not a denominator: no value is invented.
        assert_eq!(used_percent(None, Some(5.0), None, Some(0.0)), None);
    }

    /// Parses a response captured with a **real** M Plan key and dumped by
    /// `tools/probe-providers.ps1 -DumpRaw <path>`:
    ///
    /// ```text
    /// $env:QUOTTY_MINIMAX_FIXTURE = "$env:TEMP\minimax-remains.json"
    /// cargo test minimax_live -- --nocapture
    /// ```
    ///
    /// Skipped when the variable is unset: the capture carries live account
    /// figures and is deliberately not committed.
    #[test]
    fn parses_a_live_capture_when_one_is_supplied() {
        let Ok(path) = std::env::var("QUOTTY_MINIMAX_FIXTURE") else {
            return;
        };
        let raw = std::fs::read_to_string(&path).expect("fixture readable");
        // Tolerate a BOM: a capture written by a Windows tool may carry one,
        // while a real HTTP body never does.
        let raw = raw.trim_start_matches('\u{feff}');
        let payload: serde_json::Value = serde_json::from_str(raw).expect("fixture is JSON");

        let now = Utc::now();
        let snap = parse(&payload, now).expect("real payload parses");
        assert_eq!(snap.family, super::Family::MiniMax);
        assert!(
            !snap.limits.is_empty(),
            "a real M Plan response must yield at least one window"
        );
        for limit in &snap.limits {
            assert!(
                (0.0..=100.0).contains(&limit.used_percent),
                "{}: {}",
                limit.title,
                limit.used_percent
            );
            if let Some(w) = limit.window {
                assert!(w.resets_at > now, "{}: reset is in the past", limit.title);
            }
        }
        println!(
            "live parse: plan={:?} limits={:?}",
            snap.plan,
            snap.limits
                .iter()
                .map(|l| (&l.title, l.used_percent))
                .collect::<Vec<_>>()
        );
    }
}
