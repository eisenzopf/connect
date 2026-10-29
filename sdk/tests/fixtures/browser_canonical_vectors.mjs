import {readFileSync} from 'node:fs';
const load=name=>JSON.parse(readFileSync(new URL('../../../contracts/uctp-vectors/'+name,import.meta.url)));
export function canonicalVectors(){
 const baseline=load('gate6-envelope.json').vectors,templates=new Map(baseline.filter(v=>v.valid).map(v=>[v.envelope.type,v.envelope]));
 const extra=load('browser-canonical.json').vectors.map(vector=>{
  const envelope=structuredClone(templates.get(vector.template));
  for(const [path,value] of Object.entries(vector.set??{})){const keys=path.split('.');let current=envelope;for(const key of keys.slice(0,-1))current=current[key]??=(Object.create(null));current[keys.at(-1)]=structuredClone(value);}
  for(const path of vector.remove??[]){const keys=path.split('.');let current=envelope;for(const key of keys.slice(0,-1))current=current[key];delete current[keys.at(-1)];}
  if(vector.body){const {unit,count,encoding}=vector.body,text=unit.repeat(count);envelope.payload.body_encoding=encoding;envelope.payload.body=encoding==='base64'?Buffer.from(text).toString('base64'):text;}
  if(vector.nest){let value=null;for(let i=0;i<vector.nest.levels;i++)value={node:value};envelope.payload.vendor=value;}
  if(vector.array)envelope.payload.vendor=Array(vector.array.count).fill(0);
  return {id:vector.id,valid:vector.valid,envelope};
 });
 return [...baseline,...extra];
}
