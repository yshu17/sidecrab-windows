---
description: Turn the Sidecrab desktop pet on/off, or show its state (alias of /sidecrab)
allowed-tools: Bash
---
The user's argument, if any, is: $ARGUMENTS

Sidecrab is off by default and never launches on its own — only this command
starts it. Pick exactly one script below based on the argument (case-insensitive)
and run it, then report only its last line to the user, verbatim — no extra
commentary:

- empty argument -> run "Status"
- `on` -> run "On"
- `off` -> run "Off"
- anything else -> tell the user only `on`, `off`, or no argument (status) are valid; run nothing

Status:
```bash
tasklist //FI "IMAGENAME eq sidecrab.exe" 2>/dev/null | grep -qi sidecrab.exe && echo "Pet: ON" || echo "Pet: OFF"
```

On (starts Sidecrab if it isn't running yet, or shows/re-arms it if it already is — never a second instance):
```bash
"${CLAUDE_PLUGIN_ROOT}/bin/sidecrab.exe" --plugin --show
for i in $(seq 1 20); do
  tasklist //FI "IMAGENAME eq sidecrab.exe" 2>/dev/null | grep -qi sidecrab.exe && break
  sleep 0.25
done
if tasklist //FI "IMAGENAME eq sidecrab.exe" 2>/dev/null | grep -qi sidecrab.exe; then
  echo "Pet: ON"
else
  echo "Pet: ON requested, but it did not start — check it's installed (install-windows.ps1)."
fi
```

Off (stops the Sidecrab process entirely — no window, no hooks doing real work, no usage-API calls until /pet on again):
```bash
if ! tasklist //FI "IMAGENAME eq sidecrab.exe" 2>/dev/null | grep -qi sidecrab.exe; then
  echo "Pet: OFF"
else
  "${CLAUDE_PLUGIN_ROOT}/bin/sidecrab.exe" --quit
  for i in $(seq 1 20); do
    tasklist //FI "IMAGENAME eq sidecrab.exe" 2>/dev/null | grep -qi sidecrab.exe || break
    sleep 0.25
  done
  if tasklist //FI "IMAGENAME eq sidecrab.exe" 2>/dev/null | grep -qi sidecrab.exe; then
    echo "Pet: OFF requested, still shutting down."
  else
    echo "Pet: OFF"
  fi
fi
```
