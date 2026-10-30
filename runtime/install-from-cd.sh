#!/usr/bin/env bash
# OpenMaxFX — install Max Payne 1 on Linux from the retail CD (or a copy of it)
# Usage:
#   ./install-from-cd.sh                      # auto-find CD, install to ~/Games/MaxPayne1
#   ./install-from-cd.sh /path/to/CD          # CD already mounted / copied
#   ./install-from-cd.sh /path/to/CD /path/to/install
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
UNSHIELD="${UNSHIELD:-$HOME/.local/bin/unshield}"
export LD_LIBRARY_PATH="${HOME}/.local/lib:${LD_LIBRARY_PATH:-}"

DEFAULT_DEST="${HOME}/Games/MaxPayne1"

die() { echo "ERROR: $*" >&2; exit 1; }
info() { echo "==> $*"; }

find_unshield() {
  if [[ -x "$UNSHIELD" ]]; then return 0; fi
  if command -v unshield >/dev/null 2>&1; then
    UNSHIELD="$(command -v unshield)"
    return 0
  fi
  die "unshield not found. Expected at $HOME/.local/bin/unshield (OpenMaxFX builds it once)."
}

find_cd_root() {
  # 1) explicit
  if [[ -n "${1:-}" && -d "$1" ]]; then
    echo "$1"
    return 0
  fi
  # 2) mounted optical with MAXPAYNE label / Disk1
  local m
  while IFS= read -r m; do
    [[ -z "$m" ]] && continue
    if [[ -d "$m/Disk1" || -f "$m/Disk1/data1.cab" || -d "$m/MAX-FX" ]]; then
      echo "$m"
      return 0
    fi
  done < <(findmnt -n -t iso9660,udf -o TARGET 2>/dev/null || true)
  # 3) try mount /dev/sr0 (timeout — udisksctl can hang waiting for a disc)
  if [[ -b /dev/sr0 ]]; then
    if timeout 8 udisksctl mount -b /dev/sr0 >/dev/null 2>&1; then
      m="$(findmnt -n -o TARGET -S /dev/sr0 2>/dev/null || true)"
      if [[ -n "$m" && -d "$m/Disk1" ]]; then
        echo "$m"
        return 0
      fi
    fi
  fi
  return 1
}

looks_like_maxpayne_cd() {
  local root="$1"
  [[ -f "$root/Disk1/data1.cab" || -f "$root/Disk1/data2.cab" || -f "$root/Disk1/MaxPayne.exe" ]]
}

