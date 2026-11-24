# MiniMax M Plan ("MiniMax Go") subscription quota API — verified research

Status date: **2026-11-24**. All "VERIFIED" claims below were checked against a primary source
(MiniMax's own docs or a live network probe) on that date. Everything else is explicitly marked
**UNVERIFIED** with the exact probe needed to settle it with a real key.

Scope: reading the **5-hour window** and **weekly window** of an M Plan subscription (the plan
tier named **Go**; also Explore / Build). This is the official way the MiniMax console and
`mmx quota` CLI read the same numbers.

---

## 1. Summary table

| Endpoint | Method | Status | What it returns |
| --- | --- | --- | --- |
| `https://www.minimax.io/v1/token_plan/remains` | GET | **VERIFIED (primary docs + live probe)** | JSON envelope: `base_resp` + `model_remains[]`, each row carrying a rolling interval window and a weekly window, expressed as **remaining percent** (plus epoch-ms window bounds) |
| `https://api.minimax.io/v1/token_plan/remains` | GET | **VERIFIED (live probe)** — same backend | identical payload |
| `https://api.minimaxi.com/v1/token_plan/remains` | GET | **VERIFIED (live probe)** — CN site | identical payload |
| `https://www.minimaxi.com/v1/token_plan/remains` | GET | **VERIFIED (live probe)** — legacy CN host | identical payload |
| `https://www.minimax.cn/v1/token_plan/remains` | GET | **VERIFIED (live probe)** — new CN host | identical payload |
| `/v1/coding_plan/remains` (same hosts) | GET | **VERIFIED (live probe)** — legacy Coding Plan path, still routed | same shape; historically returned **per-model** rows (`MiniMax-M2`, `M2.1`, `M2.5`) and **prompt counts** instead of percentages |
| `/v1/api/openplatform/coding_plan/remains` | GET | **VERIFIED (live probe)** — this is the **console/cookie** route, not an API-key route | replies `1004 cookie is missing, log in again` without a session cookie |
| `/v1/api/openplatform/token_plan/remains` | GET | **VERIFIED absent** — HTTP 404 `404 page not found` | n/a |

The endpoint named by the user (`/v1/token_plan/remains`) **exists and is current**. It is the
endpoint MiniMax's own FAQ tells subscribers to call. There is **no** documented
`/v1/api/openplatform/token_plan/remains`.

Live probe evidence (no key sent, GET, 2026-11-24): every existing host/path above answered
**HTTP 200** with a JSON body — MiniMax signals auth failure in the body, not the status line.
Two negative controls (`/v1/api/openplatform/token_plan/remains` and `/v1/token_plan/not_remains`)
returned a plain-text `404 page not found`, so the 1004 bodies prove real routes rather than a
catch-all handler:

```json
{"base_resp":{"status_code":1004,"status_msg":"login fail: Please carry the API secret key in the 'Authorization' field of the request header"}}
```

---

## 2. Auth

### Header

```http
GET /v1/token_plan/remains HTTP/1.1
Host: www.minimax.io
Authorization: Bearer <M Plan API key>
Content-Type: application/json      (docs show it; GET needs no body — harmless)
Accept: application/json
```

- **Exactly one credential header:** `Authorization: Bearer <key>`.
  **VERIFIED** in the official FAQ curl snippet and in every independent implementation found.
- No `GroupId` query/header is needed for this endpoint. `GroupId` belongs to the legacy Open
  Platform endpoints (file APIs / old cookie-session quota scraping); implementations append it
  only when replaying a **browser session token**, never for an API key. **GroupId NOT required.**
- No request body, no query parameters, no path parameters. A bare `GET` with the header is enough.
- `Content-Type: application/json` is included by the official snippet; the request has no body, so
  it is cosmetic. Some clients send only `Accept: application/json`.

### Which key type works

- **M Plan / Token Plan Subscription Key** — the key shown at
  `https://platform.minimax.io/console/plan` (docs: "find your M Plan API key in Plan Details").
  This is the only key type that returns subscription windows.
- **Pay-as-you-go API Key does NOT work.** It is a separate key family, not interchangeable
  (official FAQ). Expect `base_resp.status_code` `2062` / `2057` instead of data.
- **Browser OAuth access token** also works (same Bearer header); several trackers prefer it, but
  for a desktop app the static Subscription Key is the correct choice — no cookie jar, no refresh.
- Minimum-privilege note: the key can be sent to `www.minimax.io` or `api.minimax.io` only; the
  same key is region-bound (a global key against the CN host yields `2049 invalid API Key` type
  failures). Keep host and key region paired.

### Key prefixes (UNVERIFIED, community reports only)

`sk-cp-*` = subscription ("Coding Plan") key, `sk-api-*` = pay-as-you-go key. Do **not** branch on the
prefix in code; any non-zero `base_resp.status_code` is the authority.

### Domain / region matrix

| Region | Official host (docs) | Alternate hosts that answer (live-verified) | Note |
| --- | --- | --- | --- |
| Global | `www.minimax.io` | `api.minimax.io` | `.io` = international site; docs console `platform.minimax.io` |
| Mainland China | `api.minimaxi.com` | `www.minimaxi.com`, `www.minimax.cn` | `minimaxi.com` = legacy CN domain; `minimax.cn` = current CN domain (`platform.minimax.cn` console) |
| Console (cookie session) | `platform.minimax.io` / `platform.minimax.cn` | — | serves HTML + cookie-auth endpoints; **not** for API keys |

The payloads are byte-identical in shape across hosts; only the account behind the key differs.
Recommended strategy: try the host matching the key's region, fall back to the other region only on
`404`/`2049`, exactly like the open-source trackers do. Do **not** send the key to an arbitrary
redirect target — several implementations disable redirect-following for this reason.

---

## 3. Response schema

### Envelope

```json
{
  "base_resp": { "status_code": 0, "status_msg": "success" },
  "model_remains": [ /* one row per quota bucket, normally "general" and "video" */ ]
}
```

- `base_resp.status_code` — the real error channel. `0` = success. **HTTP 200 does not mean success.**
- `model_remains` — absent (not merely empty) on failure responses.
- There is no `data` wrapper on `/v1/token_plan/remains`. The cookie/console route
  `/v1/api/openplatform/coding_plan/remains` is reported by some clients to wrap in `data`; the
  token_plan route returns top level. Parse defensively: if `data` is an object, descend into it.

### Row (bucket) fields — exact JSON names

Verbatim from a captured live response of the global endpoint, plus fields seen in other real
captures (`video` row, boost fields). Field names are **snake_case**.

```json
{
  "model_remains": [
    {
      "start_time": 1785164400000,
      "end_time": 1785182400000,
      "remains_time": 1877492,
      "current_interval_total_count": 0,
      "current_interval_usage_count": 0,
      "model_name": "general",
      "current_weekly_total_count": 0,
      "current_weekly_usage_count": 0,
      "weekly_start_time": 1785110400000,
      "weekly_end_time": 1785715200000,
      "weekly_remains_time": 534677492,
      "current_interval_status": 1,
      "current_interval_remaining_percent": 99,
      "current_weekly_status": 1,
      "current_weekly_remaining_percent": 99
    },
    {
      "start_time": 1785110400000,
      "end_time": 1785196800000,
      "remains_time": 16277492,
      "current_interval_total_count": 3,
      "current_interval_usage_count": 0,
      "model_name": "video",
      "current_weekly_total_count": 21,
      "current_weekly_usage_count": 0,
      "weekly_start_time": 1785110400000,
      "weekly_end_time": 1785715200000,
      "weekly_remains_time": 534677492,
      "current_interval_status": 1,
      "current_interval_remaining_percent": 100,
      "current_weekly_status": 1,
      "current_weekly_remaining_percent": 100
    }
  ],
  "base_resp": { "status_code": 0, "status_msg": "success" }
}
```

(`weekly_boost_permille: 1500` has also been observed on a real `general` row — see table.)

### Field-by-field interpretation

| JSON field | Meaning | Units / range | Evidence |
| --- | --- | --- | --- |
| `model_name` | Quota bucket name. `"general"` = text/speech pool (the one to display). `"video"` = video-generation pool (weekly-only product; its interval is 24 h). Old coding_plan responses used model names (`"MiniMax-M2"`, …) as row keys. | string | live captures |
| `current_interval_remaining_percent` | **Percent of the 5-hour window still available** (0–100). This is the 5h window. | int 0..100 (`i64`, sometimes float) | live captures; all clients invert it |
| `current_weekly_remaining_percent` | **Percent of the weekly window still available** (0–100). This is the weekly window. | int 0..100 | live captures |
| `current_interval_total_count` | Total quota of the interval window **in prompts/tokens**, legacy/count mode. **`0` in the current percentage-mode M Plan response.** | int ≥ 0, often 0 | live captures |
| `current_interval_usage_count` | ⚠️ **REMAINING count, despite the name** (this is a `/remains` endpoint). Not "used". In percentage mode it is `0`. | int ≥ 0, often 0 | 3 independent parsers agree; one outlier treats it as used |
| `current_weekly_total_count` / `current_weekly_usage_count` | Same pair for the weekly window (total / **remaining**). `0` in percentage mode. | int ≥ 0 | live captures |
| `start_time` / `end_time` | Interval window bounds, **epoch milliseconds (UTC)**. `end_time - start_time` = 18,000,000 ms = **5 h** for `general`; 86,400,000 ms = 24 h for `video`. | int64 epoch **ms** | arithmetic on two real captures |
| `weekly_start_time` / `weekly_end_time` | Weekly window bounds, epoch ms. Difference = 604,800,000 ms = **7 days**. Reset time = `weekly_end_time`. | int64 epoch **ms** | arithmetic on real captures |
| `remains_time` | Milliseconds remaining in the interval window. Always `< end_time - start_time`. | int64 **ms** | arithmetic (16,640,796 in an 18,000,000 ms window) |
| `weekly_remains_time` | Milliseconds remaining in the weekly window. | int64 **ms** | live capture |
| `current_interval_status` | Interval window state. Observed: `1` = active/counting. | int | community; semantics UNVERIFIED |
| `current_weekly_status` | Weekly window state. Observed: `1` = active/counting, `2` = weekly quota exhausted (still a real window, show 100 % used), `3` = plan has no weekly limit (`∞`; percent pinned at 100), `0`/absent = window inactive. | int | community (two independent implementations + bug report); exact enum UNVERIFIED |
| `weekly_boost_permille` | Weekly allowance boost in permille (`1500` ⇒ boosted allowance). Not needed to render percent. | int | one real capture; semantics UNVERIFIED |

**Do not invent fields.** The following are only defensive compatibility aliases some clients tolerate,
**never observed in a `/v1/token_plan/remains` response**: `current_interval_used_percent`,
`current_interval_total_percent`, `current_weekly_used_percent`, `current_weekly_total_percent`,
`interval_boost_permille` (note the `permille`/`permill` spelling drift exists in one client).
Treat them as optional; ignore if absent.

### Deriving the two app values

```
session_used_percent = clamp(100 - current_interval_remaining_percent, 0, 100)
weekly_used_percent  = clamp(100 - current_weekly_remaining_percent, 0, 100)

session_reset_at = end_time          (epoch ms → UTC instant)
weekly_reset_at  = weekly_end_time   (epoch ms → UTC instant)

session_window_duration = end_time - start_time          (= 5 h)
weekly_window_duration  = weekly_end_time - weekly_start_time (= 7 d)
```

Fallback only when the percent field is absent and counts are usable:
`used_percent = (current_interval_total_count - current_interval_usage_count) / current_interval_total_count * 100`
— with `usage_count` treated as **remaining**, per the `/remains` contract.

### Parsing algorithm (recommended for the Rust client)

1. Reject non-2xx HTTP except that **a 200 body must still be inspected**.
2. If `base_resp` exists and `status_code != 0` → error path (see §4). Do not render zeros.
3. If `data` is an object, descend one level (cookie-route compatibility).
4. Pick the `model_remains[]` row with `model_name == "general"`. If absent, take the first row
   whose `model_name != "video"`. If only `video` exists → error ("no text bucket"), do not show a
   plan at 0 %.
5. Percent fields are authoritative. `remaining_percent` is **remaining**, so `used = 100 - remaining`.
6. Reset = `end_time` / `weekly_end_time`; epoch **ms**. Guard: values ≤ 0 mean "unreported"; values
   `< 10^12` are probably seconds (defensive *1000).
7. Weekly window may legitimately be absent/non-active (`current_weekly_status` 0 or 3) — render the
   session bar only, never a fake 0 % / 100 % used weekly bar.

### Two gotchas that break naive implementations

- **HTTP status is not the auth signal.** A missing/invalid key returns HTTP **200** with
  `base_resp.status_code` = `1004` / `2049`. Checking only `resp.status` in `reqwest` would parse
  the error envelope as "no rows".
- **The percentages are REMAINING, not USED.** Showing `current_interval_remaining_percent` directly
  in a "used" progress bar gives a nearly-exhausted plan a nearly-full bar.

---

## 4. Errors, rate limits, polling

### Error signal shapes (in-band errors arrive with HTTP 200)

| HTTP | Body | Meaning / app behaviour |
| --- | --- | --- |
| 200 (or 401/403) | `{"base_resp":{"status_code":1004,"status_msg":"login fail: Please carry the API secret key in the 'Authorization' field of the request header"}}` | Missing/invalid key (live-confirmed for all hosts). Show "invalid key", do not clear the last good value silently. |
| 200 | `{"base_resp":{"status_code":2049,"status_msg":"invalid API Key"}}` | Invalid key for this region/host. Try the other region host once. |
| 200 | `{"base_resp":{"status_code":2062,"status_msg":"M Plan rate limit reached ..."}}` | Officially documented as *either* rate limit *or* "no active M Plan subscription when querying usage". Treat as "no subscription" after a retry. |
| 200 | `{"base_resp":{"status_code":2057,...}}` | "not token plan token" — key is a pay-as-you-go key. Show "needs an M Plan key". |
| 200 | `{"base_resp":{"status_code":9001,...}}` | Unknown business error; surface code only (do not leak upstream text into UI logs). |
| 401 / 403 | HTML or empty | Some deployments/gateways front the endpoint with an HTTP auth layer. Map to "invalid/expired key". |
| 429 | usually empty | Throttled. Back off. |
| 404 | `404 page not found` (plain text) | Wrong host/path (e.g. the non-existent `openplatform/token_plan`). Do not mistake it for "no plan". |
| 5xx | HTML (Cloudflare) | Transient. Retry with backoff. |

Documented error codes worth mapping (official error-code page): `1004` auth, `1008` insufficient
balance, `2056` usage limit exceeded (5-hour window), `2057` not token-plan token, `2062` M Plan
rate limit / no active subscription, `2067` usage limit reached, `2068` plan tier doesn't support
model.

### Official documented mappings (verbatim from MiniMax docs)

- `1004` — "not authorized / token not match group / cookie is missing, log in again".
- `2056` — "usage limit exceeded — wait for resource release in the next 5-hour window".
- `2062` — "M Plan rate limit reached; when querying usage, it can also mean there is no active
  M Plan subscription".

### Rate limits & polling (no published limit for this endpoint)

MiniMax publishes RPM/TPM only for inference endpoints (LLM 200 RPM / 10 M TPM). The quota endpoint
has **no documented limit** — but it is a console-shared route, so behave like the official clients:

- Default poll interval **5 minutes** (console and CLI refresh on demand, not continuously).
- Hard floor **60 s**; add ±10 % jitter; never poll per frame/paint.
- Cache the last successful snapshot; on error show `stale` + timestamp, not fabricated numbers.
- Back off on `429`/5xx: 60 s → 120 s → 300 s, cap 15 min; on `1004/2049` stop and mark key invalid
  (no point retrying).
- Use a request timeout of 10–15 s (implementations use 8–15 s) and refuse redirects when the
  Authorization header is present.

---

## 5. Plan naming — how to render "MiniMax Go"

- The subscription product is officially **M Plan**; tiers are **Go**, **Explore**, **Build**
  (Go = 1×, Explore = 3×, Build = 7.5×).
- Tier capabilities relevant to the two bars: text/speech are subject to **both** the 5-hour and
  weekly window; image/video only to the weekly window.
- **The `/v1/token_plan/remains` response does not carry a plan/tier field.** No `plan_name`,
  `plan`, or `current_subscribe_title` has been observed in a live capture, and the two most
  complete open-source clients explicitly refuse to invent one. Plan-tier inference from counts is
  impossible in the current percentage mode because the count fields are `0`.
- Therefore: render the provider name from static app config — `"MiniMax Go"` (or map a
  user-selected tier), not from the API. If a subset of accounts does return
  `plan_name`/`current_subscribe_title`/`plan`, treat any of those strings as optional
  **UNVERIFIED** hints and fall back to the configured label. Do **not** derive the tier from the
  numeric quotas.

---

## 6. Live probe plan (run locally with the real key)

### 6a. Safe probes — no key, confirm routing & error envelope

Bash / Git Bash / WSL:

```bash
# 1) canonical global endpoint (expect HTTP 200 + base_resp.status_code 1004)
curl -sS -i --max-time 15 'https://www.minimax.io/v1/token_plan/remains'
# 2) global API host, then the three CN hosts (all answer the same path)
curl -sS -i --max-time 15 'https://api.minimax.io/v1/token_plan/remains'
curl -sS -i --max-time 15 'https://api.minimaxi.com/v1/token_plan/remains'
curl -sS -i --max-time 15 'https://www.minimaxi.com/v1/token_plan/remains'
curl -sS -i --max-time 15 'https://www.minimax.cn/v1/token_plan/remains'
# 3) legacy API-key path (still routed)
curl -sS -i --max-time 15 'https://www.minimax.io/v1/coding_plan/remains'
# 4) console/cookie route (expect "cookie is missing" — proves it is NOT key auth)
curl -sS -i --max-time 15 'https://www.minimax.io/v1/api/openplatform/coding_plan/remains'
# 5) negative controls (both expect 404 — proves the 1004s above are real routes,
#    not a catch-all)
curl -sS -i --max-time 15 'https://www.minimax.io/v1/api/openplatform/token_plan/remains'
curl -sS -i --max-time 15 'https://www.minimax.io/v1/token_plan/not_remains'
```

PowerShell (note `curl.exe`, not the `curl` alias):

```powershell
curl.exe -sS -i --max-time 15 "https://www.minimax.io/v1/token_plan/remains"
```

Expected for every existing route: `HTTP/2 200` plus
`{"base_resp":{"status_code":1004,"status_msg":"login fail: Please carry the API secret key ..."}}`.

### 6b. Authenticated probe — the one that settles everything

```bash
export MINIMAX_KEY='<M Plan Subscription Key>'
curl -sS -i --max-time 15 \
  -H "Authorization: Bearer $MINIMAX_KEY" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json' \
  'https://www.minimax.io/v1/token_plan/remains'
```

```powershell
$key = Read-Host -AsSecureString "M Plan key"
$k = [Runtime.InteropServices.Marshal]::PtrToStringAuto(
      [Runtime.InteropServices.Marshal]::SecureStringToBSTR($key))
curl.exe -sS -i --max-time 15 `
  -H "Authorization: Bearer $k" `
  -H "Content-Type: application/json" `
  -H "Accept: application/json" `
  "https://www.minimax.io/v1/token_plan/remains"
```

If the key is CN-region, repeat against `https://api.minimaxi.com/v1/token_plan/remains`.

### 6c. What the response must satisfy to confirm this document

1. `base_resp.status_code == 0` and `model_remains` is a non-empty array.
2. A row with `model_name == "general"`.
3. `current_interval_remaining_percent` and `current_weekly_remaining_percent` present (integers 0–100).
4. `end_time - start_time == 18_000_000` ms (±1 s) and
   `weekly_end_time - weekly_start_time == 604_800_000` ms for the `general` row.
5. Compare `100 - current_interval_remaining_percent` against the percentage the console shows for
   the 5-hour bar at `https://platform.minimax.io/console/usage` — they should match within a
   minute.
6. Note whether `current_weekly_status` is present and its value; note whether a plan/tier string
   appears anywhere in the envelope.

### 6d. Open questions for the user's live key (UNVERIFIED until probed)

- Does the M Plan **Go** account return the same field set as the captured response above
  (percent fields present, counts `0`)?
- Is `current_weekly_status` `1` (normal), `2` (exhausted) or `3` (no weekly limit) on Go? Go is
  documented as having both windows, so `1`/`2` is expected.
- Does any plan/tier string appear (`plan_name`, `plan`, `current_subscribe_title`)? If yes, capture
  its exact spelling and affordance for "Go".
- Does the key work on `www.minimax.io` only, or also on `api.minimaxi.com` (region pairing)?
- Exact values of `base_resp.status_code` when the key is throttled vs. when the 5-hour window is
  exhausted (expected `2062` vs `2056`; confirm which appears for the quota *read*).
- Do count fields (`*_total_count`, `*_usage_count`) stay `0` across a usage-heavy period, or do
  they populate on some plans (which would allow a "prompts remaining" secondary line)?

---

## 7. Sources

1. Official FAQ, "How can I check M Plan usage?" (the exact curl for `/v1/token_plan/remains`): https://platform.minimax.io/docs/m-plan/faq · [.md source](https://platform.minimax.io/docs/m-plan/faq.md)
2. Official Usage page — 5-hour / weekly window semantics: https://platform.minimax.io/docs/m-plan/usage-rules
3. Official M Plan Introduction — tiers Go / Explore / Build; image+video are weekly-only: https://platform.minimax.io/docs/m-plan/intro
4. Official API Overview — M Plan "Subscription Key" vs pay-as-you-go API Key: https://platform.minimax.io/docs/api-reference/api-overview
5. Official Error Codes (`1004`, `2049`, `2056`, `2057`, `2062`, `2067`): https://platform.minimax.io/docs/api-reference/errorcode
6. Official Rate Limits: https://platform.minimax.io/docs/guides/rate-limits
7. Official MiniMax CLI docs (`mmx quota`): https://platform.minimax.io/docs/m-plan/minimax-cli
8. Docs index (further pages): https://platform.minimax.io/docs/llms.txt
9. Live probes, no key, 2026-11-24 — all HTTP 200 + `status_code 1004`: https://www.minimax.io/v1/token_plan/remains · https://api.minimax.io/v1/token_plan/remains · https://api.minimaxi.com/v1/token_plan/remains · https://www.minimaxi.com/v1/token_plan/remains · https://www.minimax.cn/v1/token_plan/remains ; 404 control: https://www.minimax.io/v1/api/openplatform/token_plan/remains
10. ai-usagebar (Rust) wire types incl. a verbatim live capture from `api.minimax.io`: https://docs.rs/ai-usagebar/1.12.0/src/ai_usagebar/minimax/types.rs.html
11. tokmesh-cli (Rust) real payload fixture incl. `weekly_boost_permille`: https://docs.rs/tokmesh-cli/0.1.3/src/tokmesh_cli/commands/usage/minimax_tokenplan.rs.html
12. cc-switch (Rust/Tauri) `coding_plan.rs` — `current_weekly_status` 1/2/3, epoch-ms: https://github.com/farion1231/cc-switch/blob/main/src-tauri/src/services/coding_plan.rs
13. cc-switch PR #8124 — keep the exhausted weekly bar visible: https://github.com/farion1231/cc-switch/pull/8124
14. cc-switch issue #8123 — weekly window hidden at `current_weekly_status=2`: https://github.com/farion1231/cc-switch/issues/8123
15. vibeusage (Go) `response.go`/`response_test.go` — real fixtures, epoch-ms, remaining-count semantics: https://github.com/joshuadavidthomas/vibeusage/pull/135
16. CodexBar (Swift) `MiniMaxModelRemains.swift` — compat field aliases: https://github.com/steipete/CodexBar/blob/main/Sources/CodexBarCore/Providers/MiniMax/MiniMaxModelRemains.swift
17. CodexBar `docs/minimax.md` — cookie vs API-token routes, CN console route, region hosts: https://github.com/steipete/CodexBar/blob/main/docs/minimax.md
18. CodexBar PR #149 — Bearer API-token path, GroupId only for browser tokens: https://github.com/steipete/CodexBar/pull/149
19. OpenTokenUsage MiniMax provider doc — CN host + field aliases: https://github.com/PowerUserZ/OpenTokenUsage/blob/main/docs/providers/minimax.md
20. openusage PR #534 — migration `coding_plan` → `token_plan`; percentage-mode response: https://github.com/robinebers/openusage/pull/534
21. dsh-llm-quota provider docs — host→endpoint mapping, ms timestamps, usage_count = remaining: https://github.com/cs-qyzhang/dsh-llm-quota/blob/main/docs/providers.md
22. hermes-quota-plugin `quota_providers/minimax.py` — fetch/parse contract + host fallback: https://github.com/rarf/hermes-quota-plugin/blob/v2.8.2/quota_providers/minimax.py
23. dsh-cost-meter `minimax-quota-endpoint.md` — 2026-09-16 keyless probe of the three hosts; legacy console path: https://github.com/Han-1413141/dsh-cost-meter/blob/main/docs/minimax-quota-endpoint.md
24. M Plan product page: https://www.minimax.io/m-plan
25. Console usage page: https://platform.minimax.io/console/usage
