/** Tone identity uses received analyser spectrum, independently of expected Hz. */
export function spectralTone(samples, spectrumDb, sampleRate, fftSize) {
    if (!samples.length || spectrumDb.length!==fftSize/2 || !Number.isFinite(sampleRate) || sampleRate<=0
        || !samples.every(Number.isFinite))return {rms:0,frequency_hz:0};
    const mean=samples.reduce((sum,value)=>sum+value,0)/samples.length;
    const rms=Math.sqrt(samples.reduce((sum,value)=>sum+(value-mean)**2,0)/samples.length);
    let peak=-Infinity,second=-Infinity,bin=-1;
    for(let index=1;index<spectrumDb.length;index++) {
        const hz=index*sampleRate/fftSize,value=spectrumDb[index];
        if(hz<100||hz>3000||!Number.isFinite(value))continue;
        if(value>peak){second=peak;peak=value;bin=index;}else if(value>second)second=value;
    }
    return {rms,frequency_hz:bin>=0&&peak>second&&rms>.001?bin*sampleRate/fftSize:0};
}
