// Status panel under the crab, modelled on Claude Code's usage readout:
//   ● OPUS            14:30   (short model name, local 5h reset time)
//   CTX ▮▮▮▮▯▯▯▯▯▯▯▯  34k/200k 17%
//   5H  ▮▮▮▮▮▮▯▯▯▯▯▯          63%
// The DOM is static (index.html) and always visible; values read "--" until
// real data arrives, and every value slot has a fixed width so updates never
// resize the panel.
//
// Sources, in priority order (the cache is only a fallback and a fast start —
// 5h usage also moves with other sessions/devices, so live data always wins):
//   - status dot + activity: hook events (state.json -> claude-state)
//   - model + context: the session record in sessions.d (claude-sessions), fed by
//     the statusLine (terminal) or by the hook reading the transcript (desktop);
//     else the cached last-known context, drawn stale
//   - 5-hour limit (limits.json -> claude-limits): OAuth usage API (rare refresh),
//     statusLine, or the desktop app's own samples (reset time then estimated,
//     shown "~"); else the cached value, drawn stale
// Stale = dimmed value plus a trailing "?".
// Compact mode (the default): the panel collapses into a mini plate — dot, short
// model name, 5h reset time — and grows back out of it from the centre while
// the cursor is over the strip (see setCompact/setHover).

const WARN_PCT = 50;
const CRIT_PCT = 80;
// Working with no hook event for this long = probably stuck (a single tool call
// such as a long build can legitimately run for minutes, hence generous).
const STUCK_S = 10 * 60;
// A 5h reading older than this is drawn stale even without a failed refresh.
const LIMIT_STALE_S = 45 * 60;
// Compact mode timing: linger after the cursor leaves; show the full panel for
// a moment at startup, and collapse shortly after the setting is switched on.
const COLLAPSE_DELAY_MS = 400;
const STARTUP_PEEK_MS = 3000;
const ENABLE_PEEK_MS = 500;

/// 12540 -> "12.5k", 378045 -> "378k", 1000000 -> "1M": at most 5 chars.
export function compactTokens(n) {
  if (n == null || !isFinite(n)) return "--";
  if (n >= 1e6) return +(n / 1e6).toFixed(1) + "M";
  if (n >= 99_950) return Math.round(n / 1e3) + "k"; // 99950 would round to "100.0k"
  if (n >= 1e3) return +(n / 1e3).toFixed(1) + "k"; // 34000 -> "34k", not "34.0k"
  return String(Math.round(n));
}

/// "Opus 5.5" / "Sonnet 5" / "claude-haiku-4-5" -> "OPUS" / "SONNET" / "HAIKU".
/// Known families win wherever they appear; otherwise the first word that is not
/// "claude" and has no digits ("Gemini 2" -> "GEMINI").
export function shortModel(name) {
  const words = String(name || "").toLowerCase().match(/[a-z]+/g) || [];
  const family = words.find((w) => ["opus", "sonnet", "haiku"].includes(w));
  return (family || words.find((w) => w !== "claude") || "claude").toUpperCase();
}

