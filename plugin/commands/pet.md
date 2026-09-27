---
description: Summon or toggle the Sidecrab desktop pet (alias of /sidecrab)
allowed-tools: Bash
---
Run this and report only its last line to the user, verbatim:

```bash
BEFORE_PID=$(tasklist //FI "IMAGENAME eq sidecrab.exe" //NH 2>/dev/null | grep -i sidecrab.exe | awk '{print $2}' | head -1)
"${CLAUDE_PLUGIN_ROOT}/bin/sidecrab.exe" --plugin --toggle
# The line above returns almost instantly either way: if Sidecrab wasn't
# running it only spawns a detached process and hands off; if it was, it just
# forwarded this call to the existing instance. Give the window a moment to
# appear/react, without blocking indefinitely — existing settings, position,
# status bar, usage and hooks are never touched by this command.
FLAG="$APPDATA/sidecrab/ui_state.json"
for i in $(seq 1 20); do
  tasklist //FI "IMAGENAME eq sidecrab.exe" 2>/dev/null | grep -qi sidecrab.exe && break
  sleep 0.25
done
if ! tasklist //FI "IMAGENAME eq sidecrab.exe" 2>/dev/null | grep -qi sidecrab.exe; then
  echo "Sidecrab did not start — check that it's installed (install-windows.ps1)."
elif [ -z "$BEFORE_PID" ]; then
  echo "Sidecrab started (existing settings, position and hooks apply)."
elif grep -q '"visible":true' "$FLAG" 2>/dev/null; then
  echo "Sidecrab shown."
else
  echo "Sidecrab hidden (run this again to bring it back)."
fi
```
