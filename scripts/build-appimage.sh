#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
./scripts/build.sh
mkdir -p dist/AppDir/usr/bin dist/AppDir/usr/share/applications
cp dist/clearwave dist/AppDir/usr/bin/
cp packaging/clearwave.desktop dist/AppDir/clearwave.desktop
sed -i 's/Icon=audio-input-microphone/Icon=clearwave/' dist/AppDir/clearwave.desktop
cp dist/AppDir/clearwave.desktop dist/AppDir/usr/share/applications/
cat > dist/AppDir/clearwave.svg <<'ICON'
<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256" viewBox="0 0 256 256"><rect width="256" height="256" rx="48" fill="#123b50"/><path d="M48 128h24l16-48 24 96 24-112 24 96 16-32h32" fill="none" stroke="#65e5d2" stroke-width="12" stroke-linejoin="round"/></svg>
ICON
cat > dist/AppDir/AppRun <<'RUN'
#!/bin/sh
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec "$HERE/usr/bin/clearwave" "$@"
RUN
chmod +x dist/AppDir/AppRun
# Supply APPIMAGETOOL to use an already downloaded tool.
: "${APPIMAGETOOL:=$PWD/dist/appimagetool-x86_64.AppImage}"
if [[ ! -f "$APPIMAGETOOL" ]]; then
  curl --fail --location --retry 3 https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage -o "$APPIMAGETOOL"
fi
chmod +x "$APPIMAGETOOL"
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$APPIMAGETOOL" dist/AppDir dist/Clearwave-x86_64.AppImage
