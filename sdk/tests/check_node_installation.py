#!/usr/bin/env python3
"""Pack the current Node/browser SDK and import it from a clean external install.

This qualifies archive contents and synthetic HTTP entry points only; an actual
matching native server/helper and both platform candidates remain required.
"""
import argparse
import base64
import io
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import subprocess
import tarfile
import tempfile
import time
import urllib.request

ROOT=Path(__file__).resolve().parents[2]


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream,"sha256").hexdigest()


def inputs():
    names=[ROOT/"sdk/typescript/package-lock.json",ROOT/"sdk/typescript/package.json",ROOT/"sdk/typescript/LICENSE",ROOT/"sdk/typescript/README.md",
        Path(__file__).resolve()]
    names+=list((ROOT/"sdk/typescript/dist").rglob("*"))
    return {str(p.relative_to(ROOT)):dict(sha256=digest(p),mode=p.stat().st_mode&0o777)
        for p in sorted(names) if p.is_file() and not p.is_symlink()}


def dependency_inputs(package, distribution):
    lock=json.loads((ROOT/"sdk/typescript/package-lock.json").read_text())
    pending=list(package.get("dependencies",{}))
    found={}
    while pending:
        name=pending.pop()
        if name in found:continue
        assert len(found)<32, "SDK dependency closure exceeds its fixture budget"
        selected=lock["packages"]["node_modules/"+name]
        url=selected["resolved"]
        assert url.startswith("https://registry.npmjs.org/") and "?" not in url
        request=urllib.request.Request(url,headers={"User-Agent":"Connect SDK candidate audit"})
        with urllib.request.urlopen(request,timeout=30) as response:body=response.read(8*1024*1024+1)
        assert len(body)<=8*1024*1024
        algorithm,expected=selected["integrity"].split("-",1)
        assert algorithm in ("sha256","sha512")
        assert hashlib.new(algorithm,body).digest()==base64.b64decode(expected,validate=True)
        metadata=None;license=None
        with tarfile.open(fileobj=io.BytesIO(body),mode="r:gz") as archive:
            members=archive.getmembers();assert len(members)<=10000 and sum(m.size for m in members)<=32*1024*1024
            for member in members:
                path=PurePosixPath(member.name)
                assert not path.is_absolute() and ".." not in path.parts and (member.isfile() or member.isdir())
                if member.isfile() and len(path.parts)==2:
                    if path.name=="package.json":metadata=json.loads(archive.extractfile(member).read())
                    if path.name=="LICENSE":license=archive.extractfile(member).read()
        assert metadata is not None and metadata["name"]==name and metadata["version"]==selected["version"]
        assert license, "A dependency needs its original license text"
        directory=distribution/"dependencies";directory.mkdir(exist_ok=True)
        leaf=name.removeprefix("@").replace("/","-")+"-"+selected["version"]
        path=directory/(leaf+".tgz");path.write_bytes(body)
        (directory/(leaf+".LICENSE")).write_bytes(license)
        found[name]=dict(name=name,version=selected["version"],file_name=path.name,
            sha256=digest(path),integrity=selected["integrity"],source_url=url,
            license_sha256=hashlib.sha256(license).hexdigest())
        pending.extend(metadata.get("dependencies",{}))
    return list(found.values())


