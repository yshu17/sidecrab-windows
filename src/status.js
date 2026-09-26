// Status bar under the crab: ● status dot · activity/model · tokens · context % ·
// 5-hour limit. The DOM is static (index.html) and always visible; values show
// "--" until real data arrives, and segments have fixed widths so updates never
// resize the bar.
//
// Sources: activity from the hook's state.json (claude-state); model, token
// count and context % from the session record in sessions.d/ (claude-sessions,
// filled by `sidecrab-hook statusline` from Claude Code's statusLine JSON);
// the 5-hour limit from limits.json (claude-limits).

const WARN_PCT = 50;
const CRIT_PCT = 80;
// Working with no hook event for this long = probably stuck (a single tool call
// such as a long build can legitimately run for minutes, hence generous).
const STUCK_S = 10 * 60;

/// 12540 -> "12.5k", 148000 -> "148k": never more than 5 chars (fixed slot).
export function compactTokens(n) {
  if (n == null || !isFinite(n)) return "--";
  if (n >= 1e6) return (n / 1e6).toFixed(1) + "M";
  if (n >= 99_950) return Math.round(n / 1e3) + "k"; // 99950 would round to "100.0k"
  if (n >= 1e3) return (n / 1e3).toFixed(1) + "k";
  return String(Math.round(n));
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

const pctClass = (p) => (p >= CRIT_PCT ? "crit" : p >= WARN_PCT ? "warn" : "");

export function attachStatus(el) {
  const $ = (sel) => el.querySelector(sel);
  const dot = $(".dot");
  const what = $(".what");
  const tok = $(".tok");
  const ctx = $(".ctx");
  const lim = $(".lim");

  let state = { state: "idle" };
  let sessions = [];
  let limits = null;

  const render = () => {
    // Session shown = the one the last hook event came from, else most recent.
    const own = sessions.find((x) => x.id === state.sessionId);
    const s = own || sessions[0] || {};
    // A state left behind by a session that has since died is not "working".
    const stale = state.sessionId && !own;
    const st = stale ? "idle" : dotStatus(state);
    dot.dataset.status = st;
    const busy = st !== "idle" && state.label;
    what.textContent = busy ? state.label : s.model || "Claude";

    tok.textContent = compactTokens(s.tokens);
    const cp = s.contextPct;
    ctx.textContent = cp == null ? "--%" : `${Math.round(cp)}%`;
    ctx.className = "ctx " + (cp == null ? "" : pctClass(cp));

    // Claude Code drops a window once it resets; mirror that for stale files.
    const five = limits?.fiveHour;
    const live = five && five.resetsAt * 1000 > Date.now();
    const lp = live ? Math.round(five.usedPercentage) : null;
    lim.textContent = lp == null ? "5h --" : `5h ${lp}%`;
    lim.className = "lim " + (lp == null ? "" : pctClass(lp));
  };
  render();
  setInterval(render, 15_000); // stuck detection + limit expiry

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