install_from_root() {
  local CD="$1"
  local DEST="$2"
  find_unshield
  looks_like_maxpayne_cd "$CD" || die "Not a Max Payne CD layout (need Disk1/data*.cab). Got: $CD"

  info "CD source: $CD"
  info "Install to: $DEST"
  mkdir -p "$DEST"

  # --- InstallShield game cabinets ---
  if [[ -f "$CD/Disk1/data1.cab" ]]; then
    info "Extracting InstallShield cabinets (this can take a few minutes)…"
    # Header often lives next to data1.cab; unshield wants the .cab path
    "$UNSHIELD" -d "$DEST/_unshield_tmp" -D 3 x "$CD/Disk1/data1.cab" || \
      "$UNSHIELD" -d "$DEST/_unshield_tmp" x "$CD/Disk1/data1.cab"
    # Flatten typical InstallShield component folders into DEST
    info "Arranging files…"
    # Copy everything extracted up one or more levels into DEST
    find "$DEST/_unshield_tmp" -type f | while read -r f; do
      rel="${f#"$DEST/_unshield_tmp/"}"
      # strip first path component (often a component name)
      short="${rel#*/}"
      if [[ "$short" == "$rel" ]]; then
        target="$DEST/$(basename "$f")"
      else
        target="$DEST/$short"
      fi
      mkdir -p "$(dirname "$target")"
      cp -n "$f" "$target" 2>/dev/null || cp -f "$f" "$target"
    done
    rm -rf "$DEST/_unshield_tmp"
  else
    info "No data1.cab — copying Disk1 files directly"
  fi

  # --- Loose files from Disk1 (exe, levels, etc.) ---
  if [[ -d "$CD/Disk1" ]]; then
    info "Copying Disk1 loose files (exe, levels, …)"
    # MaxPayne.exe often sits on the CD already
    [[ -f "$CD/Disk1/MaxPayne.exe" ]] && cp -f "$CD/Disk1/MaxPayne.exe" "$DEST/"
    if [[ -d "$CD/Disk1/Levels" ]]; then
      mkdir -p "$DEST/Levels" "$DEST"
      cp -f "$CD/Disk1/Levels/"*.ras "$DEST/" 2>/dev/null || true
      cp -f "$CD/Disk1/Levels/"*.ras "$DEST/Levels/" 2>/dev/null || true
    fi
    # Some layouts put ras next to exe after install
    find "$CD/Disk1" -maxdepth 2 -iname '*.ras' -exec cp -n {} "$DEST/" \; 2>/dev/null || true
  fi

  # --- Optional: rasmaker next to game for convenience ---
  local PROJECT_ROOT RAS_SRC
  PROJECT_ROOT="$(cd "$ROOT/.." && pwd)"
  RAS_SRC="$PROJECT_ROOT/vendor/max-fx-tools/Rasmaker"
  if [[ -f "$RAS_SRC/RASMaker.exe" ]]; then
    info "Copying RASMaker into game folder"
    cp -f "$RAS_SRC/RASMaker.exe" "$RAS_SRC/rl.dll" "$DEST/" 2>/dev/null || true
  fi

  # Sanity
  if [[ ! -f "$DEST/MaxPayne.exe" ]]; then
    # search one level down
    local found
    found="$(find "$DEST" -iname 'MaxPayne.exe' | head -1 || true)"
    if [[ -n "$found" ]]; then
      info "MaxPayne.exe found at $found — promoting to install root"
      cp -f "$found" "$DEST/MaxPayne.exe"
    else
      die "Install finished but MaxPayne.exe is missing. Check CD / unshield output in $DEST"
    fi
  fi

  info "Game files present:"
  ls -lh "$DEST/MaxPayne.exe" 2>/dev/null || true
  ls "$DEST"/*.ras 2>/dev/null | head -10 || true

  # --- Modern Linux wrappers ---
  info "Installing DXVK / d3d8to9 launcher…"
  if [[ -x "$ROOT/bootstrap-linux.sh" || -f "$ROOT/bootstrap-linux.sh" ]]; then
    bash "$ROOT/bootstrap-linux.sh" "$DEST"
  else
    echo "WARNING: bootstrap-linux.sh missing at $ROOT — game files are installed; run bootstrap later." >&2
  fi

  echo
  echo "============================================"
  echo " Max Payne 1 installed"
  echo " Folder: $DEST"
  echo " Start:  $DEST/launch-openmaxfx.sh"
  echo "============================================"
}

# --- main ---
# Args: [CD_ROOT] [DEST]
# Or:   DEST only via env OPENMAXFX_INSTALL_DEST when auto-detecting CD
CD_ARG="${1:-}"
DEST_ARG="${2:-${OPENMAXFX_INSTALL_DEST:-$DEFAULT_DEST}}"

# If the only argument looks like an install dest (no Disk1), treat as DEST
if [[ -n "$CD_ARG" && ! -d "$CD_ARG/Disk1" && -z "${2:-}" && "$CD_ARG" == /* ]]; then
  # Could be a mistaken dest-only call; still try as CD first
  :
fi

CD_ROOT=""
if [[ -n "$CD_ARG" && -d "$CD_ARG/Disk1" ]]; then
  CD_ROOT="$CD_ARG"
elif [[ -n "$CD_ARG" && -d "$CD_ARG" ]] && looks_like_maxpayne_cd "$CD_ARG"; then
  CD_ROOT="$CD_ARG"
else
  info "Looking for Max Payne CD…"
  if CD_ROOT="$(find_cd_root)"; then
    info "Found CD at $CD_ROOT"
  else
    die "No Max Payne CD found.
Insert the retail CD, wait a few seconds, then run again.

Or mount it yourself and pass the path:
  $0 /run/media/$USER/MAXPAYNE
  $0 /run/media/$USER/MAXPAYNE $HOME/Games/MaxPayne1"
  fi
fi

install_from_root "$CD_ROOT" "$DEST_ARG"
