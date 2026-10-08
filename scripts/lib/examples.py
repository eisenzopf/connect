"""Independent example process helpers; state is owned by each application."""
import json,subprocess
from .local_process import ROOT
EXAMPLE=ROOT/'target/debug/connect-example'
def inspect(state):return json.loads(subprocess.check_output([str(EXAMPLE),'inspect','--state',str(state)],text=True,timeout=5))
def worker(state,role,ms=700,env=None,binary=EXAMPLE):
    return subprocess.Popen([str(binary),role,'--config',str(state/(role+'.toml')),'--run-for-ms',str(ms)],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,env=env)
def finish(process,expected=0):
    try:output,error=process.communicate(timeout=15)
    except BaseException:
        process.kill();process.communicate();raise
    assert process.returncode==expected,error
    return output
def pair(state):
    reviews=worker(state,'reviews');agent=worker(state,'agent')
    try:finish(reviews);finish(agent)
    finally:
        for process in [reviews,agent]:
            if process.poll() is None:process.kill();process.communicate()
