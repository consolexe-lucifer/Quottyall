//! DeepSeek module: reads the prepaid **account balance** of a DeepSeek open
//! platform API key.
//!
//! ```text
//! GET https://api.deepseek.com/user/balance
//! Authorization: Bearer <DeepSeek API key>
//! ```
//!
//! DeepSeek's API is billed per token against a money balance — there is no
//! 5-hour or weekly quota window, so this provider deliberately draws **no
//! window**: one row per currency, a real balance figure, and (optionally) the
//! reset clock left empty. Rendering invented windows here would be a lie about
//! the product, which is exactly what this app must not do.
//!
//! The bar needs a fraction of *something*, and DeepSeek does not publish one.
//! So the fraction is anchored to a **baseline** the app keeps locally: the
//! balance the user (or the first observation) set as "full", adjusted upwards
//! automatically whenever a top-up raises the balance above it. The percentage
//! therefore means "how much of the baseline has been spent", which the settings
//! UI states in words. It is never presented as an official DeepSeek limit.
//!
//! Nothing secret is stored here: the key lives in the DPAPI-sealed store
//! (`crate::secret`), and this file only ever writes balances and baselines.

use super::{dbg_log, diag, Family, FetchError, Limit, Snapshot};
use crate::{config, secret};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

const BALANCE_URL: &str = "https://api.deepseek.com/user/balance";

/// File with the per-currency baselines. Separate from `settings.json` because
/// the poll thread owns it, and mixing it into the settings the UI writes would
/// risk losing whichever side wrote last.
const BASELINE_FILE: &str = "deepseek-baseline.json";

/// One currency's balance, as the endpoint reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Balance {
    pub currency: String,
    pub total: f64,
    pub granted: f64,
    pub topped_up: f64,
}

/// What `GET /user/balance` returns.
#[derive(Deserialize, Debug, Default)]
#[serde(default)]
struct BalanceResponse {
    is_available: Option<Value>,
    balance_infos: Option<Vec<BalanceInfo>>,
}

#[derive(Deserialize, Debug, Default)]
#[serde(default)]
struct BalanceInfo {
    currency: Option<Value>,
    total_balance: Option<Value>,
    granted_balance: Option<Value>,
    topped_up_balance: Option<Value>,
}

/// Money arrives as a JSON **string** ("110.00"), not a number. Accept both, so
/// a future change cannot blank the screen.
fn money(v: Option<&Value>) -> Option<f64> {
    let n = match v? {
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        Value::Number(n) => n.as_f64()?,
        _ => return None,
    };
    n.is_finite().then_some(n)
}

fn text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        }
        _ => None,
    }
}

fn is_true(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

/// Currency symbol for the row title; the code itself when unknown.
fn symbol(currency: &str) -> &str {
    match currency.to_ascii_uppercase().as_str() {
        "CNY" => "¥",
        "USD" => "$",
        _ => currency,
    }
}

/// Local baselines, keyed by currency code.
type Baselines = BTreeMap<String, f64>;

fn baseline_path() -> Option<std::path::PathBuf> {
    config::Settings::data_dir().map(|d| d.join(BASELINE_FILE))
}

/// Read the stored baselines. A missing file is normal (nothing tracked yet);
/// an unreadable one falls back to empty rather than failing the fetch.
fn load_baselines() -> Baselines {
    let Some(path) = baseline_path() else {
        return Baselines::new();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Baselines::new();
    };
    serde_json::from_str(raw.trim_start_matches('\u{feff}')).unwrap_or_else(|e| {
        dbg_log(&format!("deepseek: baselines unreadable: {e}"));
        Baselines::new()
    })
}

/// Persist baselines atomically, so an interrupted write cannot corrupt them.
fn save_baselines(baselines: &Baselines) -> Result<(), String> {
    let path = baseline_path().ok_or("не удалось определить каталог настроек")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("каталог настроек: {e}"))?;
    }
    let raw = serde_json::to_string_pretty(baselines).map_err(|e| format!("базовые суммы: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, raw).map_err(|e| format!("запись базовой суммы: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("переименование базовой суммы: {e}")
    })
}

/// `(baseline, changed)` for one currency.
///
/// The baseline is established from the first balance seen, and raised —
/// never lowered — when a top-up pushes the balance above it. Raising on a
/// top-up is what keeps the bar meaningful: "spent since the last top-up" would
/// otherwise jump to a negative and be clamped to 0 % forever.
fn rebase(baseline: Option<f64>, total: f64) -> (f64, bool) {
    match baseline {
        // Already recorded at exactly this balance — including the "no money in
        // this currency at all" state — so there is nothing to persist.
        Some(b) if b >= 0.0 && (b - total).abs() < 1e-9 => (b, false),
        Some(b) if b > 0.0 && total <= b => (b, false),
        _ => (total.max(0.0), true),
    }
}

