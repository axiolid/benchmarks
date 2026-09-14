#!/bin/bash
# Render proof against the running server. Kept as a script so the long
# chromium path is never truncated inline.
set -u
CHROME=~/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome
PORT=${1:-8095}
"$CHROME" --headless=new --no-sandbox --disable-gpu \
  --window-size=1440,1200 --remote-debugging-port=9422 about:blank \
  >/tmp/chrome.log 2>&1 &
CHROME_PID=$!
sleep 4
BASE="http://127.0.0.1:$PORT" node verify.mjs
STATUS=$?
kill $CHROME_PID 2>/dev/null
exit $STATUS
