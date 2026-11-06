const check=(ok,message)=>{if(!ok)throw Error(message);};
const denseOwnedArray=value=>Array.isArray(value)&&Reflect.ownKeys(value).length===value.length+1
    &&Array.from({length:value.length},(_,index)=>Object.hasOwn(value,index)).every(Boolean);
const freeze=value=>{if(value&&typeof value==='object'){for(const child of Object.values(value))freeze(child);Object.freeze(value);}return value;};

/** Fixture references only. Actual client.close promises remain the caller's
 * originals; logical IDs, SDK state and public close/reconnect never change. */
export function browserMatrixRetiredReferences({actors,state,root=globalThis}){
    let entries,closing,results,counts,released,stateOwners;
    const capture=()=>{
        if(entries)return;
        entries=Object.entries(actors).map(([role,actor])=>({role,actor,client:actor.client}));
        counts=Object.freeze({authenticated_owners:entries.length,actual_connections:denseOwnedArray(state.connections)?state.connections.length:null});
        stateOwners={conversation:state.conversation,sessions:state.sessions,connections:state.connections,
            sessionHandles:Array.isArray(state.sessions)?state.sessions.slice():null,
            connectionHandles:Array.isArray(state.connections)?state.connections.slice():null};
    };
    const facts=()=>freeze({...counts,original_client_closes:(results??[]).map((result,index)=>({
        role:entries?.[index]?.role??released?.roles[index],status:result.status,
        ...(result.status==='rejected'?{name:typeof result.reason?.name==='string'?result.reason.name.slice(0,64):'Error',
            kind:typeof result.reason?.kind==='string'?result.reason.kind.slice(0,64):null,
            certainty:typeof result.reason?.certainty==='string'?result.reason.certainty.slice(0,64):null}:{})}))});
    return {
        closeClients(perform){
            capture();
            return closing??=(async()=>{
                results=await Promise.allSettled(entries.map(({client,role})=>Promise.resolve().then(()=>perform(client,role))));
                return results;
            })();
        },
        counts(){return counts??Object.freeze({authenticated_owners:Object.keys(actors).length,actual_connections:denseOwnedArray(state.connections)?state.connections.length:null});},
        release({renderer,source,transport,keepalive}){
            if(released)return released.observation;
            const failures=[];
            if(!results||results.some(result=>result.status!=='fulfilled'))failures.push('original-client-close-unjoined');
            if(renderer?.joined!==true||source?.joined!==true||source?.released!==true||transport?.joined!==true
                ||keepalive?.stopped!==true||keepalive?.original_batch_joined!==true||keepalive?.retained_client_references!==0)failures.push('original-local-owner-unjoined');
            if(root.matrixActors!==actors)failures.push('foreign-original-actors-global');
            const roles=Reflect.ownKeys(actors),originalRoles=entries?.map(entry=>entry.role);
            if(!originalRoles||roles.length!==originalRoles.length||roles.some(role=>typeof role!=='string'||!originalRoles.includes(role)))
                failures.push('foreign-original-actor-role-set');
            if(entries?.some(({role,actor,client})=>actors[role]!==actor||actor.client!==client))failures.push('foreign-original-actor-owner');
            const available=stateOwners&&denseOwnedArray(stateOwners.sessionHandles)&&denseOwnedArray(stateOwners.connectionHandles)
                &&typeof stateOwners.conversation?.id==='string';
            const sameHandles=(current,original)=>denseOwnedArray(current)&&current.length===original.length
                &&original.every((owner,index)=>current[index]===owner);
            if(!available)failures.push('original-state-ownership-unavailable');
            else if(state.conversation!==stateOwners.conversation||state.sessions!==stateOwners.sessions||state.connections!==stateOwners.connections
                ||!sameHandles(state.sessions,stateOwners.sessionHandles)||!sameHandles(state.connections,stateOwners.connectionHandles))
                failures.push('foreign-original-state-owner');
            if(failures.length)return freeze({released:false,failures,counts:counts??this.counts()});
            const closedRoles=entries.map(entry=>entry.role),snapshot=facts();
            // Capture scalar/report facts before removing only actual SDK
            // wrapper references. state.report, logical refs and IDs survive.
            for(const {actor} of entries)delete actor.client;
            delete root.matrixActors;
            state.conversation=undefined;state.sessions.length=0;state.connections.length=0;
            for(const entry of entries){entry.client=undefined;entry.actor=undefined;}
            entries.length=0;
            stateOwners.sessionHandles.length=0;stateOwners.connectionHandles.length=0;
            stateOwners.conversation=undefined;stateOwners.sessions=undefined;stateOwners.connections=undefined;stateOwners=undefined;
            const observation=freeze({released:true,failures:[],counts:snapshot,
                retained_actor_clients:0,retained_state_handles:0});
            released={roles:closedRoles,observation};return observation;
        },
        facts(){return results?facts():freeze({...this.counts(),original_client_closes:[]});},
    };
}
