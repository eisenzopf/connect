/** Independently decoded generated-source identity. No media enters reports. */
export function markerCells(id) {
    if(!Number.isInteger(id)||id<0||id>3)throw Error("Fixture source identity must be 0..3");
    return [id&1?200:40,id&2?200:40,id&1?40:200,id&2?40:200];
}
export function classifySourceFrame(gray) {
    if(gray.length!==64)throw Error("Source classification requires independently decoded 8×8 pixels");
    if(Array.from(gray).every(value=>value>240))return "pulse";
    if(Array.from(gray).every(value=>value<12))return "black";
    const bits=[0,1,8,9].map(index=>gray[index]<=85?0:gray[index]>=155&&gray[index]<=230?1:null);
    if(bits.some(value=>value===null)||bits[0]===bits[2]||bits[1]===bits[3])return "unclassified";
    return String(bits[0]+2*bits[1]);
}
export function sourceIdentityReport(counts,expectedId,allowMutedBlack=false) {
    markerCells(expectedId);
    const expected=counts[String(expectedId)]??0;
    const wrong=[0,1,2,3].filter(id=>id!==expectedId).reduce((sum,id)=>sum+(counts[String(id)]??0),0);
    const unknown=(counts.unclassified??0)+(allowMutedBlack?0:counts.black??0);
    return {status:expected>0&&wrong===0&&unknown===0?"passed":"failed",expected_source_id:expectedId,
        recognized_expected_frames:expected,recognized_wrong_source_frames:wrong,
        unclassified_frames:counts.unclassified??0,full_white_pulse_frames:counts.pulse??0,
        black_frames:counts.black??0,muted_black_permitted:allowMutedBlack,
        recognized_source_counts:Object.fromEntries([0,1,2,3].map(id=>[String(id),counts[String(id)]??0])),
        method:"independent decoded 8×8 cell-centre pixels without spatial blending: four large gray cells encode two source bits plus complements; white AV pulses excluded from identity only"};
}
