/** Real browser capture startup; media/quality measurements remain separate. */
export async function browserMatrixSources() {
    const audio=new AudioContext({sampleRate:48000});
    await audio.resume();
    if(audio.state!=='running')throw Error('Owned real-time audio output did not start');
    const sources=[];
    let stopped=false;
    return {sources,
        async start({route,connection,options}) {
            if(stopped)throw Error('Original media source owner retired');
            const role=Number(route.role.split('-').at(-1))-1;
            const canvas=document.createElement('canvas');canvas.width=640;canvas.height=360;
            const paint=canvas.getContext('2d'),oscillator=audio.createOscillator(),gain=audio.createGain();
            const destination=audio.createMediaStreamDestination();destination.channelCount=1;
            oscillator.frequency.value=440+220*role;gain.gain.value=.05;
            oscillator.connect(gain);gain.connect(destination);oscillator.start();
            const record={index:route.index,sourceId:role,draws:0,oscillator,gain,destination,canvas,stream:undefined,capture:undefined,timer:undefined};
            sources.push(record);
            const draw=()=>{
                paint.fillStyle=`hsl(${record.draws%360} 90% 40%)`;paint.fillRect(0,0,640,360);
                paint.fillStyle='white';paint.fillRect((record.draws*7)%580,70,60,150);
                [role&1?200:40,role&2?200:40,role&1?40:200,role&2?40:200].forEach((value,index)=>{
                    paint.fillStyle=`rgb(${value},${value},${value})`;paint.fillRect(index%2*80,Math.floor(index/2)*45,80,45);
                });record.draws++;
            };
            draw();record.timer=setInterval(draw,1000/30);
            record.stream=new MediaStream([...destination.stream.getAudioTracks(),...canvas.captureStream(30).getVideoTracks()]);
            record.capture=await connection.capture({audio:true,video:'camera',stream:record.stream},options);
        },
        async close() {
            if(stopped)return;stopped=true;
            for(const source of sources){
                clearInterval(source.timer);source.capture?.close();source.stream?.getTracks().forEach(track=>track.stop());
                source.oscillator.stop();source.oscillator.disconnect();source.gain.disconnect();
            }
            await audio.close();
        },
        observation:()=>({real_time_audio_state:audio.state,sources:sources.map(source=>({index:source.index,
            source_id:source.sourceId,draws:source.draws,capture_started:Boolean(source.capture)})),
            scope:'Actual capture startup only. No decoded receive or profile-quality pass.'}),
    };
}
