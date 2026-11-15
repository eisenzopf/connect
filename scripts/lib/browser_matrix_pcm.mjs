const frequencies=[440,660,880,1100];
const check=(ok,message)=>{if(!ok)throw Error(message);};

// Actual decoded PCM, independent of RTP/SDK/sender counters. No expected-tone
// oscillator or generated samples enter this observer.
export function pcmIdentity(samples,expected) {
    check(samples.length===960&&Number.isInteger(expected)&&expected>=0&&expected<4,'Mono 20ms source identity');
    let energy=0;for(const value of samples){check(Number.isFinite(value),'Finite decoded PCM');energy+=value*value;}
    // Project the original 48kHz block directly. Unfiltered 8:1 sampling
    // folds other decoded frequencies into the four authored source tones.
    const powers=frequencies.map(hz=>{
        const coefficient=2*Math.cos(2*Math.PI*hz/48000);let a=0,b=0;
        for(let i=0;i<samples.length;i++){const value=samples[i]+coefficient*a-b;b=a;a=value;}
        return (a*a+b*b-coefficient*a*b)/960**2;
    });
    const rms=Math.sqrt(energy/samples.length),wrong=Math.max(...powers.filter((_,i)=>i!==expected));
    // Source recognition and usable-volume availability are independent.
    // Keep the original dominance/floor exactly; literal zero and unrecognized
    // PCM cannot identify a source. Quiet recognition earns no usable time.
    const source_identified=powers[expected]>8*Math.max(wrong,1e-12);
    return {rms,powers,source_identified,identified:energy/samples.length>1e-6&&source_identified};
}