/// Percent of the baseline that has been spent, 0..=100.
///
/// A zero baseline carries no information (nothing was ever tracked in that
/// currency), so it reports a neutral 0 % instead of pretending the account is
/// empty. Once a real baseline exists, a zero balance honestly reads as 100 %.
fn used_percent(baseline: f64, total: f64) -> f32 {
    if baseline <= 0.0 {
        return 0.0;
    }
    (((baseline - total) / baseline) * 100.0).clamp(0.0, 100.0) as f32
}

/// Build the snapshot, updating `baselines` in place. Returns whether the map
/// changed and therefore needs saving. Pure: the tests drive it directly.
fn build_snapshot(
    balances: &[Balance],
    available: bool,
    baselines: &mut Baselines,
) -> (Snapshot, bool) {
    let mut dirty = false;
    let mut limits = Vec::new();

    for b in balances {
        let entry = baselines.get(&b.currency).copied();
        let (baseline, changed) = rebase(entry, b.total);
        if changed {
            baselines.insert(b.currency.clone(), baseline);
            dirty = true;
        }
        // The balance is real even on the first sighting; the bar simply has
        // nothing spent against it yet.
        let used = if changed {
            0.0
        } else {
            used_percent(baseline, b.total)
        };
        let mut title = format!("Balance {}{:.2}", symbol(&b.currency), b.total);
        // Gifted credit is worth showing: it expires and it is not money the
        // user paid in. Only when there is some, to keep the row short.
        if b.granted > 0.0 && b.topped_up > 0.0 {
            title.push_str(&format!(" · bonus {}{:.2}", symbol(&b.currency), b.granted));
        }
        limits.push(Limit {
            title,
            used_percent: used,
            // No window: DeepSeek has no quota window. `None` means "no clock",
            // which is exactly the truth here.
            window: None,
        });
    }

    let plan = format!(
        "DeepSeek API · {}",
        if available {
            "средств хватает"
        } else {
            "недостаточно средств"
        }
    );
    (
        Snapshot {
            family: Family::DeepSeek,
            plan,
            limits,
        },
        dirty,
    )
}

/// Parse the endpoint's JSON. Pure, so the tests cover every branch.
pub fn parse(payload: &Value, baselines: &mut Baselines) -> Result<(Snapshot, bool), FetchError> {
    let resp: BalanceResponse =
        serde_json::from_value(payload.clone()).map_err(|e| format!("ответ DeepSeek: {e}"))?;

    let infos = resp
        .balance_infos
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            FetchError::from("DeepSeek не вернул данных о балансе (ключ без доступа?)")
        })?;

    let mut balances = Vec::new();
    for info in infos {
        let currency = text(info.currency.as_ref())
            .map(|c| c.to_ascii_uppercase())
            .unwrap_or_else(|| "CNY".to_string());
        // A balance row without a readable total is not a zero: skip it rather
        // than show "0.00", which would look like an empty account.
        let Some(total) = money(info.total_balance.as_ref()) else {
            diag(&format!("deepseek: {currency} row without total_balance"));
            continue;
        };
        balances.push(Balance {
            currency,
            total,
            granted: money(info.granted_balance.as_ref()).unwrap_or(0.0),
            topped_up: money(info.topped_up_balance.as_ref()).unwrap_or(0.0),
        });
    }
    if balances.is_empty() {
        return Err("DeepSeek не вернул ни одной суммы баланса".into());
    }

    Ok(build_snapshot(
        &balances,
        is_true(resp.is_available.as_ref()),
        baselines,
    ))
}

/// Read the `error.message` of a DeepSeek error body, when there is one.
fn error_detail(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    v.get("error")?.get("message")?.as_str().map(str::to_string)
}

