#!/usr/bin/env python3
"""Build a pinned pure Python wheel and exercise its workers from a clean venv."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import venv
import zipfile

from check_python_helper import ROOT, digest, sources


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--build-python',type=Path,default=Path(sys.executable))
    args=parser.parse_args()
    output=args.output.absolute()
    assert output.parent.resolve().is_relative_to((ROOT/'.local').resolve())
    assert re.fullmatch(r'[a-zA-Z0-9_-]+',output.name)
    output.mkdir(parents=True,mode=0o700,exist_ok=False)
    private=output.with_name(output.name+'-private');private.mkdir(mode=0o700,exist_ok=False)
    dist=ROOT/'dist/sdk/python'/output.name;dist.mkdir(mode=0o700,parents=True,exist_ok=False)
    before=sources()
    extra=['sdk/python/build-requirements.txt','.github/workflows/gate6-linux-fixtures.yml']
    for name in extra:before[name]={'sha256':digest(ROOT/name),'mode':(ROOT/name).stat().st_mode & 0o777}
    (output/'source-inputs.json').write_text(json.dumps(before,indent=2)+'\n')
    environment=dict(os.environ,PYTHONNOUSERSITE='1',PYTHONDONTWRITEBYTECODE='1')
    for key in ['PYTHONPATH','PYTHONHOME','PYTHONUSERBASE']:environment.pop(key,None)
    def run(name,command,*,cwd=private,timeout=30):
        phase={'passed':False,'phase':name,'source_inputs_sha256':digest(output/'source-inputs.json')}
        (output/'summary.json').write_text(json.dumps(phase,indent=2)+'\n')
        with (private/(name+'.log')).open('wb') as log:
            child=subprocess.Popen(command,cwd=cwd,env=environment,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            timed=False;forced=False
            try:child.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                timed=True;os.killpg(child.pid,signal.SIGTERM)
                try:child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    forced=True;os.killpg(child.pid,signal.SIGKILL);child.wait(timeout=5)
        phase.update(exit_code=child.returncode,timed_out=timed,forced_cleanup=forced,log_sha256=digest(private/(name+'.log')))
        (output/'summary.json').write_text(json.dumps(phase,indent=2)+'\n')
        assert child.returncode==0 and not timed and not forced,name
        return (private/(name+'.log')).read_text()
    versions=json.loads(run('builder',[str(args.build_python.absolute()),'-c',
        'import importlib.metadata,json; print(json.dumps({n:importlib.metadata.version(n) for n in ("setuptools","wheel")}))']))
    assert versions=={'setuptools':'80.9.0','wheel':'0.45.1'}
    source=private/'source';source.mkdir(mode=0o700)
    shutil.copy2(ROOT/'sdk/python/pyproject.toml',source/'pyproject.toml')
    shutil.copytree(ROOT/'sdk/python/connect_sdk',source/'connect_sdk',ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
    environment['SOURCE_DATE_EPOCH']=subprocess.check_output(['git','show','-s','--format=%ct','HEAD'],cwd=ROOT,text=True).strip()
    run('build',[str(args.build_python.absolute()),'-c','from setuptools.build_meta import build_wheel; build_wheel('+repr(str(dist))+')'],cwd=source)
    wheels=list(dist.glob('*.whl'));assert len(wheels)==1;wheel=wheels[0]
    runtime={n.removeprefix('sdk/python/'):v for n,v in before.items() if n.startswith('sdk/python/connect_sdk/')}
    with zipfile.ZipFile(wheel) as archive:
        names=archive.namelist();assert len(names)==len(set(names))
        assert {n for n in names if n.startswith('connect_sdk/')}==set(runtime)
        for name,guard in runtime.items():assert hashlib.sha256(archive.read(name)).hexdigest()==guard['sha256'],name
        metadata=archive.read('connect_sdk-0.1.0.dist-info/METADATA').decode()
        assert 'Requires-Python: >=3.11' in metadata and 'Requires-Dist:' not in metadata
    project=private/'external-project';project.mkdir(mode=0o700)
    target=project/'venv';venv.EnvBuilder(with_pip=False,symlinks=True).create(target)
    python=target/'bin/python'
    run('install',[sys.executable,'-m','pip','--python',str(python),'install','--no-cache-dir','--disable-pip-version-check','--no-index','--no-deps',str(wheel)],cwd=project)
    consumer=project/'consume.py';shutil.copy2(ROOT/'sdk/tests/python_installed_consumer.py',consumer)
    installed=json.loads(run('consumer',[str(python),'-I',str(consumer)],cwd=project,timeout=40))
    assert installed['passed'] and installed['owned_workers_joined']==6
    after=sources()
    for name in extra:after[name]={'sha256':digest(ROOT/name),'mode':(ROOT/name).stat().st_mode & 0o777}
    assert before==after
    result={'passed':True,'scope':'Clean external venv wheel import and actual local synthetic HTTP control/worker ownership through public async and synchronous SDKs. No original native helper/server, restart, named remote or full SDK qualification.',
        'build_tools':versions,'wheel_sha256':digest(wheel),'wheel_name':wheel.name,'wheel_runtime_files_verified':len(runtime),
        'source_inputs_unchanged':True,'source_count':len(before),'source_inputs_sha256':digest(output/'source-inputs.json'),
        'consumer':installed,'consumer_log_sha256':digest(private/'consumer.log'),
        'raw_logs_and_temporary_state_excluded_from_artifact':True,'finished_at':datetime.datetime.now(datetime.timezone.utc).isoformat()}
    (output/'summary.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result))


if __name__=='__main__':main()
