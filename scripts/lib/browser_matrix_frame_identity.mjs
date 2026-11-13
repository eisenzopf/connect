import {markerFrames} from '../video/marker_calibration.mjs';
export const MATRIX_FRAME_ID_PROFILE='connect/matrix-decoded-source-frame-id/1';
const WIDTH=128,HEIGHT=72,FRAMES=1860;
const pulseFrames=new Set(markerFrames().filter(frame=>frame<FRAMES).flatMap(first=>[first,first+1,first+2]));
const unavailable=reason=>Object.freeze({profile:MATRIX_FRAME_ID_PROFILE,status:'unavailable',reason,
    ordinal:null,observed_source_id:null,pulse_exception:false});

/** Decode ONLY the declared original matrix footer in the already sampled
 * 128x72 RGBA pixels. These are pixels, never an estimated clock/counter. */
export function readMatrixFrameOrdinal(pixels){
    if(!(pixels instanceof Uint8Array||pixels instanceof Uint8ClampedArray)||pixels.length!==WIDTH*HEIGHT*4)
        return unavailable('Exact original 128x72 RGBA sample required');
    let ordinal=0;
    for(let bit=0;bit<11;bit++){
        // Source centres x=16+32*bit, y=357.5 lie within the existing
        // 32x8 footer cells after 640x360 ->128x72 sampling. No new readback.
        const x=Math.floor((16+32*bit)/5),offset=(71*WIDTH+x)*4;
        if(pixels[offset+3]!==255)return unavailable('Opaque original footer required');
        const luma=.299*pixels[offset]+.587*pixels[offset+1]+.114*pixels[offset+2];
        if(luma<=64)continue;
        if(luma>=191){ordinal|=1<<bit;continue;}
        return unavailable('Footer contrast does not establish every original bit');
    }
    return Object.freeze({profile:MATRIX_FRAME_ID_PROFILE,status:'decoded',ordinal});
}

/** The established white-pulse exception stays explicit. During a pulse the
 * footer proves ordinal, not independent pixel-source identity. The expected
 * logical source is route association; it is never a fabricated observation. */
export function matrixFrameIdentity(pixels,kind,expected){
    if(!Number.isInteger(expected)||expected<0||expected>3)return unavailable('Declared source association required');
    const decoded=readMatrixFrameOrdinal(pixels);
    if(decoded.status!=='decoded')return decoded;
    if(decoded.ordinal===2047)return unavailable('Preprogramme sentinel is not a programme frame');
    if(decoded.ordinal>=FRAMES)return unavailable('Original programme ordinal out of range');
    const pulse=pulseFrames.has(decoded.ordinal);
    if(kind==='pulse'){
        if(!pulse)return unavailable('Observed white pulse disagrees with original ordinal cadence');
        return Object.freeze({profile:MATRIX_FRAME_ID_PROFILE,status:'identified',ordinal:decoded.ordinal,
            observed_source_id:null,pulse_exception:true,associated_source_id:expected});
    }
    if(!['0','1','2','3'].includes(kind))return unavailable('Original source marker is not identified');
    if(Number(kind)!==expected)return Object.freeze({profile:MATRIX_FRAME_ID_PROFILE,status:'wrong-source',
        ordinal:decoded.ordinal,observed_source_id:Number(kind),pulse_exception:false,associated_source_id:expected});
    if(pulse)return unavailable('Original ordinal requires its actual white cadence pulse');
    return Object.freeze({profile:MATRIX_FRAME_ID_PROFILE,status:'identified',ordinal:decoded.ordinal,
        observed_source_id:Number(kind),pulse_exception:false,associated_source_id:expected});
}
