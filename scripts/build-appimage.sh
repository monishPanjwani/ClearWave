#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
./scripts/build.sh
mkdir -p dist/AppDir/usr/bin dist/AppDir/usr/share/applications
cp dist/clearwave dist/AppDir/usr/bin/
mkdir -p dist/AppDir/usr/share/icons/hicolor/scalable/status
cp assets/tray/*.svg dist/AppDir/usr/share/icons/hicolor/scalable/status/
mkdir -p dist/AppDir/usr/share/doc/clearwave
cp LICENSE THIRD_PARTY_NOTICES.md README.md dist/AppDir/usr/share/doc/clearwave/
cp -R dist/licenses dist/AppDir/usr/share/doc/clearwave/
cp packaging/clearwave.desktop dist/AppDir/clearwave.desktop
sed -i 's/Icon=audio-input-microphone/Icon=clearwave/' dist/AppDir/clearwave.desktop
cp dist/AppDir/clearwave.desktop dist/AppDir/usr/share/applications/
cp assets/clearwave.svg dist/AppDir/clearwave.svg
cp assets/clearwave.png dist/AppDir/clearwave.png
ln -sfn clearwave.png dist/AppDir/.DirIcon
cat > dist/AppDir/AppRun <<'RUN'
#!/bin/sh
HERE=$(CDPATH= cd -- "${0%/*}" && pwd)
exec "$HERE/usr/bin/clearwave" "$@"
RUN
chmod +x dist/AppDir/AppRun
# Supply APPIMAGETOOL to use an already downloaded tool.
: "${APPIMAGETOOL:=$PWD/dist/appimagetool-x86_64.AppImage}"
if [[ ! -f "$APPIMAGETOOL" ]]; then
  curl --fail --location --retry 3 --connect-timeout 20 --max-time 120 https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage -o "$APPIMAGETOOL"
fi
chmod +x "$APPIMAGETOOL"
# Cache the runtime too; appimagetool's implicit download can wait indefinitely.
: "${APPIMAGE_RUNTIME:=$PWD/dist/runtime-x86_64}"
if [[ ! -f "$APPIMAGE_RUNTIME" ]]; then
  curl --fail --location --retry 3 --connect-timeout 20 --max-time 120 https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64 -o "$APPIMAGE_RUNTIME"
fi
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$APPIMAGETOOL" --runtime-file "$APPIMAGE_RUNTIME" dist/AppDir dist/Clearwave-x86_64.AppImage
