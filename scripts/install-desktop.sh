#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

# Finish the build before installing anything in the user's desktop.
./scripts/build.sh gui
python3 - <<'PY'
from pathlib import Path
import os
import tempfile

root = Path.cwd()
data = Path(os.environ.get('XDG_DATA_HOME') or Path.home() / '.local/share')
if not data.is_absolute():
    raise SystemExit('XDG_DATA_HOME must be an absolute path.')
app_dir = data / 'clearwave'
applications = data / 'applications'
app_dir.mkdir(parents=True, exist_ok=True)
applications.mkdir(parents=True, exist_ok=True)
binary = app_dir / 'clearwave'

def atomic_write(destination, content, mode):
    fd, name = tempfile.mkstemp(prefix='.clearwave-', dir=destination.parent)
    try:
        with os.fdopen(fd, 'wb') as file:
            file.write(content)
        os.chmod(name, mode)
        os.replace(name, destination)
    finally:
        if os.path.exists(name):
            os.unlink(name)

atomic_write(binary, (root / 'dist/clearwave').read_bytes(), 0o755)
# Escape the desktop-entry string layer and the Exec argument layer.
command = str(binary).replace('%', '%%')
command = ''.join('\\' + c if c in '\\"`$' else c for c in command)
command = command.replace('\\', '\\\\')
entry = (root / 'packaging/clearwave.desktop').read_text()
entry = entry.replace('Exec=clearwave open', 'Exec="' + command + '" open')
destination = applications / 'clearwave.desktop'
atomic_write(destination, entry.encode(), 0o644)
print(f'Installed {binary}')
print(f'Installed {destination}')
PY
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "${XDG_DATA_HOME:-$HOME/.local/share}/applications"
fi
printf 'Open your app launcher and search for Clearwave.\n'
