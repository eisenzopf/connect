import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserCaptureControls,BrowserOperationProgress,browserInputDevices} from '../typescript/dist/browser_ui.js';
import {BrowserSdkError} from '../typescript/dist/browser_sdk.js';
class Element extends EventTarget {
 children=[];disabled=false;attributes={};textContent='';
 focus(){document.activeElement=this;}
 append(...nodes){this.children.push(...nodes);}
 setAttribute(key,value){this.attributes[key]=value;}
}
function deferred(){let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return{promise,resolve,reject};}
async function dom(fn){const descriptor=Object.getOwnPropertyDescriptor(globalThis,'document');globalThis.document={createElement:()=>new Element()};try{return await fn();}finally{if(descriptor)Object.defineProperty(globalThis,'document',descriptor);else delete globalThis.document;}}
async function devices(enumerate,fn){const descriptor=Object.getOwnPropertyDescriptor(globalThis,'navigator');Object.defineProperty(globalThis,'navigator',{configurable:true,value:{mediaDevices:{enumerateDevices:enumerate,getUserMedia(){throw Error('enumeration cannot prompt');}}}});try{return await fn();}finally{if(descriptor)Object.defineProperty(globalThis,'navigator',descriptor);else delete globalThis.navigator;}}
const tick=()=>new Promise(resolve=>setTimeout(resolve,0));
test('input enumeration has a finite cancellable read budget and cannot request capture permission',async()=>{
 let calls=0;await devices(()=>{calls++;return new Promise(()=>{});},async()=>{
 const abort=new AbortController();abort.abort();await assert.rejects(()=>browserInputDevices({timeoutMs:100,signal:abort.signal}),{kind:'cancelled'});assert.equal(calls,0);
 await assert.rejects(()=>browserInputDevices({timeoutMs:10}),{kind:'deadline'});assert.equal(calls,1);
 });
});
test('input enumeration returns immutable local input choices and excludes output/group IDs',()=>devices(async()=>[{kind:'audioinput',deviceId:'mic',label:'',groupId:'private-group'},{kind:'videoinput',deviceId:'cam',label:'<script>untrusted label</script>'},{kind:'audiooutput',deviceId:'out',label:'speaker'}],async()=>{
 const d=await browserInputDevices({timeoutMs:100});assert.deepEqual(d,{microphones:[{id:'mic',label:'Microphone 1'}],cameras:[{id:'cam',label:'<script>untrusted label</script>'}]});assert(Object.isFrozen(d)&&Object.isFrozen(d.cameras[0]));
}));
test('capture UI deduplicates pending clicks and stops late owners after closing without another capture',()=>dom(async()=>{
 const late=deferred();let calls=0,closed=0;
 const controls=new BrowserCaptureControls({capture:()=>{calls++;return late.promise;}},{audio:true},{timeoutMs:100});
 const first=controls.start();assert.equal(controls.start(),first);assert.equal(controls.state,'pending');assert.equal(controls.element.attributes['aria-busy'],'true');await tick();controls.close();late.resolve({close(){closed++;},closed:false});await assert.rejects(first,{kind:'cancelled'});assert.equal(calls,1);assert.equal(closed,1);assert.equal(controls.state,'closed');assert.equal(controls.element.attributes['aria-busy'],'false');
}));
test('capture permission failure has a safe accessible retry for the same Connection and copied selection',()=>dom(async()=>{
 const denied=new BrowserSdkError('application','read-only',undefined,{code:403,category:'capture',reason:'permission_denied'});let calls=0,stop=0,seen;
 const selection={audio:true},options={timeoutMs:100};const controls=new BrowserCaptureControls({async capture(input,wait){calls++;seen={...input,timeout:wait.timeoutMs};if(calls===1)throw denied;return{closed:false,close(){stop++;}};}},selection,options);
 selection.audio=false;options.timeoutMs=60000;
 await assert.rejects(controls.start(),error=>error===denied);assert.equal(controls.state,'failed');assert.match(controls.element.children[0].textContent,/permission was denied/);assert.equal(controls.element.children[0].attributes.role,'status');assert.equal(controls.element.children[1].type,'button');assert.equal(controls.element.children[1].disabled,false);
 await controls.start();assert.equal(controls.state,'ready');assert.equal(seen.audio,true);assert.equal(seen.timeout,100);assert.equal(calls,2);controls.close();assert.equal(stop,1);
}));
test('capture stop aborts a pending permission request and cannot accept its later success',()=>dom(async()=>{
 const late=deferred();let signal,stops=0;
 const controls=new BrowserCaptureControls({capture:async(_,options)=>{signal=options.signal;return late.promise;}},{audio:true},{timeoutMs:100});const pending=controls.start();await tick();controls.stop();assert.equal(signal.aborted,true);late.resolve({closed:false,close(){stops++;}});await assert.rejects(pending,{kind:'cancelled'});assert.equal(stops,1);assert.equal(controls.state,'idle');controls.close();
}));
test('operation progress retries only original reads and keeps its captured observation budget',()=>dom(async()=>{
 let reads=0;const options={timeoutMs:100},completion={value:{id:'original'},snapshot:{state:'completed'}};
 const view=new BrowserOperationProgress({async wait(wait){reads++;assert.equal(wait.timeoutMs,100);if(reads===1)throw new BrowserSdkError('deadline','read-only');return completion;}},options);options.timeoutMs=60000;
 await assert.rejects(view.observe());assert.equal(view.state,'failed');assert.match(view.element.children[0].textContent,/original operation/);assert.equal(await view.observe(),completion);assert.equal(reads,2);assert.equal(view.state,'ready');view.close();
}));
test('operation panel closure cancels only its read and a late completion cannot update the closed panel',()=>dom(async()=>{
 const late=deferred();let signal;const view=new BrowserOperationProgress({wait:options=>{signal=options.signal;return late.promise;}},{timeoutMs:100});const pending=view.observe();assert.equal(view.observe(),pending);await tick();view.close();assert(signal.aborted);late.resolve({value:'historical'});await assert.rejects(pending,{kind:'cancelled'});assert.equal(view.state,'closed');assert.equal(view.element.children[0].textContent,'Operation progress closed.');
}));
test('pre-aborted capture and operation panels cannot call their underlying handles',()=>dom(async()=>{
 const abort=new AbortController();abort.abort();let called=0;const options={timeoutMs:100,signal:abort.signal};const capture=new BrowserCaptureControls({capture:()=>{called++;}},{audio:true},options),view=new BrowserOperationProgress({wait:()=>{called++;}},options);
 await assert.rejects(capture.start(),{kind:'cancelled'});await assert.rejects(view.observe(),{kind:'cancelled'});assert.equal(called,0);
}));

