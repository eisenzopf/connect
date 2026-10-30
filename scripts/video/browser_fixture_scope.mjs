/** Bound network exceptions to the explicitly owned Linux fixture. */
import {isIPv4} from 'node:net';

export function validateBrowserFixtureScope(spec, runtime=process) {
    const bridge=spec.ownedContainerBridge;
    if(bridge){
        const privateIp=value=>isIPv4(value??'')&&(value.startsWith('10.')||value.startsWith('192.168.')
            ||value.startsWith('172.')&&Number(value.split('.')[1])>=16&&Number(value.split('.')[1])<=31);
        const relay=bridge.require_relay===true;
        if(bridge.require_relay!==undefined&&typeof bridge.require_relay!=='boolean')
            throw Error('Explicit owned internal bridge configuration required');
        const ports=Number.isInteger(bridge.port_start)&&Number.isInteger(bridge.port_end)
            &&(relay?bridge.port_start===32000&&bridge.port_end===32031:
                bridge.port_start>=20000&&bridge.port_end===bridge.port_start+15&&bridge.port_end<=60015);
        if(runtime.platform!=='linux'||runtime.arch!=='x64'||runtime.env.CONNECT_GATE6_OWNED_BROWSER_CONTAINER!=='1'
            ||spec.designatedRemoteHost!=='connect.fixture.internal'||!privateIp(bridge.ip)||!ports
            ||relay!==Boolean(spec.requireRelay))throw Error('Explicit owned internal bridge configuration required');
    }
    if(spec.iceRestart&&(spec.profileMode!=='clean'||spec.participants.length!==2
        ||spec.designatedRemoteHost&&!(bridge?.require_relay===true&&spec.requireRelay)))
        throw Error('ICE restart qualification requires two owned clean-profile participants');
    if(spec.requireRelay&&(!spec.iceRestart||spec.participants.some(p=>p.connection?.iceTransportPolicy!=='relay')))
        throw Error('Owned relay qualification requires explicit relay-only original-peer ICE recovery');
}
