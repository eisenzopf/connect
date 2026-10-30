"""One source-pinned coturn4.18 container on an already owned internal bridge."""
import base64
import hashlib
import hmac
import ipaddress
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import time
import uuid


def command(args, timeout=15):
    try:
        return subprocess.check_output(args,text=True,stderr=subprocess.PIPE,timeout=timeout).strip()
    except subprocess.CalledProcessError as error:
        error.owned_docker_failure=next((name for name,pattern in [
            ('user-configured-subnet-required','user configured subnets'),
            ('permission','permission denied'),('capability','operation not permitted'),
            ('capacity','no space left'),('mount','invalid mount config')]
            if pattern in (error.stderr or '').lower()),'other')
        raise


def private_write(path,text):
    with open(path,'x',opener=lambda name,flags:os.open(name,flags,0o600)) as stream:stream.write(text)


class OwnedBridgeTurn:
    def __init__(self,network,image,probe_image,gateway,directory,transport):
        assert transport in ('udp','tcp')
        assert re.fullmatch(r'[0-9a-f]{64}',network)
        assert all(re.fullmatch(r'sha256:[0-9a-f]{64}',value) for value in (image,probe_image))
        self.network,self.image,self.probe_image=network,image,probe_image
        self.gateway=ipaddress.IPv4Address(gateway);assert self.gateway.is_private
        assert not self.gateway.is_loopback and not self.gateway.is_link_local
        self.directory=Path(directory).resolve();self.transport=transport
        assert self.directory.is_relative_to(Path(__file__).resolve().parents[2]/'.local')
        self.cid=None;self.ip=None;self.report=dict(passed=False,transport=transport)

    def state(self):
        return json.loads(command(['docker','inspect','--format','{{json .State}}',self.cid]))

    def logs(self):
        text=command(['docker','logs',self.cid]);(self.directory/'turnserver.log').write_text(text)
        return text

    def start(self):
        info=json.loads(command(['docker','network','inspect',self.network]))[0]
        assert info['Id']==self.network and info['Internal'] and info['Driver']=='bridge'
        subnet=ipaddress.IPv4Network(info['IPAM']['Config'][0]['Subnet']);assert self.gateway in subnet
        assert subnet.is_private
        self.directory.mkdir(mode=0o700,exist_ok=False)
        image=json.loads(command(['docker','image','inspect',self.image]))[0]
        probe=json.loads(command(['docker','image','inspect',self.probe_image]))[0]
        assert image['Id']==self.image and image['Os']=='linux' and image['Architecture'] in ('arm64','amd64')
        assert probe['Id']==self.probe_image and probe['Architecture']==image['Architecture'] and probe['Os']=='linux'
        base=['docker','run','--rm','--read-only','--network','none','--cap-drop','ALL',
            '--cap-add','NET_BIND_SERVICE','--security-opt','no-new-privileges']
        version=command([*base,'--entrypoint','/usr/bin/turnserver',self.image,'--version'])
        assert version=='4.18.0','The zero-allocation drain oracle is qualified only for coturn4.18.0'
        binary=command([*base,'--entrypoint','/usr/bin/sha256sum',self.image,'/usr/bin/turnserver']).split()[0]
        assert re.fullmatch(r'[0-9a-f]{64}',binary)
        self.report.update(image_id=self.image,architecture=image['Architecture'],version=version,binary_sha256=binary)
        self.report['startup_stage']='container-create'
        gate='''i=0
while [ ! -f /fixture/turnserver.conf ]; do
    i=$((i+1))
    [ "$i" -lt 400 ] || exit 1
    sleep 0.025
done
exec /usr/bin/turnserver -c /fixture/turnserver.conf'''
        self.cid=command(['docker','create','--name','connect-g6-turn-'+uuid.uuid4().hex[:12],
            '--read-only','--network',self.network,'--user',f'{os.getuid()}:{os.getgid()}',
            '--cap-drop','ALL','--cap-add','NET_BIND_SERVICE','--security-opt','no-new-privileges',
            '--tmpfs','/tmp:rw,nosuid,nodev,mode=1777,size=16m',
            '--mount',f'type=bind,source={self.directory},target=/fixture','--workdir','/fixture',
            '--entrypoint','/bin/sh',self.image,'-eu','-c',gate])
        assert re.fullmatch(r'[0-9a-f]{64}',self.cid)
        self.report['startup_stage']='network-start'
        command(['docker','start',self.cid])
        container=json.loads(command(['docker','inspect',self.cid]))[0]
        networks=list(container['NetworkSettings']['Networks'].values())
        assert len(networks)==1 and networks[0]['NetworkID']==self.network
        self.ip=ipaddress.IPv4Address(networks[0]['IPAddress'])
        assert self.ip in subnet and self.ip!=self.gateway
        self.report['startup_stage']='private-configuration'
        shared=secrets.token_hex(32)
        self.username=f'{int(time.time())+180}:synthetic-g6-bridge'
        self.password=base64.b64encode(hmac.new(shared.encode(),self.username.encode(),hashlib.sha1).digest()).decode()
        private_write(self.directory/'turnserver.pending.conf','\n'.join([
            f'listening-ip={self.ip}',f'relay-ip={self.ip}','listening-port=3478',
            'min-port=32000','max-port=32031','relay-threads=1','realm=gate6.invalid','use-auth-secret',
            f'static-auth-secret={shared}',f'allowed-peer-ip={self.gateway}',f'allowed-peer-ip={self.ip}',
            'denied-peer-ip=0.0.0.0-255.255.255.255','no-multicast-peers','no-tls','no-dtls','no-tcp-relay',
            'no-tcp' if self.transport=='udp' else 'no-udp','fingerprint','verbose',
            'user-quota=8','total-quota=8','drain-min-allocations=0','userdb=/fixture/turn.sqlite',
            'pidfile=/fixture/turn.pid','log-file=stdout','simple-log','new-log-timestamp=false','']))
        os.replace(self.directory/'turnserver.pending.conf',self.directory/'turnserver.conf')
        self.url=f'turn:{self.ip}:3478?transport={self.transport}'
        private_write(self.directory/'fixture.json',json.dumps(dict(url=self.url,ip=str(self.ip),
            listener_port=3478,relay_port_start=32000,relay_port_end=32031,
            username=self.username,credential=self.password))+'\n')
        refs=self.directory/'secret-references.toml'
        private_write(refs,'version = 1\n'+''.join(f'\n[references.{name}]\nstorage = "environment"\nvariable = "{name}"\n'
            for name in ['CONNECT_GATE6_TURN_USER','CONNECT_GATE6_TURN_PASSWORD']))
        self.environment=dict(CONNECT_SECRET_REFERENCES=str(refs),CONNECT_GATE6_TURN_USER=self.username,
            CONNECT_GATE6_TURN_PASSWORD=self.password,CONNECT_G6_REMOTE_BROWSER_RELAY_CONFIG=str(self.directory/'fixture.json'))
        # A real unauthenticated STUN binding exchange in a separate owned
        # container proves the selected control listener, without external IO.
        code='''import socket,struct,secrets,time,sys
ip,kind=sys.argv[1:];transaction=secrets.token_bytes(12);request=struct.pack('!HHI12s',1,0,0x2112A442,transaction)
until=time.monotonic()+5
while True:
 try:
  with socket.socket(socket.AF_INET,socket.SOCK_STREAM if kind=='tcp' else socket.SOCK_DGRAM) as s:
   s.settimeout(.5)
   if kind=='udp':s.sendto(request,(ip,3478));response,source=s.recvfrom(2048);assert source==(ip,3478)
   else:
    s.connect((ip,3478));s.sendall(request);response=b''
    while len(response)<20:
     part=s.recv(20-len(response))
     if not part:raise OSError('owned STUN stream ended')
     response+=part
   assert len(response)>=20 and response[:2]==b'\\x01\\x01' and response[8:20]==transaction
   break
 except (OSError,AssertionError):
  if time.monotonic()>=until:raise
  time.sleep(.025)
print('owned-stun-binding-ready')'''
        probe_name='connect-g6-turn-probe-'+uuid.uuid4().hex[:12]
        self.report['startup_stage']='control-binding'
        try:
            ready=command(['docker','run','--rm','--name',probe_name,'--read-only','--network',self.network,'--cap-drop','ALL',
                '--security-opt','no-new-privileges','--env','PYTHONDONTWRITEBYTECODE=1',
                '--entrypoint','python3',self.probe_image,'-c',code,str(self.ip),self.transport],timeout=15)
        finally:
            subprocess.run(['docker','rm','--force',probe_name],capture_output=True,timeout=10)
        assert ready=='owned-stun-binding-ready'
        self.report['control_binding_ready']=True
        return self

    def close(self):
        if self.cid is None:return self.report
        forced=False
        try:
            if self.state()['Running']:
                command(['docker','kill','--signal','USR1',self.cid])
                began=time.monotonic()
                while 'Drain complete (0 allocations remaining, threshold 0)' not in self.logs():
                    assert self.state()['Running'] and time.monotonic()-began<10
                    time.sleep(.05)
                self.report['zero_allocations_before_exit']=True
                self.report['drain_seconds']=time.monotonic()-began
                assert self.report['drain_seconds']<=10
                exit_code=int(command(['docker','wait',self.cid],timeout=10))
                self.report['original_exit_code']=exit_code
                assert exit_code==0
            text=self.logs()
            self.report['allocation_successes']=len(re.findall(r'ALLOCATE processed, success',text))
            self.report['log_sha256']=hashlib.sha256(text.encode()).hexdigest()
            self.report['original_container_joined']=not self.state()['Running']
            self.report['passed']=bool(self.report.get('zero_allocations_before_exit')
                and self.report.get('original_exit_code')==0 and self.report['original_container_joined'])
        finally:
            if self.state()['Running']:
                forced=True;command(['docker','stop','--time','5',self.cid],timeout=15)
            command(['docker','rm',self.cid])
            self.cid=None;self.report.update(owned_container_removed=True,forced_cleanup=forced)
            self.report['passed']=self.report['passed'] and not forced
        return self.report
