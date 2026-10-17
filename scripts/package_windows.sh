#!/bin/bash
set -e

# Ensure we are in the project root
cd "$(dirname "$0")/.."
PROJECT_ROOT="$(pwd)"

mkdir -p dist package/q

# Resolve default version from Cargo.toml
DEFAULT_VERSION=$(grep -m1 -E '^version\s*=\s*"' "$PROJECT_ROOT/Cargo.toml" 2>/dev/null | sed -E 's/.*"([0-9.]+)".*/\1/' || true)
DEFAULT_VERSION="${DEFAULT_VERSION:-0.1.0}"

# Resolve release tag and version
TAG_NAME="${TAG_NAME:-${GITHUB_REF_NAME:-v$DEFAULT_VERSION}}"
TAG_NAME="${TAG_NAME#refs/tags/}"
TAG_NAME="${TAG_NAME#refs/heads/}"
TAG_NAME="${TAG_NAME//\//-}"

# Extract version starting with digit if available, else fallback to Cargo.toml version
EXTRACTED_VER=$(echo "$TAG_NAME" | grep -oE '[0-9]+\.[0-9]+(\.[0-9]+)?' | head -n1 || true)
VERSION="${VERSION:-$EXTRACTED_VER}"
VERSION="${VERSION:-$DEFAULT_VERSION}"
if [[ ! "$VERSION" =~ ^[0-9] ]]; then
  VERSION="$DEFAULT_VERSION"
fi
VERSION="${VERSION%%-*}"

echo "Packaging Windows distribution:"
echo "  Tag:     $TAG_NAME"
echo "  Version: $VERSION"

# 1. Copy application binaries
cp target/release/q.exe package/q/q.exe
cp target/release/q.exe package/q/schedule.exe
cp target/release/qdaemon.exe package/q/qdaemon.exe
cp README.md package/q/ 2>/dev/null || true

echo "Verifying package contents:"
ls -la package/q/

if [ ! -f "package/q/q.exe" ] || [ ! -f "package/q/qdaemon.exe" ] || [ ! -f "package/q/schedule.exe" ]; then
  echo "FATAL ERROR: Required binaries (q.exe, schedule.exe, qdaemon.exe) not found in package!" >&2
  exit 1
fi

# 2. Compress portable zip
cmake -E chdir package cmake -E tar cfv "../dist/q-${TAG_NAME}-windows-x64.zip" --format=zip q
echo "Windows portable zip created: dist/q-${TAG_NAME}-windows-x64.zip"

# 3. Compile Inno Setup EXE Installer
echo ""
echo ">>> Checking for Inno Setup compiler..."
ISCC_BIN=""
if command -v iscc >/dev/null 2>&1; then
  ISCC_BIN="$(command -v iscc)"
elif command -v iscc.exe >/dev/null 2>&1; then
  ISCC_BIN="$(command -v iscc.exe)"
elif [ -f "/c/Program Files (x86)/Inno Setup 6/ISCC.exe" ]; then
  ISCC_BIN="/c/Program Files (x86)/Inno Setup 6/ISCC.exe"
elif [ -f "/c/Program Files/Inno Setup 6/ISCC.exe" ]; then
  ISCC_BIN="/c/Program Files/Inno Setup 6/ISCC.exe"
fi

if [ -n "$ISCC_BIN" ]; then
  echo "Compiling Inno Setup installer using $ISCC_BIN..."

  if command -v cygpath >/dev/null 2>&1; then
    ISS_FILE_WIN="$(cygpath -w "$PROJECT_ROOT/packaging/windows/q.iss")"
  else
    ISS_FILE_WIN="$PROJECT_ROOT\\packaging\\windows\\q.iss"
  fi

  MSYS2_ARG_CONV_EXCL="*" MSYS_NO_PATHCONV=1 "$ISCC_BIN" \
    -DMyAppVersion="$VERSION" \
    -DOutputBaseFilename="q-${TAG_NAME}-windows-x64-setup" \
    "$ISS_FILE_WIN"

  echo "Windows setup installer created: dist/q-${TAG_NAME}-windows-x64-setup.exe"
else
  echo "Inno Setup (iscc) not found; skipping Windows EXE installer."
fi

# 4. Compile WiX Toolset MSI Installer
echo ""
echo ">>> Checking for WiX Toolset..."
WIX_HEAT=""
WIX_CANDLE=""
WIX_LIGHT=""
if command -v heat >/dev/null 2>&1 && command -v candle >/dev/null 2>&1 && command -v light >/dev/null 2>&1; then
  WIX_HEAT="heat"
  WIX_CANDLE="candle"
  WIX_LIGHT="light"
elif [ -f "/c/Program Files (x86)/WiX Toolset v3.11/bin/candle.exe" ]; then
  WIX_HEAT="/c/Program Files (x86)/WiX Toolset v3.11/bin/heat.exe"
  WIX_CANDLE="/c/Program Files (x86)/WiX Toolset v3.11/bin/candle.exe"
  WIX_LIGHT="/c/Program Files (x86)/WiX Toolset v3.11/bin/light.exe"
