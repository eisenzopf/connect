#!/usr/bin/env python3
"""Prepare an isolated installed Python native consumer; never contact a server."""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys

from check_python_helper import ROOT, digest


def prepare(output, build_python=Path(sys.executable)):
    output=Path(output).absolute()
    assert output.resolve().is_relative_to((ROOT/'.local').resolve())
    output.mkdir(mode=0o700,parents=True,exist_ok=False)
    evidence=output/(output.name+'-installation')
    environment=dict(os.environ,PYTHONNOUSERSITE='1',PYTHONDONTWRITEBYTECODE='1')
    for key in ('PYTHONPATH','PYTHONHOME','PYTHONUSERBASE'):environment.pop(key,None)
    with (output/'prepare.log').open('wb') as log:
        child=subprocess.Popen([sys.executable,str(ROOT/'sdk/tests/check_python_installation.py'),
            '--output',str(evidence),'--build-python',str(Path(build_python).absolute())],
            cwd=ROOT,env=environment,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        try:child.wait(timeout=180)
        except subprocess.TimeoutExpired:
            os.killpg(child.pid,signal.SIGTERM)
            try:child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid,signal.SIGKILL);child.wait(timeout=5)
            raise AssertionError('Installed Python package preparation deadline elapsed') from None
        assert child.returncode==0,'Installed Python package preparation failed; private log retained'
    installation=json.loads((evidence/'summary.json').read_text())
    assert installation['passed'] and installation['source_inputs_unchanged']
    project=evidence.with_name(evidence.name+'-private')/'external-project'
    python=project/'venv/bin/python'
    observed=json.loads(subprocess.check_output([str(python),'-I','-c',
        'import connect_sdk,json,sys; print(json.dumps({"path":connect_sdk.__path__[0],"version":sys.version.split()[0]}))'],
        cwd=project,env=environment,text=True,timeout=10))
    installed=Path(observed['path'])
    expected={p.name:digest(p) for p in (ROOT/'sdk/python/connect_sdk').iterdir() if p.is_file()}
    actual={p.name:digest(p) for p in installed.iterdir() if p.is_file()}
    assert actual==expected
    consumer=project/'native-consume.py'
    shutil.copyfile(ROOT/'sdk/tests/python_native_server_consumer.py',consumer)
    movement=project/'movement-consume.py'
    shutil.copyfile(ROOT/'sdk/tests/python_movement_server_consumer.py',movement)
    administrative=project/'admin-consume.py'
    shutil.copyfile(ROOT/'sdk/tests/python_administrative_server_consumer.py',administrative)
    consultation=project/'consultation-consume.py'
    shutil.copyfile(ROOT/'sdk/tests/python_consultation_server_consumer.py',consultation)
    result={'profile':'python-native-package/1','python':str(python.relative_to(ROOT)),
        'consumer':str(consumer.relative_to(ROOT)),'consumer_sha256':digest(consumer),
        'movement_consumer':str(movement.relative_to(ROOT)),'movement_consumer_sha256':digest(movement),
        'administrative_consumer':str(administrative.relative_to(ROOT)),'administrative_consumer_sha256':digest(administrative),
        'consultation_consumer':str(consultation.relative_to(ROOT)),'consultation_consumer_sha256':digest(consultation),
        'wheel_sha256':installation['wheel_sha256'],'installed_dist_matches':True,
        'installed_dist_sha256':actual,'python_version':observed['version'],
        'synthetic_http_workers_joined':installation['consumer']['owned_workers_joined'],
        'native_server_qualified':False,'publication':False}
    (output/'package-result.json').write_text(json.dumps(result,indent=2)+'\n')
    return result


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--build-python',type=Path,default=Path(sys.executable))
    args=parser.parse_args()
    print(json.dumps(prepare(args.output,args.build_python)))
