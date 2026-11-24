# DeepSeek Open Platform balance API — verified research

Status date: **2026-11-24**. Everything marked **VERIFIED** was checked against a primary source
(`api-docs.deepseek.com`, the official OpenAPI-rendered reference) or a live network probe on that
date. Everything else is **UNVERIFIED** with the exact probe that would settle it.

Purpose: read the **prepaid money balance** of a DeepSeek Open Platform account for the Quotty
"DeepSeek" provider. This is **not** a subscription and has **no 5-hour / weekly quota windows** —
see §6.

---

## 1. Summary

| Item | Value | Status |
| --- | --- | --- |
| Endpoint | `GET https://api.deepseek.com/user/balance` | **VERIFIED** — official reference `GET /user/balance`, server `https://api.deepseek.com` |
| Method | `GET` only (no documented POST/HEAD) | **VERIFIED** |
| Query params | none | **VERIFIED** (none in reference/code samples) |
| Request body | none | **VERIFIED** |
| Auth | `Authorization: Bearer <OPEN PLATFORM api key>` | **VERIFIED** |
| Other headers | none required. (`Accept: application/json` harmless.) No `Content-Type` needed on GET. | **VERIFIED** (reference shows only Authorization) |
| Key type | Open Platform API key created at `https://platform.deepseek.com/api_keys` | **VERIFIED** — this is **not** the MiniMax M Plan key, not a web-session token; app-login inference tokens get 401 (community-verified, see §2) |
| Response | JSON: `is_available` (bool) + `balance_infos[]` of money fields as **decimal strings** | **VERIFIED** |
| Currency | `CNY` or `USD` (`enum`) | **VERIFIED** |
| Trailing slash | `/user/balance/` — cannot be verified without a key (auth runs before routing); use no trailing slash | **UNVERIFIED** |
| `/v1/user/balance` variant | Docs document only `/user/balance`. `/v1/user/balance` answers 401 exactly like unknown paths do without a key, so it is **not** evidence of existence | **UNVERIFIED — do not use** |
| Error channel | Real HTTP status codes (401/402/429/5xx), **not** a 200 body envelope | **VERIFIED for 401 (live)**; other bodies UNVERIFIED |
| Windows / percent | none — it is a money balance | **VERIFIED by absence** (§6) |

Live probe (no key attached, 2026-11-24):

```
GET https://api.deepseek.com/user/balance
HTTP/1.1 401
{"error":{"message":"Authentication Fails (governor) (request_id: eb26199e-91ed-4534-9723-d0c3d5b36b02)","type":"authentication_error","param":null,"code":"invalid_request_error"}}
```

---

## 2. Auth

```http
GET /user/balance HTTP/1.1
Host: api.deepseek.com
Authorization: Bearer <Open Platform API key>
Accept: application/json
```

- **Exactly one credential header:** `Authorization: Bearer <key>`. **VERIFIED** by the reference's
  `bearerAuth` scheme and its curl/Python samples.
- **No `GroupId`, no account id, no organization header, no signing.** The key alone identifies the
  account. (Contrast: MiniMax's quota API — also Bearer — but a totally different key family and
  product.)
- Key source: `https://platform.deepseek.com/api_keys`. Keys are **account-scoped**, so one key
  reads the account's wallet; there is no per-key sub-wallet in the documented schema.
- The **inference/session token of the DeepSeek app account is not an Open Platform API key**;
  sending it as `Authorization: Bearer` to `/user/balance` returns **401**. (Community-verified in
  `dsh-cost-meter` docs; consistent with our live 401 shape.) The app must read the Open Platform key.
- A **402** on inference calls means the wallet is empty — that is a billing state, not an auth
  failure. Do not map 402 to "bad key".

---

## 3. Response schema (exact)

Reference: `GetUserBalanceResponse`. **All properties are optional** in the schema — parse
defensively. Money values are **strings**, not JSON numbers.

