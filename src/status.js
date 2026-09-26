// Status panel under the crab, modelled on Claude Code's usage readout:
//   ● activity/model            378k/1M
//   ctx ▮▮▮▮▯▯▯▯▯▯▯▯▯▯▯▯▯▯▯      38%
//   5h  ▮▮▯▯▯▯▯▯▯▯▯▯▯▯▯  15%  4h37m
// The DOM is static (index.html) and always visible; values read "--" until
// real data arrives, and every segment has a fixed width so updates never
// resize the panel.
//
// Hybrid sources:
//   - status dot + activity: hook events (state.json -> claude-state)
//   - model + context tokens: the session's sessions.d record, filled by the
//     statusLine (terminal) or, without one (desktop app), by the hook reading
//     the transcript (claude-sessions)
//   - 5-hour limit (limits.json -> claude-limits): OAuth usage API or statusLine
//     (exact reset time), else the desktop app's own usage samples (percentage
//     exact, reset time estimated and shown with "~"); "--" when none is current

const WARN_PCT = 50;
const CRIT_PCT = 80;
// Working with no hook event for this long = probably stuck (a single tool call
// such as a long build can legitimately run for minutes, hence generous).
const STUCK_S = 10 * 60;

/// 12540 -> "12.5k", 378045 -> "378k", 1000000 -> "1M": at most 5 chars.
export function compactTokens(n) {
  if (n == null || !isFinite(n)) return "--";
  if (n >= 1e6) return +(n / 1e6).toFixed(1) + "M";
  if (n >= 99_950) return Math.round(n / 1e3) + "k"; // 99950 would round to "100.0k"
  if (n >= 1e3) return (n / 1e3).toFixed(1) + "k";
  return String(Math.round(n));
}

/// Seconds until reset -> "4h37m" / "12m".
export function untilText(resetsAtS, nowS = Date.now() / 1000) {
  const s = Math.max(0, resetsAtS - nowS);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return h > 0 ? `${h}h${String(m).padStart(2, "0")}m` : `${m}m`;
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

const level = (p) => (p >= CRIT_PCT ? "crit" : p >= WARN_PCT ? "warn" : "");

export function attachStatus(el) {
  const $ = (sel) => el.querySelector(sel);
  const dot = $(".dot");
  const what = $(".what");
  const tok = $(".tok");
  const ctx = $('[data-kind="ctx"]');
  const lim = $('[data-kind="lim"]');

  let state = { state: "idle" };
  let sessions = [];
  let limits = null;

  const meter = (row, pct) => {
    const known = pct != null && isFinite(pct);
    const p = known ? Math.max(0, Math.min(100, pct)) : 0;
    row.querySelector(".fill").style.width = `${p}%`;
    row.querySelector(".v").textContent = known ? `${Math.round(pct)}%` : "--%";
    row.dataset.level = known ? level(pct) : "";
  };

  const render = () => {
    const now = Date.now() / 1000;
    // Session shown = the one the last hook event came from, else most recent.
    const own = sessions.find((x) => x.id === state.sessionId);
    const s = own || sessions[0] || {};
    // A state left behind by a session that has since died is not "working".
    const st = state.sessionId && !own ? "idle" : dotStatus(state, now);
    dot.dataset.status = st;
    what.textContent = st !== "idle" && state.label ? state.label : s.model || "Claude";

    const size = s.contextSize;
    tok.textContent =
      s.tokens == null
        ? "--"
        : size
          ? `${compactTokens(s.tokens)}/${compactTokens(size)}`
          : compactTokens(s.tokens);
    meter(ctx, s.contextPct ?? (s.tokens != null && size ? (s.tokens / size) * 100 : null));

    // Claude Code drops a window once it resets; mirror that for stale data.
    const five = limits?.fiveHour;
    const live = five && five.resetsAt > now;
    meter(lim, live ? five.usedPercentage : null);
    lim.querySelector(".t").textContent = live
      ? (limits.estimated ? "~" : "") + untilText(five.resetsAt, now)
      : "--";
  };
  render();
  setInterval(render, 15_000); // stuck detection, reset countdown, expiry

  return {
    setState(s) {
      state = s || { state: "idle" };
      render();
    },
    setSessions(list) {
      sessions = Array.isArray(list) ? list : [];
      render();
    },
    setLimits(l) {
      limits = l;
      render();
    },
  };
}
