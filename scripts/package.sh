#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

BIN_NAME="macrond"
OS_NAME="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH_NAME="$(uname -m)"
RELEASE_DIR="$ROOT_DIR/release"
TARGET_BIN="$ROOT_DIR/target/release/$BIN_NAME"
DIST_BIN="$RELEASE_DIR/$BIN_NAME"
ARCHIVE_NAME="${BIN_NAME}-${OS_NAME}-${ARCH_NAME}.tar.gz"
ARCHIVE_PATH="$RELEASE_DIR/$ARCHIVE_NAME"
CHECKSUM_FILE="$RELEASE_DIR/SHA256SUMS.txt"
NOTES_FILE="$RELEASE_DIR/README_COPY.txt"
INSTALL_SCRIPT="$RELEASE_DIR/install_macrond.sh"

mkdir -p "$RELEASE_DIR"

cargo build --release

# Avoid copying macOS extended attributes into distribution artifacts.
COPYFILE_DISABLE=1 cp -f "$TARGET_BIN" "$DIST_BIN"
chmod +x "$DIST_BIN"
codesign --force --sign - "$DIST_BIN" >/dev/null 2>&1 || true

rm -f "$ARCHIVE_PATH"
tar -C "$RELEASE_DIR" -czf "$ARCHIVE_PATH" "$BIN_NAME"

(
  cd "$RELEASE_DIR"
  shasum -a 256 "$BIN_NAME" "$ARCHIVE_NAME" > "$(basename "$CHECKSUM_FILE")"
)

cat > "$NOTES_FILE" <<'TXT'
Macrond distribution files are generated in this directory.

Copy to target machine from here:
- macrond
or
- macrond-<os>-<arch>.tar.gz

Recommended copy flow:
1) Copy release/macrond and release/install_macrond.sh to target directory.
2) Run:
   ./install_macrond.sh
3) Verify:
   ./macrond --version

Fallback commands (if still blocked):
  chmod +x ./macrond
  xattr -dr com.apple.quarantine ./macrond
  codesign --force --sign - ./macrond
TXT

cat > "$INSTALL_SCRIPT" <<'TXT'
#!/usr/bin/env bash
set -euo pipefail

BIN="./macrond"
if [[ ! -f "$BIN" ]]; then
  echo "ERROR: ./macrond not found in current directory"
  exit 1
fi

chmod +x "$BIN"
xattr -dr com.apple.quarantine "$BIN" 2>/dev/null || true
codesign --force --sign - "$BIN" >/dev/null 2>&1 || true

echo "Done. Try:"
echo "  ./macrond --version"
echo
echo "Fallback commands (if still blocked):"
echo "  chmod +x ./macrond"
echo "  xattr -dr com.apple.quarantine ./macrond"
echo "  codesign --force --sign - ./macrond"
TXT
chmod +x "$INSTALL_SCRIPT"

printf 'Package complete.\n- Binary: %s\n- Archive: %s\n- Checksums: %s\n- Notes: %s\n' \
  "$DIST_BIN" "$ARCHIVE_PATH" "$CHECKSUM_FILE" "$NOTES_FILE"