```json
{
  "is_available": true,
  "balance_infos": [
    {
      "currency": "CNY",
      "total_balance": "110.00",
      "granted_balance": "10.00",
      "topped_up_balance": "100.00"
    }
  ]
}
```

| Field | Type | Meaning (verbatim docs sense) | Notes |
| --- | --- | --- | --- |
| `is_available` | `boolean` | "Whether the user's balance is sufficient for API calls." | `false` ⇒ account cannot make calls. It is a *sufficiency flag*, not a "balance loaded" flag. Still render the amounts when `false`. |
| `balance_infos` | `array<object>` | Wallet entries, typically one per currency. | Documented as an array; **no documented ordering guarantee**. |
| `[] .currency` | `string` enum `CNY`, `USD` | "The currency of the balance." | Only these two values documented. |
| `[] .total_balance` | `string` (nullable: false) | "The total available balance, **including** the granted balance and the topped-up balance." | Decimal string, 2 dp in the example. **Display this number.** |
| `[] .granted_balance` | `string` | "The total not expired granted balance." (赠金 — promotional credit) | Decimal string. Expires; not the same as top-up. |
| `[] .topped_up_balance` | `string` | "The total topped-up balance." (充值余额 — money you paid) | Decimal string. |

### Interpretation rules

- **Strings, not numbers.** `"110.00"` — parse with a decimal (`rust_decimal`/`f64` after trim);
  do **not** expect `110.0`. Beware locale/precision: treat the string as authoritative for display,
  parse only for sorting/comparison.
- **`total_balance` includes both sub-balances.** The schema says total = granted + topped-up; the
  example satisfies it (`110.00 = 10.00 + 100.00`). Whether the API guarantees exact cent-level
  arithmetic equality on every account is **UNVERIFIED** — render `total_balance` as the truth and use
  the two components only as a breakdown.
- **Multiple entries:** documented as an array, so a wallet may carry more than one currency entry.
  Whether a single account ever returns **both** CNY and USD simultaneously is **UNVERIFIED** (no
  dual example published). Community plugins do "multi-currency selection", implying it happens or
  is expected. **Do not assume `balance_infos[0]` is the right entry** — select explicitly:
  prefer `CNY` for a CN account / `USD` for an international account, else the first entry, and show
  the currency code in the UI. Never sum different currencies.
- **`is_available` is not `total_balance > 0`.** Trust the flag for "can call the API"; show the
  numbers regardless.
- **Amounts are money** (CNY or USD), not tokens, not percent. There is no denominator, no limit,
  no window; a progress bar would be fabricated.

### Recommended Rust shape

```rust
#[derive(serde::Deserialize)]
struct BalanceEnvelope {
    #[serde(default)] is_available: Option<bool>,
    #[serde(default)] balance_infos: Vec<BalanceInfo>,
}
#[derive(serde::Deserialize)]
struct BalanceInfo {
    #[serde(default)] currency: Option<String>,
    #[serde(default)] total_balance: Option<String>,
    #[serde(default)] granted_balance: Option<String>,
    #[serde(default)] topped_up_balance: Option<String>,
}
```

Keep every field `Option`/`default`: the schema marks all four optional, and a partial payload must
degrade to "no data", not to a fabricated `0.00`.

---

## 4. Errors, rate limits, polling

### Status codes (official error-code page; semantics authoritative)

| HTTP | Official meaning | App behaviour |
| --- | --- | --- |
| **401** | "Authentication fails due to the wrong API key." | **VERIFIED live body** (below). Map to "invalid/expired key — check the Open Platform key". Do not retry in a loop. |
| **402** | "Insufficient Balance — you have run out of balance." | Wallet empty. On a *balance read* it may or may not appear; if it does, show "balance exhausted" (and still show last known amount). Not an auth error. |
| **429** | "Rate Limit Reached — too many requests, or concurrency/QPS limit." | Back off (see polling). |
| **500** | Server error — retry after a brief wait. | Retry with backoff. |
| **503** | Server overloaded — retry after a brief wait. | Retry with backoff. |
| **504** | Gateway timeout — retry later. | Retry with backoff. |
| 400 / 422 / 408 / 413 | Request-shape errors for other endpoints; not expected for this `GET` with no body. | Treat as programming error; log status only. |

