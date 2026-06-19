#!/usr/bin/env bash
set -euo pipefail

KEY_DEFAULT="aivpn://eyJpIjoiMTAuMC4wLjUiLCJrIjoib3Y4RmJKUlR6QmNDYkpPdklKTnFlMVA1N0ZiZXRJRzJIR0xJdGY2TjMxYz0iLCJwIjoiMngzUWRPZWZPWG5xQ2l1STcrczA3N2dSYngydWV5UjNCa05FMzBkSUliWT0iLCJzIjoiMTg1LjIwNC41NC4zMDo0NDMifQ"
KEY="${1:-$KEY_DEFAULT}"
PKG="com.aivpn.client"
ACTIVITY=".MainActivity"

ADB="${ADB:-$HOME/Library/Android/sdk/platform-tools/adb}"
if [[ ! -x "$ADB" ]]; then
  if command -v adb >/dev/null 2>&1; then
    ADB="$(command -v adb)"
  else
    echo "ERROR: adb not found. Set ADB=/path/to/adb"
    exit 1
  fi
fi

DEVICE="$($ADB devices | awk '$2=="device" { print $1; exit }')"
if [[ -z "$DEVICE" ]]; then
  echo "ERROR: no online emulator/device"
  exit 1
fi

echo "Using adb: $ADB"
echo "Using device: $DEVICE"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

ui_dump() {
  "$ADB" -s "$DEVICE" shell uiautomator dump /sdcard/window_dump.xml >/dev/null
  "$ADB" -s "$DEVICE" pull /sdcard/window_dump.xml "$TMP_DIR/window_dump.xml" >/dev/null
}

center_by_resource_id() {
  local rid="$1"
  python3 - "$TMP_DIR/window_dump.xml" "$rid" <<'PY'
import re
import sys
import xml.etree.ElementTree as ET

xml_path, rid = sys.argv[1], sys.argv[2]
root = ET.parse(xml_path).getroot()
for n in root.iter('node'):
    if n.attrib.get('resource-id') == rid:
        b = n.attrib.get('bounds', '')
        m = re.match(r'\[(\d+),(\d+)\]\[(\d+),(\d+)\]', b)
        if not m:
            continue
        x1, y1, x2, y2 = map(int, m.groups())
        print((x1 + x2) // 2, (y1 + y2) // 2)
        sys.exit(0)
sys.exit(1)
PY
}

center_by_text() {
  local pattern="$1"
  python3 - "$TMP_DIR/window_dump.xml" "$pattern" <<'PY'
import re
import sys
import xml.etree.ElementTree as ET

xml_path, pattern = sys.argv[1], sys.argv[2]
root = ET.parse(xml_path).getroot()
rx = re.compile(pattern, re.IGNORECASE)
for n in root.iter('node'):
    t = n.attrib.get('text', '')
    if rx.search(t):
        b = n.attrib.get('bounds', '')
        m = re.match(r'\[(\d+),(\d+)\]\[(\d+),(\d+)\]', b)
        if not m:
            continue
        x1, y1, x2, y2 = map(int, m.groups())
        print((x1 + x2) // 2, (y1 + y2) // 2)
        sys.exit(0)
sys.exit(1)
PY
}

text_by_resource_id() {
  local rid="$1"
  python3 - "$TMP_DIR/window_dump.xml" "$rid" <<'PY'
import sys
import xml.etree.ElementTree as ET

xml_path, rid = sys.argv[1], sys.argv[2]
root = ET.parse(xml_path).getroot()
for n in root.iter('node'):
    if n.attrib.get('resource-id') == rid:
        print(n.attrib.get('text', ''))
        sys.exit(0)
sys.exit(1)
PY
}

"$ADB" -s "$DEVICE" logcat -c
"$ADB" -s "$DEVICE" shell am force-stop "$PKG"
"$ADB" -s "$DEVICE" shell am start -n "$PKG/$ACTIVITY" >/dev/null
sleep 1

ui_dump
read -r input_x input_y < <(center_by_resource_id "$PKG:id/editConnectionKey")
read -r connect_x connect_y < <(center_by_resource_id "$PKG:id/btnConnect")

echo "Input field center: $input_x,$input_y"
echo "Connect button center: $connect_x,$connect_y"

"$ADB" -s "$DEVICE" shell input tap "$input_x" "$input_y"
"$ADB" -s "$DEVICE" shell input keyevent KEYCODE_MOVE_END
for _ in $(seq 1 260); do
  "$ADB" -s "$DEVICE" shell input keyevent KEYCODE_DEL >/dev/null
done

python3 - "$ADB" "$DEVICE" "$KEY" <<'PY'
import subprocess
import sys

adb, device, key = sys.argv[1], sys.argv[2], sys.argv[3]
for i in range(0, len(key), 25):
    chunk = key[i:i+25]
    subprocess.run([adb, '-s', device, 'shell', 'input', 'text', chunk], check=True)
PY

sleep 1
ui_dump
entered="$(text_by_resource_id "$PKG:id/editConnectionKey" || true)"
if [[ "$entered" != "$KEY" ]]; then
  echo "WARNING: connection key in UI does not exactly match expected key"
  echo "Entered length: ${#entered}, expected length: ${#KEY}"
else
  echo "Connection key entered successfully (len=${#entered})"
fi

"$ADB" -s "$DEVICE" shell input tap "$connect_x" "$connect_y"
sleep 1

for _ in $(seq 1 5); do
  ui_dump
  if read -r allow_x allow_y < <(center_by_text '^(OK|Allow|Разрешить)$'); then
    echo "Tapping VPN permission at $allow_x,$allow_y"
    "$ADB" -s "$DEVICE" shell input tap "$allow_x" "$allow_y"
    break
  fi
  sleep 1
done

start_ts="$(date +%s)"
end_ts=$((start_ts + 35))
success=0
while [[ "$(date +%s)" -lt "$end_ts" ]]; do
  logs="$($ADB -s "$DEVICE" logcat -d -v time AivpnService:V ActivityManager:I AndroidRuntime:E '*:S' || true)"
  if echo "$logs" | grep -q "Handshake successful"; then
    success=1
    break
  fi
  sleep 1
done

final_logs="$($ADB -s "$DEVICE" logcat -d -v time AivpnService:V ActivityManager:I AndroidRuntime:E '*:S' || true)"
echo "\n=== Key AivpnService lines ==="
echo "$final_logs" | grep -E "startVpn:|Creating UDP socket|Socket bound|Sending handshake|Waiting for ServerHello|ServerHello received|Handshake successful|Tunnel error|Failed to bind socket|Stale network|Network changed during setup|Current network lost" || true

if [[ "$success" -eq 1 ]]; then
  echo "\nRESULT: SUCCESS (Handshake successful seen in logs)"
  exit 0
fi

echo "\nRESULT: FAILURE (Handshake successful not seen within timeout)"
exit 2
