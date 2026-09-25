// Status label under the crab: current Claude activity + 5-hour rate limit.
// Activity comes from the hook's state.json label; limits from limits.json,
// written by `sidecrab-hook statusline` (Claude Code statusLine command).

const WARN_PCT = 50;
const CRIT_PCT = 80;

export function attachStatus(el) {
  let label = "";
  let limits = null;

  const render = () => {
    el.textContent = "";
    if (label) el.append(label);
    const five = limits?.fiveHour;
    // Claude Code drops a window once it resets; mirror that for stale files.
    if (five && five.resetsAt * 1000 > Date.now()) {
      const pct = Math.round(five.usedPercentage);
      if (label) el.append(" · ");
      const span = document.createElement("span");
      span.textContent = `5h ${pct}%`;
      if (pct >= CRIT_PCT) span.className = "crit";
      else if (pct >= WARN_PCT) span.className = "warn";
      el.append(span);
    }
  };
  setInterval(render, 30_000); // expire the limit segment on reset

  return {
    setState(s) {
      const st = s?.state || "idle";
      label = st === "idle" ? "" : s?.label || "";
      render();
    },
    setLimits(l) {
      limits = l;
      render();
    },
  };
}