**401 body — VERIFIED live (2026-11-24), exact shape:**

```json
{
  "error": {
    "message": "Authentication Fails (governor) (request_id: eb26199e-91ed-4534-9723-d0c3d5b36b02)",
    "type": "authentication_error",
    "param": null,
    "code": "invalid_request_error"
  }
}
```

Notes that matter for parsing:
- OpenAI-style envelope: `error.message` / `error.type` / `error.param` / `error.code`.
- **`code` is `"invalid_request_error"` even for auth failures.** Do **not** key on `code`; key on
  the HTTP status and on `type == "authentication_error"`.
- `request_id` is not a separate field — it is embedded in the human-readable `message` string.
  Don't parse it; if you log, log the status only (the message may contain request metadata).
- The `(governor)` vs `(missing key)` wording differs depending on whether a key was supplied.

**402 / 429 / 5xx body shapes: UNVERIFIED.** DeepSeek publishes the status codes and causes but
**does not publish response bodies** for them (the reference documents only the 200 response). The
`{"error":{...}}` envelope is consistent across what we can observe (401 live-verified) and is the
OpenAI-compatible convention DeepSeek follows, so a tolerant parser should read `error.message` /
`error.type` if present and otherwise fall back to the status code. Probe in §7d to confirm.

### Rate limits & polling

- **No documented rate limit exists for `/user/balance`.** The Rate Limit page documents only model
  **concurrency** limits (`deepseek-flash` 2500, `deepseek-v4-pro` 500 concurrent requests; exceeding
  → HTTP 429) and states they are "calculated at the account level, regardless of which API Key is
  used". A balance read is not a model request and is not counted by that model-concurrency rule.
- **Nothing in the docs forbids a 5-minute poll.** At 5 min that is 288 requests/day — negligible.
  A conservative hard floor of **60 s** is recommended for UI freshness; community balance plugins
  poll as fast as 30 s, which is not documented as a problem but is unnecessary.
- Recommended: **default 5 min**, floor 60 s, ±10 % jitter, exponential backoff on 429/5xx
  (60 s → 300 s, cap 15 min), request timeout 10–15 s, cache the last good value with a timestamp,
  never overwrite a good value with an error, and never render `0.00` for "unknown".
- No pagination, no ETag/Last-Modified documented — plain full read each time.

---

## 5. Spend / usage history: the honest limitation

- The **API Reference index contains no usage, spend, invoice, or transaction endpoint.** The only
  account-level endpoint is `GET /user/balance`
  (Chat Completions, Responses, Messages, FIM, List Models, Get User Balance, Files, Tools).
- Therefore: **there is no documented public API to read spend history or per-key usage.** Balance is
  the only per-key number obtainable programmatically today.
- The **web console** (`https://platform.deepseek.com`) exposes usage/consumption pages for humans
  (the docs link the console for API keys and top-up; a usage page exists there). That is a browser
  session feature, not an API — do not scrape it from the desktop app.
- Per-request token counts are returned inside each **inference response** (`usage` object:
  `prompt_tokens`, `completion_tokens`, `total_tokens`, cache hit/miss split), which lets an app
  estimate cost locally, but that is not `user/balance` and is not needed here.
- Consequence for Quotty: for DeepSeek, render **balance (money) only** — optionally a local
  "balance trend" chart built from your own sampled balances, clearly labelled as sampled, not
  server-reported spend.

**UNVERIFIED**: whether an undocumented usage endpoint exists. The probe in §7e checks the console
API only if the user explicitly wants it; do not ship reliance on it.

---

## 6. No 5-hour / weekly windows — evidence

The DeepSeek Open Platform is **prepaid money**, not a subscription plan:

1. **Billing model is per-token deduction from a wallet.** Official pricing page: "We bill model
   usage based on the total number of input and output tokens"; "The corresponding fees will be
   directly deducted from your topped-up balance or granted balance, with a preference for using the
   granted balance first when both balances are available." No quota period, no window, no reset.
2. **The only account state endpoint is a money balance** (`total_balance` / `granted_balance` /
   `topped_up_balance`), with a currency enum. There is no window field, no percentage, no reset
   timestamp, no plan tier.
3. **Out-of-quota is expressed as a money state, not a window:** HTTP 402 "Insufficient Balance —
   you have run out of balance", not "5-hour limit reached".
4. **Rate limiting is concurrency-based and account-level**, not a 5-hour/weekly allowance; the docs
   list no rolling usage windows anywhere.
5. No plan/tier product is documented for the API. Marketing/console may offer top-up and granted
   credits (赠金, expiring), but **nothing analogous to MiniMax M Plan's Go/Explore/Build windows**
   exists in the DeepSeek Open Platform docs.

⇒ **Do not render any 5-hour or weekly bar for DeepSeek.** A "percent used" bar would have to invent
a denominator. Render a balance chip (amount + currency), and if you must show a bar, do it only
against a **user-configured** low-balance threshold, labelled as such.

> Disambiguation: third-party resellers (OpenCode Go, Command Code, WorkBuddy, etc.) sell
> $10/month plans *backed by* DeepSeek Flash tokens and those plans do have 5-hour/weekly walls.
> They are **not** the DeepSeek Open Platform and do not use `api.deepseek.com/user/balance`; do not
> let a user's mention of "a DeepSeek plan with a weekly limit" change the DeepSeek provider's
> balance-only rendering.

---

## 7. Live probe plan (run locally with the real key)

Safety rules: read the key from a file or env var, **never echo it**, never paste it in a shell that
logs history, and never use `curl -v` (verbose mode prints the `Authorization` request header).
`curl -i` prints only the response.

### 7a. Safe probe — no key (confirms routing + exact 401 envelope)

```bash
curl -sS -i --max-time 15 'https://api.deepseek.com/user/balance'
```

```powershell
curl.exe -sS -i --max-time 15 "https://api.deepseek.com/user/balance"
```

Expected: `HTTP/1.1 401` and the `{"error":{"message":"Authentication Fails ...","type":
"authentication_error","param":null,"code":"invalid_request_error"}}` body. This proves reachability
and the error envelope without any credential.

### 7b. Authenticated read — key from a file (bash)

```bash
# one-time: printf '%s' '<KEY>' > ~/.deepseek.key && chmod 600 ~/.deepseek.key
KEY=$(tr -d '\r\n' < "$HOME/.deepseek.key")
curl -sS --max-time 15 \
  --header "Authorization: Bearer ${KEY}" \
  --header 'Accept: application/json' \
  'https://api.deepseek.com/user/balance'
unset KEY
```

### 7c. Authenticated read — key from env (PowerShell, Windows)

```powershell
# Put the key in the user environment once:
#   [Environment]::SetEnvironmentVariable("DEEPSEEK_API_KEY", "<KEY>", "User")
# or set it for this process only — do not hardcode it in scripts.
$k = $env:DEEPSEEK_API_KEY
curl.exe -sS --max-time 15 -H "Authorization: Bearer $k" -H "Accept: application/json" "https://api.deepseek.com/user/balance"
```

(Reads `$env:DEEPSEEK_API_KEY`; the value is never printed. If you prefer a file, use
`Get-Content "$env:USERPROFILE\.deepseek.key" -Raw` and `.Trim()` it into `$k`.)

### 7d. Optional error-shape probes (documented codes without published bodies)

```bash
# 429 body: burst ~30 reads with a valid key and keep only 429 responses
for i in $(seq 1 30); do
  curl -sS -o /tmp/ds_$i.body -w '%{http_code}\n' --max-time 5 \
    -H "Authorization: Bearer ${KEY}" 'https://api.deepseek.com/user/balance'
done
grep -l . /tmp/ds_*.body >/dev/null; # inspect any non-200 body; DELETE the files afterwards
```