fn get_json(key: &str) -> Result<Value, FetchError> {
    let started = std::time::Instant::now();
    let resp = ureq::get(BALANCE_URL)
        .set("Authorization", &format!("Bearer {key}"))
        .set("Accept", "application/json")
        .set("User-Agent", "Quotty/0.1")
        .timeout(std::time::Duration::from_secs(20))
        .call();

    match resp {
        Ok(r) => {
            diag(&format!(
                "deepseek: {} in {} ms",
                r.status(),
                started.elapsed().as_millis()
            ));
            r.into_json::<Value>()
                .map_err(|e| FetchError::from(format!("ответ DeepSeek: {e}")))
        }
        Err(ureq::Error::Status(401, r)) => {
            // The body names the reason ("Authentication Fails, Your api key
            // ... is invalid"); keep it, it is not secret.
            let detail = r.into_string().ok().and_then(|b| error_detail(&b));
            Err(FetchError::from(match detail {
                Some(d) => format!("ключ DeepSeek не принят: {d}"),
                None => "ключ DeepSeek не принят".to_string(),
            }))
        }
        // 402 is the wallet being empty — a billing state, not a bad key. The
        // endpoint even reports it for a valid key; saying "ключ не принят"
        // here would send the user hunting for the wrong problem.
        Err(ureq::Error::Status(402, r)) => {
            let detail = r.into_string().ok().and_then(|b| error_detail(&b));
            Err(FetchError::from(match detail {
                Some(d) => format!("на балансе DeepSeek недостаточно средств: {d}"),
                None => "на балансе DeepSeek недостаточно средств".to_string(),
            }))
        }
        Err(ureq::Error::Status(403, _)) => Err(FetchError::from(
            "доступ к балансу DeepSeek запрещён для этого ключа",
        )),
        Err(ureq::Error::Status(429, _)) => {
            diag("deepseek: status 429 (rate limited)");
            Err(FetchError::rate_limited("лимит запросов DeepSeek"))
        }
        Err(ureq::Error::Status(code, _)) => {
            diag(&format!("deepseek: status {code}"));
            Err(FetchError::from(format!("DeepSeek: status {code}")))
        }
        Err(e) => Err(FetchError::from(format!("сеть: {e}"))),
    }
}

pub fn fetch() -> Result<Snapshot, FetchError> {
    let key = secret::load(secret::Slot::DEEPSEEK).map_err(|e| {
        if !secret::exists(secret::Slot::DEEPSEEK) {
            FetchError::from("ключ DeepSeek не задан")
        } else {
            dbg_log(&format!("deepseek: {e}"));
            FetchError::from(e)
        }
    })?;

    let payload = get_json(&key).map_err(|e| {
        // A missing key is a configuration state, not an incident: everything
        // else is worth a line in the log so "why is this empty" is answerable.
        FetchError::from(e.msg)
    })?;

    let mut baselines = load_baselines();
    let (snapshot, dirty) = parse(&payload, &mut baselines)?;
    if dirty {
        if let Err(e) = save_baselines(&baselines) {
            // Not fatal: the bar is still correct for this poll.
            dbg_log(&format!("deepseek: baseline not saved: {e}"));
        }
    }
    Ok(snapshot)
}

/// Reset the baseline of every trackable currency to its current balance.
/// Called from the settings UI; the next poll then reports "0 % spent".
pub fn reset_baselines() -> Result<(), String> {
    let key = secret::load(secret::Slot::DEEPSEEK)?;
    let payload = get_json(&key).map_err(|e| e.msg)?;
    let resp: BalanceResponse =
        serde_json::from_value(payload).map_err(|e| format!("ответ DeepSeek: {e}"))?;
    let mut baselines = Baselines::new();
    for info in resp.balance_infos.unwrap_or_default() {
        let currency = text(info.currency.as_ref())
            .map(|c| c.to_ascii_uppercase())
            .unwrap_or_else(|| "CNY".to_string());
        if let Some(total) = money(info.total_balance.as_ref()) {
            baselines.insert(currency, total);
        }
    }
    save_baselines(&baselines)
}

/// Current baselines, for the settings UI to show what the percentage means.
pub fn baselines() -> Baselines {
    load_baselines()
}

#[cfg(test)]
mod tests {
    use super::{build_snapshot, parse, rebase, used_percent, Balance, Baselines};

    fn map(pairs: &[(&str, f64)]) -> Baselines {
        pairs.iter().map(|(c, v)| (c.to_string(), *v)).collect()
    }

