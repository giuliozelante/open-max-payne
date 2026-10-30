#!/usr/bin/env bash
# Open Max Payne — Wine-safe stack for localized retail CDs / language packs.
# Same crash class as Italian: FixPack hi-res data/ + ASI dinput under Wine.
# Language RAS naming: x_<lang>.ras (french, german, italian, spanish, …).
# Usage: ./localized-safe-stack.sh /path/to/MaxPayne1
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
# shellcheck disable=SC1091
source "$ROOT/detect-localized-lang.sh"

GAME="${1:-}"
[[ -n "$GAME" && -d "$GAME" ]] || {
  echo "Usage: $0 /path/to/MaxPayne1" >&2
  exit 1
}
GAME="$(cd "$GAME" && pwd)"
OVERLAY="$ROOT/overlays"

info() { echo "==> $*"; }

LOCALIZED_LANG=""
if LOCALIZED_LANG="$(detect_localized_lang "$GAME")"; then
  info "Localized language pack: x_${LOCALIZED_LANG}.ras — applying conservative Wine stack"
else
  info "No non-English x_<lang>.ras — ensuring DXVK/d3d8to9 only (hi-res/ASI left alone)"
fi

# Migrate old Italian-only park folder name
if [[ -d "$GAME/data.disabled-italian" && ! -e "$GAME/data.disabled-hires" ]]; then
  mv "$GAME/data.disabled-italian" "$GAME/data.disabled-hires"
fi

# 1) FixPack hi-res textures — park on any localized pack
if [[ -n "$LOCALIZED_LANG" && -d "$GAME/data" && ! -L "$GAME/data" ]]; then
  if [[ ! -f "$GAME/data/globalconfig.txt" ]]; then
    info "Parking FixPack/hi-res data/ → data.disabled-hires"
    rm -rf "$GAME/data.disabled-hires"
    mv "$GAME/data" "$GAME/data.disabled-hires"
  else
    info "Keeping data/ (has globalconfig.txt — likely a mod database)."
  fi
fi

# 2) ASI loader (huge dinput.dll) — park on localized packs
if [[ -n "$LOCALIZED_LANG" && -f "$GAME/dinput.dll" ]]; then
  sz=$(stat -c%s "$GAME/dinput.dll" 2>/dev/null || echo 0)
  if [[ "$sz" -gt 500000 ]]; then
    info "Parking ASI loader dinput.dll → dinput.dll.asi-loader.bak"
    mv -f "$GAME/dinput.dll" "$GAME/dinput.dll.asi-loader.bak"
  fi
fi

# 3) Prefer Rockstar 1.05 drop-in if EXE missing or still SafeDisc-wrapped
need_105=0
if [[ ! -f "$GAME/MaxPayne.exe" ]]; then
  need_105=1
elif /usr/bin/python3 - "$GAME/MaxPayne.exe" <<'PY'
import struct, sys
from pathlib import Path
data = Path(sys.argv[1]).read_bytes()
if data[:2] != b"MZ":
    sys.exit(1)
e = struct.unpack_from("<I", data, 0x3C)[0]
nsec = struct.unpack_from("<H", data, e + 6)[0]
opt = struct.unpack_from("<H", data, e + 20)[0]
off = e + 24 + opt
for i in range(nsec):
    name = data[off + i * 40 : off + i * 40 + 8].split(b"\0", 1)[0].decode("ascii", "ignore")
    if name.lower().startswith("stxt"):
        sys.exit(0)
sys.exit(1)
PY
then
  need_105=1
fi
if [[ "$need_105" -eq 1 && -f "$GAME/maxpayne1-05patch.exe" ]]; then
  info "Applying Rockstar 1.05 drop-in as MaxPayne.exe"
  [[ -f "$GAME/MaxPayne.exe" && ! -f "$GAME/MaxPayne.exe.safedisc.bak" ]] &&
    cp -f "$GAME/MaxPayne.exe" "$GAME/MaxPayne.exe.safedisc.bak"
  cp -f "$GAME/maxpayne1-05patch.exe" "$GAME/MaxPayne.exe"
  chmod u+x "$GAME/MaxPayne.exe" || true
fi

# 4) d3d8to9 + DXVK
mkdir -p "$OVERLAY/d3d8to9" "$OVERLAY/dxvk"
if [[ -f "$OVERLAY/d3d8to9/d3d8.dll" ]]; then
  cp -f "$OVERLAY/d3d8to9/d3d8.dll" "$GAME/d3d8.dll"
  info "Installed d3d8to9 → d3d8.dll"
fi
if [[ -f "$OVERLAY/dxvk/x32/d3d9.dll" ]]; then
  if [[ -f "$GAME/d3d9.dll" ]]; then
    sz=$(stat -c%s "$GAME/d3d9.dll" 2>/dev/null || echo 0)
    if [[ "$sz" -lt 200000 && ! -f "$GAME/d3d9.dll.thirteenag.bak" ]]; then
      cp -f "$GAME/d3d9.dll" "$GAME/d3d9.dll.thirteenag.bak"
    fi
  fi
  cp -f "$OVERLAY/dxvk/x32/d3d9.dll" "$GAME/d3d9.dll"
  info "Installed DXVK x32 → d3d9.dll"
fi

# 5) Refresh launcher
bash "$ROOT/bootstrap-linux.sh" "$GAME"

info "Localized-safe stack ready: $GAME${LOCALIZED_LANG:+ (lang=$LOCALIZED_LANG)}"
info "Launch: $GAME/launch-openmaxfx.sh"
