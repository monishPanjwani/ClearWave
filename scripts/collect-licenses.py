#!/usr/bin/env python3
"""Collect installed Cargo dependency license files for Linux binary distribution."""
import json, pathlib, subprocess, shutil
root=pathlib.Path(__file__).resolve().parent.parent
metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--offline','--format-version','1','--filter-platform','x86_64-unknown-linux-gnu'],cwd=root))
destination=root/'dist/licenses'
if destination.exists():shutil.rmtree(destination)
destination.mkdir(parents=True)
tree=subprocess.check_output(['cargo','tree','--locked','--offline','--edges','normal,build','--prefix','none','--format','{p}','--target','x86_64-unknown-linux-gnu'],cwd=root,text=True)
selected={tuple(line.split()[:2]) for line in tree.splitlines()}
index=[]
for package in sorted(metadata['packages'],key=lambda p:(p['name'],p['version'])):
    if package['name']=='clearwave' or (package['name'],'v'+package['version']) not in selected:continue
    folder=pathlib.Path(package['manifest_path']).parent
    out=destination/(package['name']+'-'+package['version'])
    names=[]
    for path in folder.rglob('*'):
        name=path.name.upper()
        if path.is_file() and (name.startswith(('LICENSE','COPYING','COPYRIGHT','OFL')) or name=='NOTICE'):
            relative=path.relative_to(folder)
            target=out/relative;target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(path,target);names.append(str(relative))
    supplemental=None
    if package['name'] in {'ecolor','eframe','egui','egui-winit','egui_glow','emath','epaint'}:supplemental=root/'licenses/egui'
    elif package['name'] in {'gl_generator','khronos_api'}:supplemental=root/'licenses/gl-rs'
    elif package['name']=='profiling':supplemental=root/'licenses/profiling'
    if not names and supplemental:
        for source in supplemental.iterdir():
            if source.is_file():
                out.mkdir(parents=True,exist_ok=True);shutil.copyfile(source,out/source.name);names.append(source.name)
    index.append({'name':package['name'],'version':package['version'],'license':package['license'],'repository':package['repository'],'files':names})
(destination/'index.json').write_text(json.dumps(index,indent=2)+'\n')
print(f'Collected notices for {len(index)} dependencies in {destination}')
missing=[p['name'] for p in index if not p['files']]
if missing:print('No standalone notice file in cached package (review metadata/source before redistribution): '+', '.join(missing))
