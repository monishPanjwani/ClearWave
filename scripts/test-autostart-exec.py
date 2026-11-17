#!/usr/bin/python3
"""Validate Exec against GLib's actual Desktop Entry launcher, in temporary XDG."""
import json, os, pathlib, subprocess, sys, tempfile, time
try:
    import gi
    gi.require_version('GioUnix', '2.0')
    from gi.repository import GioUnix as DesktopNamespace
except (ImportError, ValueError):
    from gi.repository import Gio as DesktopNamespace
root=pathlib.Path(__file__).resolve().parent.parent
binary=pathlib.Path(sys.argv[1] if len(sys.argv)>1 else root/'target/debug/clearwave').resolve()
with tempfile.TemporaryDirectory(prefix='clearwave-exec-') as tmp:
    folder=pathlib.Path(tmp); target=folder/'path with spaces " %$`\\runner'; result=folder/'arguments.json'
    target.write_text('#!/usr/bin/python3\nimport json,sys,pathlib\npathlib.Path('+repr(str(result))+').write_text(json.dumps(sys.argv[1:]))\n')
    target.chmod(0o755)
    env={**os.environ,'XDG_CONFIG_HOME':str(folder/'config'),'XDG_RUNTIME_DIR':tmp,'DBUS_SESSION_BUS_ADDRESS':'unix:path='+tmp+'/no-bus','APPIMAGE':str(target)}
    subprocess.run([str(binary),'autostart','enable'],env=env,check=True,stdout=subprocess.DEVNULL)
    entry=folder/'config/autostart/clearwave.desktop'
    subprocess.run(['desktop-file-validate',str(entry)],check=True)
    app=DesktopNamespace.DesktopAppInfo.new_from_filename(str(entry));assert app is not None
    assert app.launch([],None)
    deadline=time.monotonic()+5
    while not result.exists():
        assert time.monotonic()<deadline,'Desktop Entry Exec did not launch';time.sleep(.02)
    assert json.loads(result.read_text())==['startup']
    # A moved AppImage must still be disableable; no replacement microphone is involved.
    target.unlink()
    subprocess.run([str(binary),'autostart','disable'],env=env,check=True,stdout=subprocess.DEVNULL)
    assert not entry.exists()
    print('PASS: GLib Desktop Entry Exec escaping and moved AppImage disable')
