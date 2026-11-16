#!/usr/bin/python3
"""Deterministic local audio-server double. Generates silence; no audio APIs."""
import fcntl,json,os,struct,sys,time
from pathlib import Path
root=Path(os.environ['CLEARWAVE_MOCK_DIR'])
name=Path(sys.argv[0]).name
if name in ('parec','pacat'):
    device=sys.argv[sys.argv.index('--device')+1]
    prefix='capture' if name=='parec' else 'playback'
    (root/f'{prefix}-{os.getpid()}').write_text(str(os.getpid()))
    (root/f'{prefix}-device-{os.getpid()}').write_text(device)
    try:
        if name=='parec':
            channels=2 if '--channels=2' in sys.argv else 1
            while True:
                sys.stdout.buffer.write(struct.pack('<f',0.0)*480*channels)
                sys.stdout.buffer.flush();time.sleep(.01)
        else:
            while sys.stdin.buffer.read(1920): pass
    except (BrokenPipeError,KeyboardInterrupt): pass
    sys.exit(0)
with (root/'state.lock').open('a') as lock:
    fcntl.flock(lock,fcntl.LOCK_EX)
    path=root/'state.json'
    state=json.loads(path.read_text()) if path.exists() else {'modules':{},'next':100,'default_source':'mic','default_sink':'speakers'}
    args=sys.argv[1:]
    if args and args[0]=='--format=json':args=args[1:]
    result=''
    if args==['info']:
        result={'default_source_name':state['default_source'],'default_sink_name':state['default_sink'],'server_name':'Isolated generated-signal test server'}
    elif args[:1]==['load-module']:
        module=state['next'];state['next']+=1;state['modules'][str(module)]=args;result=str(module)
    elif args[:1]==['unload-module']:state['modules'].pop(args[1],None)
    elif args[:1]==['get-default-source']:result=state['default_source']
    elif args[:1]==['get-default-sink']:result=state['default_sink']
    elif args[:1]==['set-default-source']:state['default_source']=args[1]
    elif args[:1]==['set-default-sink']:state['default_sink']=args[1]
    elif args==['list','sources']:result=[{'name':'mic','index':14,'description':'Generated microphone','monitor_of_sink':None}]
    elif args==['list','sinks']:result=[{'name':'speakers','index':7,'description':'Discard output'}]
    elif args==['list','source-outputs'] or args==['list','sink-inputs']:
        capture=args[1]=='source-outputs';prefix='capture' if capture else 'playback';result=[]
        for p in root.glob(prefix+'-device-*'):
            pid=p.name.rsplit('-',1)[1]
            if not Path('/proc/'+pid).exists():continue
            result.append({'source':14,'sink':7,'properties':{'application.process.id':pid}})
    else:
        print('Unsupported mock command: '+str(args),file=sys.stderr);sys.exit(1)
    temp=path.with_suffix('.tmp');temp.write_text(json.dumps(state));temp.replace(path)
    print(json.dumps(result) if isinstance(result,(dict,list)) else result)