elif [ -f "/c/Program Files (x86)/WiX Toolset v3.14/bin/candle.exe" ]; then
  WIX_HEAT="/c/Program Files (x86)/WiX Toolset v3.14/bin/heat.exe"
  WIX_CANDLE="/c/Program Files (x86)/WiX Toolset v3.14/bin/candle.exe"
  WIX_LIGHT="/c/Program Files (x86)/WiX Toolset v3.14/bin/light.exe"
fi

if [ -n "$WIX_CANDLE" ] && [ -n "$WIX_LIGHT" ] && [ -n "$WIX_HEAT" ]; then
  echo "Compiling WiX MSI installer..."

  if command -v cygpath >/dev/null 2>&1; then
    SOURCE_DIR_WIN="$(cygpath -w "$PROJECT_ROOT/package/q")"
    LICENSE_RTF_WIN="$(cygpath -w "$PROJECT_ROOT/packaging/windows/License.rtf")"
    WXS_Q_WIN="$(cygpath -w "$PROJECT_ROOT/packaging/windows/q.wxs")"
    WXS_FILES_WIN="$(cygpath -w "$PROJECT_ROOT/packaging/windows/files.wxs")"
    OUT_DIR_WIN="$(cygpath -w "$PROJECT_ROOT/packaging/windows/")\\"
    MSI_OUT_WIN="$(cygpath -w "$PROJECT_ROOT/dist/q-${TAG_NAME}-windows-x64.msi")"
    OBJ_Q_WIN="$(cygpath -w "$PROJECT_ROOT/packaging/windows/q.wixobj")"
    OBJ_FILES_WIN="$(cygpath -w "$PROJECT_ROOT/packaging/windows/files.wixobj")"
  else
    SOURCE_DIR_WIN="$PROJECT_ROOT\\package\\q"
    LICENSE_RTF_WIN="$PROJECT_ROOT\\packaging\\windows\\License.rtf"
    WXS_Q_WIN="$PROJECT_ROOT\\packaging\\windows\\q.wxs"
    WXS_FILES_WIN="$PROJECT_ROOT\\packaging\\windows\\files.wxs"
    OUT_DIR_WIN="$PROJECT_ROOT\\packaging\\windows\\"
    MSI_OUT_WIN="$PROJECT_ROOT\\dist\\q-${TAG_NAME}-windows-x64.msi"
    OBJ_Q_WIN="$PROJECT_ROOT\\packaging\\windows\\q.wixobj"
    OBJ_FILES_WIN="$PROJECT_ROOT\\packaging\\windows\\files.wixobj"
  fi

  # Remove empty directories to avoid empty directory components
  find package/q -type d -empty -delete 2>/dev/null || true

  MSYS2_ARG_CONV_EXCL="*" MSYS_NO_PATHCONV=1 "$WIX_HEAT" dir "$SOURCE_DIR_WIN" \
    -cg AppFiles -dr INSTALLFOLDER -scom -sreg -sfrag -srd -ag -sw5150 \
    -var var.SourceDir -out "$WXS_FILES_WIN"

  # Replace any remaining PUT-GUID-HERE with valid GUIDs
  python3 -c "import re, uuid, os; p='${PROJECT_ROOT}/packaging/windows/files.wxs'; f=open(p,'r',encoding='utf-8',errors='ignore'); c=f.read(); f.close(); open(p,'w',encoding='utf-8').write(re.sub(r'PUT-GUID-HERE', lambda m: str(uuid.uuid4()).upper(), c))" 2>/dev/null || \
  python -c "import re, uuid, os; p='${PROJECT_ROOT}/packaging/windows/files.wxs'; f=open(p,'r',encoding='utf-8',errors='ignore'); c=f.read(); f.close(); open(p,'w',encoding='utf-8').write(re.sub(r'PUT-GUID-HERE', lambda m: str(uuid.uuid4()).upper(), c))" 2>/dev/null || \
  sed -i 's/PUT-GUID-HERE/*/g' "$PROJECT_ROOT/packaging/windows/files.wxs" 2>/dev/null || true

  MSYS2_ARG_CONV_EXCL="*" MSYS_NO_PATHCONV=1 "$WIX_CANDLE" \
    -dVersion="$VERSION" -dSourceDir="$SOURCE_DIR_WIN" -dLicenseRtf="$LICENSE_RTF_WIN" \
    -arch x64 "$WXS_Q_WIN" "$WXS_FILES_WIN" -out "$OUT_DIR_WIN"

  MSYS2_ARG_CONV_EXCL="*" MSYS_NO_PATHCONV=1 "$WIX_LIGHT" \
    -sval -ext WixUIExtension "$OBJ_Q_WIN" "$OBJ_FILES_WIN" -out "$MSI_OUT_WIN"

  rm -f packaging/windows/*.wixobj packaging/windows/files.wxs packaging/windows/*.wixpdb
  echo "Windows MSI installer created: dist/q-${TAG_NAME}-windows-x64.msi"
else
  echo "WiX Toolset (candle/light/heat) not found; skipping Windows MSI installer."
fi

echo ""
echo "=================================================="
echo " Windows Packaging Completed"
shopt -s nullglob
PACKAGES=(dist/*.zip dist/*.exe dist/*.msi)
shopt -u nullglob
if [ ${#PACKAGES[@]} -gt 0 ]; then
  ls -lh "${PACKAGES[@]}"
else
  echo "No packages generated."
fi
echo "=================================================="