/// Epoch seconds -> local wall clock "14:30".
export function clockText(epochS) {
  const d = new Date(epochS * 1000);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/// Dot colour from Claude's work status only — never from token percentages.
export function dotStatus(state, nowS = Date.now() / 1000) {
  const st = state?.state || "idle";
  if (st === "error") return "error";
  if (st === "permission") return "attention";
  if (st === "thinking" || st === "tool") {
    return state.ts && nowS - state.ts > STUCK_S ? "error" : "working";
  }
  return "idle";
}

/// The usage_cache.json document -> the same shapes the live sources produce.
export function fromCache(cache) {
  const c = cache || {};
  return {
    limits:
      c.fiveHourUsed != null && c.fiveHourResetTime != null
        ? {
            fiveHour: { usedPercentage: c.fiveHourUsed, resetsAt: c.fiveHourResetTime },
            estimated: !!c.fiveHourEstimated,
            ts: c.fiveHourUpdated || 0,
          }
        : null,
    context:
      c.contextUsed != null
        ? { tokens: c.contextUsed, contextSize: c.contextMax, contextPct: c.contextPercentage, model: c.model }
        : null,
  };
}

const level = (p) => (p >= CRIT_PCT ? "crit" : p >= WARN_PCT ? "warn" : "");

export function attachStatus(el) {
  const $ = (sel) => el.querySelector(sel);
  const dot = $(".row .dot");
  const what = $(".what");
  const resetRow = $(".rs");
  const miniTime = $(".mini .mt");
  const mini = $(".mini");
  const miniDot = $(".mini .dot");
  const miniModel = $(".mini .m");
  const ctx = $('[data-kind="ctx"]');
  const lim = $('[data-kind="lim"]');

  let state = { state: "idle" };
  let sessions = [];
  let limits = null; // live (limits.json)
  let cached = { limits: null, context: null };

  // The frame collapses into (and grows out of) the mini plate: scale it to the
  // plate's size. offsetWidth is layout size, unaffected by the transforms.
  const fitFrame = () => {
    const w = el.offsetWidth, h = el.offsetHeight;
    if (!w || !h) return;
    el.style.setProperty("--kx", (mini.offsetWidth / w).toFixed(3));
    el.style.setProperty("--ky", (mini.offsetHeight / h).toFixed(3));
  };
  window.addEventListener("resize", fitFrame); // Size S/M/L

  const meter = (row, pct, stale) => {
    const known = pct != null && isFinite(pct);
    const p = known ? Math.max(0, Math.min(100, pct)) : 0;
    row.querySelector(".fill").style.width = `${p}%`;
    row.querySelector(".v").textContent = known ? `${Math.round(pct)}%${stale ? "?" : ""}` : "--%";
    row.dataset.level = known ? level(pct) : "";
    row.dataset.stale = known && stale ? "1" : "";
  };

  const render = () => {
    const now = Date.now() / 1000;
    // Session shown = the one the last hook event came from, else the most recent.
    const own = sessions.find((x) => x.id === state.sessionId);
    const live = own || sessions.find((x) => x.tokens != null) || sessions[0] || null;
    // A state left behind by a session that has since died is not "working".
    const st = state.sessionId && !own ? "idle" : dotStatus(state, now);
    dot.dataset.status = st;
    miniDot.dataset.status = st;
    el.dataset.dot = st; // state colour of the plate's dot

    // Context: live session record, else cached last-known (stale).
    const fromLive = live && live.tokens != null;
    const c = fromLive ? live : cached.context || {};
    const model = (live?.model || c.model || "Claude").toUpperCase();
    const short = shortModel(model);
    what.textContent = short;

    const size = c.contextSize;
    const tok = c.tokens == null ? "--" : size ? `${compactTokens(c.tokens)}/${compactTokens(size)}` : compactTokens(c.tokens);
    ctx.querySelector(".tok").textContent = tok;
    meter(ctx, c.contextPct ?? (c.tokens != null && size ? (c.tokens / size) * 100 : null), !fromLive);

    // 5h: live limits, else the cache. A window already past its reset is unknown.
    const src = limits || cached.limits;
    const five = src?.fiveHour;
    const valid = five && five.resetsAt > now;
    const stale = !limits || limits.stale === true || (src.ts && now - src.ts > LIMIT_STALE_S);
    meter(lim, valid ? five.usedPercentage : null, valid && stale);
    // Reset time comes straight from the source's resetsAt (never computed here);
    // "~" marks the desktop-sample estimate, "--:--" = no live window.
    const reset = valid ? (src.estimated ? "~" : "") + clockText(five.resetsAt) : "--:--";
    resetRow.textContent = reset;
    resetRow.dataset.stale = valid && stale ? "1" : "";
    // The plate is as wide as its content: refit the collapsed frame when it changed.
    if (miniModel.textContent !== short || miniTime.textContent !== reset) {
      miniModel.textContent = short;
      miniTime.textContent = reset;
      miniTime.dataset.stale = resetRow.dataset.stale;
      fitFrame();
    }
  };
  render();
  fitFrame();
  setInterval(render, 15_000); // stuck detection, staleness, window expiry

  // Compact mode. Expanded while the cursor is over the strip; collapses a beat
  // after it leaves (a brief slip off the edge doesn't flicker it).
  let compact = false;
  let hover = false;
  let collapseTimer = null;
  const setMini = (on) => {
    el.dataset.mini = on ? "1" : "";
  };
  const collapseSoon = (ms) => {
    clearTimeout(collapseTimer);
    collapseTimer = setTimeout(() => setMini(true), ms);
  };
  let started = false;

  return {
    setCompact(on) {
      compact = !!on;
      clearTimeout(collapseTimer);
      setMini(false);
      if (compact && !hover) collapseSoon(started ? ENABLE_PEEK_MS : STARTUP_PEEK_MS);
      started = true;
    },
    setHover(on) {
      hover = !!on;
      if (!compact) return;
      clearTimeout(collapseTimer);
      if (hover) setMini(false);
      else collapseSoon(COLLAPSE_DELAY_MS);
    },
    setState(s) {
      state = s || { state: "idle" };
      render();
    },
    setSessions(list) {
      sessions = Array.isArray(list) ? list : [];
      render();
    },
    setLimits(l) {
      limits = l && l.fiveHour ? l : null;
      render();
    },
    setCache(cache) {
      cached = fromCache(cache);
      render();
    },
  };
}
