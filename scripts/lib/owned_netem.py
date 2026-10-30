"""Bounded UDP-only netem in one explicitly owned Docker network namespace."""
import json
import os
from pathlib import Path
import platform
import subprocess


class OwnedNetem:
    handles = {'to_client':'20:', 'to_server':'30:'}

    def __init__(self, start, end):
        assert platform.system()=='Linux' and Path('/.dockerenv').is_file()
        assert os.environ.get('CONNECT_GATE6_OWNED_BROWSER_CONTAINER')=='1'
        assert 1024<=start<=end<=65535 and end-start==7
        self.start,self.end=start,end
        self.installed=False

    def command(self,*args):
        return subprocess.check_output(['tc',*args],text=True,stderr=subprocess.PIPE,timeout=5)

    def qdiscs(self):
        return json.loads(self.command('-j','-s','qdisc','show','dev','lo'))

    def snapshot(self):
        rows={row['handle']:row for row in self.qdiscs()}
        return {direction:{field:int(rows[handle].get(field,0))
            for field in ['bytes','packets','drops','overlimits','requeues','backlog','qlen']}
            for direction,handle in self.handles.items()}

    def loss(self,percent):
        assert percent in (3,100)
        for direction,handle in self.handles.items():
            band='1:2' if direction=='to_client' else '1:3'
            self.command('qdisc','change','dev','lo','parent',band,'handle',handle,'netem','limit','512',
                'delay','50ms','50ms','loss','random',str(percent)+'%')

    def __enter__(self):
        before=self.qdiscs()
        assert len(before)==1 and before[0]['kind']=='noqueue' and before[0]['handle']=='0:'
        self.command('qdisc','add','dev','lo','root','handle','1:','prio','bands','3',
            'priomap',*(['0']*16))
        self.installed=True
        try:
            for direction,handle in self.handles.items():
                band='1:2' if direction=='to_client' else '1:3'
                self.command('qdisc','add','dev','lo','parent',band,'handle',handle,'netem',
                    'limit','512','delay','50ms','50ms','loss','random','3%')
                for port in range(self.start,self.end+1):
                    self.command('filter','add','dev','lo','parent','1:','protocol','ip','prio','1','u32',
                        'match','ip','protocol','17','0xff','match','ip',
                        'sport' if direction=='to_client' else 'dport',str(port),'0xffff','flowid',band)
            return self
        except BaseException:
            self.close()
            raise

    def close(self):
        if self.installed:
            rows=self.qdiscs()
            assert any(row['handle']=='1:' and row['kind']=='prio' for row in rows)
            self.command('qdisc','del','dev','lo','root')
            self.installed=False
        after=self.qdiscs()
        assert len(after)==1 and after[0]['kind']=='noqueue' and after[0]['handle']=='0:'

    def __exit__(self,*_):
        self.close()