CONSUMER = """
import assert from 'node:assert/strict';
import http from 'node:http';
import {fileURLToPath} from 'node:url';
import {NodeClient,HttpClient} from '@rudeless/connect';
import {BrowserHttpClient} from '@rudeless/connect/browser';
assert.equal(typeof NodeClient,'function');
assert.equal(typeof BrowserHttpClient,'function');
assert.ok(fileURLToPath(import.meta.resolve('@rudeless/connect')).startsWith(process.cwd()+'/node_modules/'));
let requests=0;
const server=http.createServer((req,res)=>{
    let body='';
    req.on('data',chunk=>{body+=chunk;if(body.length>4096)req.destroy();});
    req.on('end',()=>{
        assert.equal(req.url,'/v1/operations');
        assert.equal(req.headers.authorization,'Bearer synthetic-candidate');
        assert.deepEqual(JSON.parse(body),{op:'health'});
        requests++;
        res.writeHead(200,{'content-type':'application/json'});
        res.end(JSON.stringify({profile:'connect-control/1',result:{Ok:{kind:'health',data:{
            installation_id:'synthetic-candidate',schema:1,profile:'connect-control/1'
        }}}}));
    });
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
try{
    const endpoint='http://127.0.0.1:'+server.address().port;
    for(const Type of [HttpClient,BrowserHttpClient]){
        const value=await new Type(endpoint,'synthetic-candidate',3000).health();
        assert.deepEqual(value,{
            installation_id:'synthetic-candidate',schema:1,profile:'connect-control/1'
        });
    }
    assert.equal(requests,2);
}finally{
    server.closeAllConnections();
    await new Promise(resolve=>server.close(resolve));
}
process.stdout.write(JSON.stringify({passed:true,requests,node_entry_imported:true,browser_entry_imported:true,installed_from_external_project:true})+'\\n');
"""


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output",type=Path,required=True)
    args=parser.parse_args()
    os.umask(0o077)
    output=args.output.resolve()
    assert output.is_relative_to((ROOT/".local").resolve()) and not output.exists()
    output.mkdir(parents=True,mode=0o700)
    private=output/"private";private.mkdir(mode=0o700)
    package=json.loads((ROOT/"sdk/typescript/package.json").read_text())
    assert package["name"]=="@rudeless/connect" and package["version"]=="0.1.0"
    distribution=ROOT/"dist/sdk/typescript"/output.name
    distribution.mkdir(parents=True,exist_ok=False)
    before=inputs()
    (output/"source-inputs.json").write_text(json.dumps(before,sort_keys=True)+"\n")
    environment=dict(os.environ)
    for name in ("NODE_PATH","NODE_OPTIONS"):environment.pop(name,None)
    def run(name,command,cwd,budget=30):
        started=time.monotonic()
        result=subprocess.run(command,cwd=cwd,env=environment,capture_output=True,timeout=budget)
        (private/(name+".log")).write_bytes(result.stdout+result.stderr)
        assert result.returncode==0, "Bounded installed SDK step failed: "+name
        return result.stdout.decode(),round(time.monotonic()-started,3)
    raw,pack_seconds=run("pack",["npm","--offline","--ignore-scripts","--cache",str(private/"pack-cache"),
        "pack","--json","--pack-destination",str(distribution)],ROOT/"sdk/typescript")
    json.loads(raw)  # CLI report shape varies; verify the actual archive instead.
    archives=list(distribution.glob("*.tgz"));assert len(archives)==1
    archive=archives[0]
    assert archive.name==package["name"].removeprefix("@").replace("/","-")+"-"+package["version"]+".tgz"
    assert archive.parent==distribution and archive.suffix==".tgz"
    verified=0
    with tarfile.open(archive,"r:gz") as source:
        seen=set()
        for member in source:
            path=PurePosixPath(member.name)
            assert not path.is_absolute() and ".." not in path.parts and path.parts[0]=="package"
            assert member.name not in seen;seen.add(member.name)
            assert ".deps" not in path.parts and ".local" not in path.parts
            if member.isdir():continue
            assert member.isfile(),"Only ordinary SDK files belong in the archive"
            name="/".join(path.parts[1:])
            data=source.extractfile(member).read()
            if name=="package.json":
                value=json.loads(data)
                for key in ("name","version","exports","engines","license"):
                    assert value[key]==package[key]
                assert value.get("dependencies",{})==package.get("dependencies",{}) and not value.get("optionalDependencies")
            else:
                expected=ROOT/"sdk/typescript"/name
                assert expected.is_file() and not expected.is_symlink()
                assert hashlib.sha256(data).hexdigest()==digest(expected),name
            verified+=1
        assert {"package/LICENSE","package/README.md","package/dist/index.js",
                "package/dist/browser.js","package/dist/index.d.ts","package/dist/browser.d.ts"}<=seen
    dependencies=dependency_inputs(package,distribution)
    with tempfile.TemporaryDirectory(prefix="connect-node-installed-") as directory:
        project=Path(directory).resolve()
        assert not project.is_relative_to(ROOT.resolve())
        (project/"package.json").write_text('{"private":true,"type":"module"}\n')
        cache=project/"empty-cache";assert not cache.exists()
        _,install_seconds=run("install",["npm","install","--offline","--ignore-scripts",
            "--no-audit","--no-fund","--cache",str(cache),str(archive),*[str(distribution/"dependencies"/d["file_name"]) for d in dependencies]],project)
        installed=project/"node_modules/@rudeless/connect"
        assert not installed.is_symlink() and installed.is_dir()
        assert (installed/"LICENSE").read_bytes()==(ROOT/"LICENSE").read_bytes()
        tool=ROOT/"sdk/typescript/node_modules/typescript/bin/tsc"
        assert tool.is_file(), "The locked SDK compiler is required for declaration validation"
        compiler=json.loads((ROOT/"sdk/typescript/node_modules/typescript/package.json").read_text())["version"]
        types=project/"consume.ts"
        types.write_text("import {NodeClient,HttpClient} from '@rudeless/connect';\nimport {BrowserClient,BrowserHttpClient} from '@rudeless/connect/browser';\nconst native:typeof NodeClient=NodeClient; const http:typeof HttpClient=HttpClient; const browser:typeof BrowserClient=BrowserClient; const browserHttp:typeof BrowserHttpClient=BrowserHttpClient; void [native,http,browser,browserHttp];\n")
        _,type_seconds=run("types",["node",str(tool),"--strict","--noEmit","--module","NodeNext",
            "--moduleResolution","NodeNext","--target","ES2022","--lib","ES2022,DOM,DOM.Iterable",
            "--typeRoots",str(project/"node_modules/@types"),"--types","node",str(types)],project)
        raw,consumer_seconds=run("consumer",["node","--input-type=module","-e",CONSUMER],project,15)
        consumer=json.loads(raw);assert consumer["passed"]
    assert inputs()==before
    report=dict(passed=True,format="connect/installed-node-sdk-candidate/1",
        scope="Local clean external empty-cache archive install and actual synthetic HTTP Node/browser entry points. No real browser, native helper/server, platform pair or whole SDK qualification.",
        package_name=package["name"],version=package["version"],dependency_archives=dependencies,
        archive_name=archive.name,archive_sha256=digest(archive),files_verified=verified,
        consumer_outside_repository=True,empty_cache=True,install_scripts_disabled=True,
        license_text_verified=True,source_inputs_unchanged=True,
        source_inputs_sha256=digest(output/"source-inputs.json"),consumer=consumer,
        elapsed_seconds=dict(pack=pack_seconds,install=install_seconds,types=type_seconds,consumer=consumer_seconds),
        external_declarations_checked=True,typescript_version=compiler,compiler_entry_sha256=digest(tool),
        node_version=subprocess.check_output(["node","--version"],text=True).strip())
    (output/"summary.json").write_text(json.dumps(report,indent=2)+"\n")
    print(json.dumps(report))


if __name__=="__main__":main()