test('capture UI cannot publish ready for an owner already retired before startup resolves',()=>dom(async()=>{
 const controls=new BrowserCaptureControls({capture:async()=>({closed:true,close(){}})},{audio:true},{timeoutMs:100});await assert.rejects(controls.start(),{kind:'cancelled'});assert.equal(controls.state,'failed');controls.close();
}));
test('capture UI preserves keyboard focus through pending and denied states without moving unrelated focus',()=>dom(async()=>{
 const late=deferred();const controls=new BrowserCaptureControls({capture:()=>late.promise},{audio:true},{timeoutMs:100});const [message,start,stop]=controls.element.children;start.focus();const pending=controls.start();assert.equal(document.activeElement,stop);late.reject(new BrowserSdkError('unsupported','read-only'));await assert.rejects(pending);assert.equal(document.activeElement,start);controls.close();
}));
test('capture panel retains a paused startup and resumes or pauses only the existing capture owner',()=>dom(async()=>{
 let callback,starts=0,resumes=0,pauses=0;const owner={closed:false,close(){this.closed=true;callback('closed');},pause(){pauses++;callback('paused');},async resume(){resumes++;callback('active');}};
 const controls=new BrowserCaptureControls({async capture(input){starts++;callback=input.state;callback('paused');return owner;}},{audio:true},{timeoutMs:100});await controls.start();assert.equal(controls.state,'paused');assert.equal(controls.element.children[3].textContent,'Resume capture');await controls.toggle();assert.equal(controls.state,'ready');await controls.toggle();assert.equal(controls.state,'paused');assert.equal(starts,1);assert.equal(resumes,1);assert.equal(pauses,1);controls.close();
}));
test('late capture resume cannot reopen a closed panel or start another capture',()=>dom(async()=>{
 const late=deferred();let callback,resumes=0;const owner={closed:false,close(){this.closed=true;callback('closed');},async resume(){resumes++;await late.promise;callback('active');}};
 const controls=new BrowserCaptureControls({async capture(input){callback=input.state;callback('paused');return owner;}},{audio:true},{timeoutMs:100});await controls.start();const pending=controls.toggle();assert.equal(controls.toggle(),pending);await tick();controls.close();late.resolve();await assert.rejects(pending,{kind:'cancelled'});assert.equal(controls.state,'closed');assert.equal(resumes,1);assert.equal(controls.element.children[3].disabled,true);
}));