Expected: probably all 200 (no per-endpoint limit is documented); if a 429 appears, capture its body
verbatim. **402** can effectively only be observed on an empty wallet — do not empty the wallet to
test; infer behaviour from the status code instead.

### 7e. What the response must satisfy to confirm this document

1. HTTP 200, `Content-Type: application/json`.
2. `is_available` is a JSON **boolean**; `balance_infos` is an **array**.
3. Every money value is a JSON **string** (`"110.00"`), not a number.
4. `currency` ∈ {`CNY`, `USD`}.
5. `total_balance` equals `granted_balance + topped_up_balance` (check; report if it does not).
6. Record how many `balance_infos` entries the account has, their order, and currencies.
7. Note the effect of `is_available` after a small spend (does it flip at 0, or earlier?).

---

## 8. Open questions for the user's live key (UNVERIFIED until probed)

- Does the wallet return **one** entry or several? If several, which order and which currency pair?
- Is `total_balance` exactly `granted_balance + topped_up_balance` to the cent, always?
- What does `is_available` do at exactly 0 (and does a granted-only balance count as available)?
- What are the exact 402 and 429 response bodies (for user-facing message mapping)?
- Does `/user/balance` accept a trailing slash / any query params for the user's deployment?
- Is the account's currency CNY or USD, and does that follow the key's region or the account's
  billing setup? (Affects which entry the app prefers.)
- Does an expired granted balance (`granted_balance: "0.00"`) still return the field, or omit it?

---

## 9. Sources

1. Official reference — Get User Balance (`GET /user/balance`, bearer auth, full schema + example): https://api-docs.deepseek.com/api/get-user-balance/
2. Same page, markdown/OpenAPI-rendered source: https://api-docs.deepseek.com/api/get-user-balance.md
3. Chinese reference (same schema; 赠金/充值 wording): https://api-docs.deepseek.com/zh-cn/api/get-user-balance/
4. Official Error Codes (401/402/429/500/503/504 definitions): https://api-docs.deepseek.com/quick_start/error_codes/
5. Official Models & Pricing (per-token billing, deduction from topped-up/granted balance): https://api-docs.deepseek.com/quick_start/pricing/
6. Official Rate Limit & Isolation (concurrency limits, 429; no per-endpoint balance limit): https://api-docs.deepseek.com/quick_start/rate_limit/
7. Official Token & Token Usage (tokens as billing unit): https://api-docs.deepseek.com/quick_start/token_usage/
8. Official "Your First API Call" (base_url `https://api.deepseek.com`, key from platform.deepseek.com): https://api-docs.deepseek.com/quick_start/your_first_api_call/
9. Official API Reference index (no usage/spend endpoint): https://api-docs.deepseek.com/api/create-chat-completion/
10. Live probes, no key, 2026-11-24: `https://api.deepseek.com/user/balance` → HTTP 401 +
    `{"error":{"message":"Authentication Fails (governor) ...","type":"authentication_error","param":null,"code":"invalid_request_error"}}`;
    `https://api.deepseek.com/user/balance/` and `https://api.deepseek.com/v1/user/balance` → same 401
    (auth runs before routing, so these are **not** evidence the variants exist).
11. dsh-cost-meter balance-credentials doc — app-login inference token gets 401 on `/user/balance`;
    Open Platform key is a distinct credential: https://github.com/Han-1413141/dsh-cost-meter/blob/main/docs/balance-credentials.md
12. DeepSeek platform console (API keys, top-up pages linked from official docs): https://platform.deepseek.com/
13. DeepSeek API keys page: https://platform.deepseek.com/api_keys
14. DeepSeek top-up page: https://platform.deepseek.com/top_up
15. DeepSeek API status: https://status.deepseek.com/
