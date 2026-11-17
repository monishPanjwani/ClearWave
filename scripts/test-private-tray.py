#!/usr/bin/python3
"""Run under dbus-run-session; uses generated audio and temp XDG paths only."""
import json, os, pathlib, shutil, socket, subprocess, sys, tempfile, time
import dbus, dbus.service, dbus.mainloop.glib
from gi.repository import GLib

dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
bus=dbus.SessionBus()
context=GLib.MainContext.default()
registrations=[]
host_registered=False
class Watcher(dbus.service.Object):
    @dbus.service.method('org.kde.StatusNotifierWatcher', in_signature='s', out_signature='')
    def RegisterStatusNotifierItem(self, service): registrations.append(str(service))
    @dbus.service.method('org.freedesktop.DBus.Properties', in_signature='ss', out_signature='v')
    def Get(self, interface, prop):
        if prop=='IsStatusNotifierHostRegistered': return dbus.Boolean(host_registered)
        if prop=='RegisteredStatusNotifierItems': return dbus.Array(registrations,signature='s')
        if prop=='ProtocolVersion': return dbus.Int32(0)
        raise dbus.exceptions.DBusException('Unknown property')
    @dbus.service.method('org.freedesktop.DBus.Properties', in_signature='s', out_signature='a{sv}')
    def GetAll(self, interface): return {'IsStatusNotifierHostRegistered':dbus.Boolean(host_registered),'RegisteredStatusNotifierItems':dbus.Array(registrations,signature='s'),'ProtocolVersion':dbus.Int32(0)}

def wait(predicate, timeout=8):
    end=time.monotonic()+timeout
    while time.monotonic()<end:
        while context.pending(): context.iteration(False)
        if predicate(): return
        time.sleep(.03)
    raise AssertionError('condition timed out')

root=pathlib.Path(__file__).resolve().parent.parent
exe=pathlib.Path(sys.argv[1] if len(sys.argv)>1 else root/'target/debug/clearwave').resolve()
with tempfile.TemporaryDirectory(prefix='clearwave-private-tray-') as tmp:
    folder=pathlib.Path(tmp);(folder/'bin').mkdir();(folder/'config').mkdir()
    shutil.copy(root/'tests/fixtures/audio-server.py',folder/'bin/audio');(folder/'bin/audio').chmod(0o755)
    for cmd in ['pactl','parec','pacat']: (folder/'bin'/cmd).symlink_to('audio')
    env={**os.environ,'XDG_RUNTIME_DIR':tmp,'XDG_CONFIG_HOME':str(folder/'config'),'CLEARWAVE_MOCK_DIR':tmp,'PATH':str(folder/'bin')}
    watcher=Watcher(bus,'/StatusNotifierWatcher'); name=dbus.service.BusName('org.kde.StatusNotifierWatcher',bus)
    child=subprocess.Popen([str(exe),'daemon'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
    def request(command,**kw):
        s=socket.socket(socket.AF_UNIX);s.settimeout(15);s.connect(str(folder/'clearwave/engine.sock'));s.sendall((json.dumps({'command':command,**kw})+'\n').encode());data=b''
        while b'\n' not in data: data+=s.recv(65536)
        s.close();reply=json.loads(data);assert reply['ok'],reply;return reply
    try:
        wait(lambda:(folder/'clearwave/engine.sock').exists())
        # A watcher without a registered host must not claim the tray is visible.
        time.sleep(.6)
        assert not request('status')['status']['tray_available']
        host_registered=True
        wait(lambda:request('status')['status']['tray_available'],timeout=12)
        service=registrations[-1]
        item=bus.get_object(service,'/StatusNotifierItem',introspect=False)
        props=dbus.Interface(item,'org.freedesktop.DBus.Properties')
        assert str(props.Get('org.kde.StatusNotifierItem','Status'))=='Active'
        assert str(props.Get('org.kde.StatusNotifierItem','IconName')) == ''
        pixmaps=props.Get('org.kde.StatusNotifierItem','IconPixmap')
        assert {int(p[0]) for p in pixmaps} == {32,64}
        for width,height,data in pixmaps:
            assert len(data)==int(width)*int(height)*4
            assert any(int(v) for v in data), 'tray logo is empty'
        menu=bus.get_object(service,'/MenuBar',introspect=False)
        menu=dbus.Interface(menu,'com.canonical.dbusmenu')
        def entries():
            _,layout=menu.GetLayout(dbus.Int32(0),dbus.Int32(-1),dbus.Array([],signature="s"))
            return {str(row[1].get('label','')):int(row[0]) for row in layout[2]}
        labels=entries();assert 'Open ClearWave' in labels and 'Quit ClearWave' in labels,labels
        config=folder/'settings.json';config.write_text(json.dumps({'input':{'device':'mic','processing':{'denoise':False}},'output':{'device':'speakers'}}))
        c=json.loads(subprocess.check_output([str(exe),'engine','status','--json'],env=env))['status']['config'];c['input']['device']='mic';c['input']['processing']['denoise']=False;c['output']['device']='speakers'
        request('configure',config=c)

        def click(label):
            wait(lambda:label in entries());menu.Event(entries()[label],'clicked',dbus.String('',variant_level=1),dbus.UInt32(0))
        click('Start at login');wait(lambda:request('status')['status']['autostart']['enabled'])
        def login_checked():
            _,layout=menu.GetLayout(dbus.Int32(0),dbus.Int32(-1),dbus.Array([],signature='s'))
            return next(int(row[1].get('toggle-state',0)) for row in layout[2] if str(row[1].get('label',''))=='Start at login')
        wait(lambda:login_checked()==1)
        assert (folder/'config/autostart/clearwave.desktop').exists()
        idle=request('status')['status'];assert not idle['input']['running'] and not idle['output']['running']
        click('Start at login');wait(lambda:not request('status')['status']['autostart']['enabled'])
        click('Start output');wait(lambda:request('status')['status']['output']['running'])
        click('Start microphone');wait(lambda:request('status')['status']['input']['running'])
        click('Stop microphone');wait(lambda:not request('status')['status']['input']['running']);assert request('status')['status']['output']['running']
        # The watcher can disappear/reappear; processing remains alive and one icon re-registers.
        before_restart=len(registrations)
        bus.release_name('org.kde.StatusNotifierWatcher');wait(lambda:not request('status')['status']['tray_available'])
        assert request('status')['status']['output']['running']
        bus.request_name('org.kde.StatusNotifierWatcher');wait(lambda:len(registrations)>before_restart,timeout=12)
        menu=dbus.Interface(bus.get_object(registrations[-1],'/MenuBar',introspect=False),'com.canonical.dbusmenu')
        assert len(registrations)==before_restart+1
        click('Stop all processing');wait(lambda:not request('status')['status']['output']['running'])
        assert not json.loads((folder/'state.json').read_text())['modules']
        # Simulate the GUI activation endpoint; explicit Quit must notify it, not spawn a window.
        gui=socket.socket(socket.AF_UNIX,socket.SOCK_DGRAM);gui.bind(str(folder/'clearwave/gui.sock'));gui.settimeout(5)
        click('Quit ClearWave');wait(lambda:child.poll() is not None);assert gui.recv(16)==b'quit';gui.close()
        assert child.returncode==0 and not (folder/'clearwave/engine.sock').exists()
        time.sleep(.5);assert child.poll()==0
        print('PASS: private SNI/DBusMenu, independent controls, watcher recovery, StopAll cleanup, Quit + GUI notification')
    finally:
        if child.poll() is None:child.kill()
        child.wait()
