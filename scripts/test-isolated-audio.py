#!/usr/bin/env python3
"""Manual integration check: private PipeWire server, all hardware disabled, generated samples only."""
import tempfile,pathlib,os,subprocess,time,json,socket
root=pathlib.Path(tempfile.mkdtemp(prefix='clearwave-pipewire-'));root.chmod(0o700)
config=root/'pipewire.conf';config.write_text('''context.properties = { core.daemon = true core.name = pipewire-0 support.dbus = false default.clock.rate = 48000 }
context.spa-libs = { audio.convert.* = audioconvert/libspa-audioconvert support.* = support/libspa-support }
context.modules = [
 { name = libpipewire-module-protocol-native }
 { name = libpipewire-module-metadata }
 { name = libpipewire-module-spa-node-factory }
 { name = libpipewire-module-client-node }
 { name = libpipewire-module-adapter }
 { name = libpipewire-module-link-factory }
 { name = libpipewire-module-session-manager }
 { name = libpipewire-module-access args = { access.force = unrestricted } }
]
context.objects = [ { factory = spa-node-factory args = { factory.name = support.node.driver node.name = Dummy-Driver node.group = pipewire.dummy priority.driver = 20000 } } ]
''')
env=os.environ.copy();env.update(XDG_RUNTIME_DIR=str(root),PIPEWIRE_RUNTIME_DIR=str(root),PULSE_RUNTIME_PATH=str(root/'pulse'),PULSE_SERVER='unix:'+str(root/'pulse/native'))
children=[]
def launch(args):
 p=subprocess.Popen(args,env=env,stdout=open(root/(pathlib.Path(args[0]).name+'.out'),'w'),stderr=open(root/(pathlib.Path(args[0]).name+'.err'),'w'));children.append(p);return p
def run(args):return subprocess.run(args,env=env,text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=12)
try:
 env['XDG_CONFIG_HOME']=str(root/'config')
 wp=root/'config/wireplumber/wireplumber.conf.d';wp.mkdir(parents=True)
 (wp/'no-hardware.conf').write_text('wireplumber.profiles = { main = { hardware.audio = disabled hardware.bluetooth = disabled hardware.video-capture = disabled monitor.alsa-midi = disabled monitor.alsa = disabled monitor.bluez = disabled monitor.bluez-midi = disabled monitor.v4l2 = disabled monitor.libcamera = disabled support.portal-permissionstore = disabled support.reserve-device = disabled } }')
 launch(['pipewire','-c',str(config)]);time.sleep(.3);launch(['wireplumber']);time.sleep(.5);launch(['pipewire-pulse']);time.sleep(.7)
 r=run(['pactl','info']);print('Private server',r.returncode,r.stdout,r.stderr)
 r=run(['pactl','load-module','module-null-sink','sink_name=discard','channels=2']);print('Null sink',r.returncode,r.stdout,r.stderr)
 assert all(x['name'] in ('discard','auto_null') for x in json.loads(run(['pactl','--format=json','list','sinks']).stdout))
 env['XDG_CONFIG_HOME']=str(root/'config')
 launch([str(pathlib.Path('target/debug/clearwave').resolve()),'daemon'])
 time.sleep(.2)
 import socket
 sockpath=root/'clearwave/engine.sock'
 def ipc(request):
  c=socket.socket(socket.AF_UNIX);c.settimeout(15);c.connect(str(sockpath));c.sendall(json.dumps(request).encode()+b'\n');data=b''
  while b'\n' not in data:data+=c.recv(65536)
  c.close();return json.loads(data)
 c=ipc({'command':'status'})['status']['config'];c['output']['device']='discard';print('configure',ipc({'command':'configure','config':c})['ok'])
 result=ipc({'command':'start','input':False});assert result['ok'],result
 route=result['status']['output_route']
 import threading,struct,math
 samples=[];sample_lock=threading.Lock();stop=threading.Event()
 capture=subprocess.Popen(['parec','--raw','--latency-msec=30','--process-time-msec=10','--format=float32le','--rate=48000','--channels=2','--device=discard.monitor','--client-name=ValidationCapture'],env=env,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL);children.append(capture)
 def read():
  while not stop.is_set():
   b=capture.stdout.read(3840)
   if len(b)!=3840:break
   with sample_lock:samples.extend(struct.unpack('<960f',b));del samples[:-96000]
 threading.Thread(target=read,daemon=True).start()
 playback=subprocess.Popen(['pacat','--raw','--latency-msec=30','--process-time-msec=10','--format=float32le','--rate=48000','--channels=2','--device='+route,'--client-name=ValidationTone'],env=env,stdin=subprocess.PIPE,stderr=subprocess.DEVNULL);children.append(playback)
 def tone():
  i=0
  try:
   while not stop.is_set():
    data=[]
    for _ in range(480):data.extend([.05*math.sin(2*math.pi*1000*i/48000),0.0]);i+=1
    playback.stdin.write(struct.pack('<960f',*data));playback.stdin.flush()
  except BrokenPipeError:pass
 tone_thread=threading.Thread(target=tone,daemon=True);tone_thread.start()
 def rms():
  with sample_lock:data=samples[-48000:]
  assert len(data)>1000
  return [math.sqrt(sum(v*v for v in data[c::2])/(len(data)//2)) for c in range(2)]
 time.sleep(2);neutral=rms();assert .032<neutral[0]<.038 and neutral[1]<1e-6,neutral;print('Neutral stereo RMS',neutral)
 c['output']['processing']['gain_db']=-12;assert ipc({'command':'configure','config':c})['ok'];time.sleep(1.5);attenuated=rms();assert .22<attenuated[0]/neutral[0]<.28,attenuated;print('-12 dB live RMS',attenuated)
 c['output']['processing']['gain_db']=0;c['output']['processing']['eq_db'][2]=6;assert ipc({'command':'configure','config':c})['ok'];time.sleep(1.5);equalized=rms();assert 1.85<equalized[0]/neutral[0]<2.15,equalized;print('+6 dB EQ live RMS',equalized)
 assert ipc({'command':'status'})['status']['output']['running']
 stop.set();capture.terminate();playback.terminate();tone_thread.join(timeout=2)
 try:playback.stdin.close()
 except BrokenPipeError:pass
 assert ipc({'command':'stop','input':False})['ok']
 assert ipc({'command':'shutdown'})['ok']
 modules=json.loads(run(['pactl','--format=json','list','modules']).stdout);assert not any('clearwave_' in json.dumps(m) for m in modules),modules
 print('PASS: generated stereo route, isolation, live gain/EQ, cleanup')
 print('ROOT',root)
finally:
 for p in reversed(children):p.terminate()
 for p in children:
  try:p.wait(timeout=5)
  except subprocess.TimeoutExpired:p.kill();p.wait()