    #[test]
    fn cny_balance_is_rendered_with_the_yuan_sign() {
        let json = r#"{
            "is_available": true,
            "balance_infos": [{
                "currency": "CNY",
                "total_balance": "110.00",
                "granted_balance": "10.00",
                "topped_up_balance": "100.00"
            }]
        }"#;
        let mut baselines = map(&[("CNY", 110.0)]);
        let (snap, dirty) = parse(&serde_json::from_str(json).unwrap(), &mut baselines).unwrap();
        assert!(!dirty);
        assert_eq!(snap.family, super::Family::DeepSeek);
        assert_eq!(snap.plan, "DeepSeek API · средств хватает");
        assert_eq!(snap.limits.len(), 1);
        assert!(
            snap.limits[0].title.starts_with("Balance ¥110.00"),
            "{}",
            snap.limits[0].title
        );
        assert!(snap.limits[0].window.is_none(), "у баланса нет окна сброса");
        assert!((snap.limits[0].used_percent - 0.0).abs() < 0.01);
    }

    #[test]
    fn usd_balance_is_rendered_with_the_dollar_sign() {
        let json = r#"{"is_available": true, "balance_infos": [{
            "currency": "USD", "total_balance": "4.20",
            "granted_balance": "0.00", "topped_up_balance": "4.20"
        }]}"#;
        let mut baselines = map(&[("USD", 10.0)]);
        let (snap, _) = parse(&serde_json::from_str(json).unwrap(), &mut baselines).unwrap();
        assert!(
            snap.limits[0].title.starts_with("Balance $4.20"),
            "{}",
            snap.limits[0].title
        );
        // 4.20 of 10.00 spent -> 58 %.
        assert!((snap.limits[0].used_percent - 58.0).abs() < 0.01);
    }

    #[test]
    fn a_zero_balance_reads_as_fully_spent_not_as_missing() {
        let json = r#"{"is_available": false, "balance_infos": [{
            "currency": "CNY", "total_balance": "0.00",
            "granted_balance": "0.00", "topped_up_balance": "0.00"
        }]}"#;
        let mut baselines = map(&[("CNY", 50.0)]);
        let (snap, dirty) = parse(&serde_json::from_str(json).unwrap(), &mut baselines).unwrap();
        assert!(!dirty);
        assert!(snap.plan.ends_with("недостаточно средств"), "{}", snap.plan);
        assert!(snap.limits[0].title.contains("¥0.00"));
        assert!((snap.limits[0].used_percent - 100.0).abs() < 0.01);
    }

    #[test]
    fn a_top_up_raises_the_baseline_and_resets_the_bar() {
        let json = r#"{"is_available": true, "balance_infos": [{
            "currency": "CNY", "total_balance": "150.00",
            "granted_balance": "0.00", "topped_up_balance": "150.00"
        }]}"#;
        let mut baselines = map(&[("CNY", 100.0)]);
        let (snap, dirty) = parse(&serde_json::from_str(json).unwrap(), &mut baselines).unwrap();
        assert!(dirty, "a top-up must be persisted");
        assert_eq!(baselines["CNY"], 150.0);
        assert!((snap.limits[0].used_percent - 0.0).abs() < 0.01);
    }

    #[test]
    fn first_sighting_establishes_the_baseline() {
        let json = r#"{"is_available": true, "balance_infos": [{
            "currency": "CNY", "total_balance": "73.50",
            "granted_balance": "0", "topped_up_balance": "73.50"
        }]}"#;
        let mut baselines = Baselines::new();
        let (snap, dirty) = parse(&serde_json::from_str(json).unwrap(), &mut baselines).unwrap();
        assert!(dirty);
        assert_eq!(baselines["CNY"], 73.5);
        assert!((snap.limits[0].used_percent - 0.0).abs() < 0.01);
    }

    #[test]
    fn both_currencies_get_their_own_row() {
        let json = r#"{"is_available": true, "balance_infos": [
            {"currency": "CNY", "total_balance": "100.00", "granted_balance": "0", "topped_up_balance": "100.00"},
            {"currency": "USD", "total_balance": "5.00", "granted_balance": "0", "topped_up_balance": "5.00"}
        ]}"#;
        let mut baselines = map(&[("CNY", 100.0), ("USD", 5.0)]);
        let (snap, _) = parse(&serde_json::from_str(json).unwrap(), &mut baselines).unwrap();
        assert_eq!(snap.limits.len(), 2);
        assert!(snap.limits[0].title.contains('¥'));
        assert!(snap.limits[1].title.contains('$'));
    }

    #[test]
    fn granted_credit_is_named_in_the_title() {
        let json = r#"{"is_available": true, "balance_infos": [{
            "currency": "CNY", "total_balance": "110.00",
            "granted_balance": "10.00", "topped_up_balance": "100.00"
        }]}"#;
        let mut baselines = map(&[("CNY", 110.0)]);
        let (snap, _) = parse(&serde_json::from_str(json).unwrap(), &mut baselines).unwrap();
        assert!(
            snap.limits[0].title.contains("bonus ¥10.00"),
            "{}",
            snap.limits[0].title
        );
    }

    #[test]
    fn an_empty_or_missing_balance_list_is_an_error() {
        for json in [
            r#"{"is_available": true, "balance_infos": []}"#,
            r#"{"is_available": true}"#,
        ] {
            let mut baselines = Baselines::new();
            let err = parse(&serde_json::from_str(json).unwrap(), &mut baselines).unwrap_err();
            assert!(err.msg.contains("баланс"), "{}", err.msg);
            assert!(!err.rate_limited);
        }
    }

    #[test]
    fn a_row_without_a_readable_total_is_skipped_not_zeroed() {
        let json = r#"{"is_available": true, "balance_infos": [
            {"currency": "CNY", "total_balance": "not-a-number"},
            {"currency": "USD", "total_balance": "3.00", "granted_balance": "0", "topped_up_balance": "3.00"}
        ]}"#;
        let mut baselines = map(&[("USD", 3.0)]);
        let (snap, _) = parse(&serde_json::from_str(json).unwrap(), &mut baselines).unwrap();
        assert_eq!(snap.limits.len(), 1);
        assert!(snap.limits[0].title.contains("$3.00"));
    }

    #[test]
    fn the_no_window_case_is_not_drawn_as_a_reset() {
        let balances = vec![Balance {
            currency: "CNY".into(),
            total: 10.0,
            granted: 0.0,
            topped_up: 10.0,
        }];
        let mut baselines = map(&[("CNY", 10.0)]);
        let (snap, _) = build_snapshot(&balances, true, &mut baselines);
        assert!(snap.limits[0].window.is_none());
    }

    #[test]
    fn baseline_math_never_lowers_a_good_reference() {
        // Untouched balance keeps the baseline.
        assert_eq!(rebase(Some(100.0), 90.0), (100.0, false));
        // A top-up raises it.
        assert_eq!(rebase(Some(100.0), 250.0), (250.0, true));
        // Nothing usable before -> establish from the current balance.
        assert_eq!(rebase(None, 42.0), (42.0, true));
        assert_eq!(rebase(Some(0.0), 42.0), (42.0, true));
    }

    #[test]
    fn percent_is_clamped_and_zero_baseline_is_handled() {
        assert!((used_percent(100.0, 25.0) - 75.0).abs() < 0.01);
        assert_eq!(used_percent(100.0, 1000.0), 0.0);
        // No baseline yet: neutral, not "everything is gone".
        assert_eq!(used_percent(0.0, 0.0), 0.0);
        assert_eq!(used_percent(0.0, 5.0), 0.0);
        // A real baseline with an emptied wallet is a genuine 100 %.
        assert_eq!(used_percent(10.0, 0.0), 100.0);
    }

    #[test]
    fn a_currency_that_never_had_money_is_not_repersisted_every_poll() {
        assert_eq!(rebase(Some(0.0), 0.0), (0.0, false));
        assert_eq!(rebase(Some(7.96), 7.96), (7.96, false));
    }

    /// Parses a response captured with a **real** DeepSeek key and dumped by
    /// `tools/probe-providers.ps1 -DumpRaw <path>`:
    ///
    /// ```text
    /// $env:QUOTTY_DEEPSEEK_FIXTURE = "$env:TEMP\deepseek-balance.json"
    /// cargo test deepseek_live -- --nocapture
    /// ```
    ///
    /// Skipped when the variable is unset: the capture carries live account
    /// figures and is deliberately not committed.
    #[test]
    fn parses_a_live_capture_when_one_is_supplied() {
        let Ok(path) = std::env::var("QUOTTY_DEEPSEEK_FIXTURE") else {
            return;
        };
        let raw = std::fs::read_to_string(&path).expect("fixture readable");
        let raw = raw.trim_start_matches('\u{feff}');
        let payload: serde_json::Value = serde_json::from_str(raw).expect("fixture is JSON");

        let mut baselines = Baselines::new();
        let (snap, dirty) = parse(&payload, &mut baselines).expect("real payload parses");
        assert_eq!(snap.family, super::Family::DeepSeek);
        assert!(!snap.limits.is_empty(), "real balance response has a row");
        for limit in &snap.limits {
            assert!(limit.window.is_none(), "DeepSeek has no reset window");
            assert!((0.0..=100.0).contains(&limit.used_percent));
            assert!(limit.title.starts_with("Balance "), "{}", limit.title);
        }
        // The first sighting always establishes a baseline worth saving.
        assert!(dirty);
        println!(
            "live parse: plan={:?} rows={:?} baselines={:?}",
            snap.plan,
            snap.limits
                .iter()
                .map(|l| (&l.title, l.used_percent))
                .collect::<Vec<_>>(),
            baselines
        );
    }
}
