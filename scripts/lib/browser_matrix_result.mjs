// Observe one original in-page programme with short automation requests. The
// media, operation and teardown deadlines remain inside that original owner.
import {setTimeout as delay} from 'node:timers/promises';

export async function originalBrowserMatrixResult(page,{deadline,now=()=>performance.now(),wait=delay}){
    if(!Number.isFinite(deadline)||deadline<=now())throw Error('Original browser programme deadline required');
    let phase;
    for(;;){
        if(now()>=deadline){
            const error=Error('Original browser programme host deadline');
            error.name='OriginalProgrammeDeadline';error.matrixPhase=phase;throw error;
        }
        // This function is deliberately synchronous: WebDriver BiDi must not
        // await the 62-second workload in one script.callFunction request.
        const result=await page.evaluate(()=>{
            const original=globalThis.connectMatrixOriginalProgramme;
            if(!original)return {state:'missing'};
            return {state:original.state,phase:globalThis.matrixPhase??null,
                ...(original.state==='fulfilled'?{value:original.value}:{}),
                ...(original.state==='rejected'?{error:original.error}:{})};
        });
        phase=result?.phase;
        if(result?.state==='fulfilled')return result.value;
        if(result?.state==='rejected'){
            const error=Error(result.error?.message??'Original browser programme failed');
            error.name=result.error?.name??'Error';
            if(typeof result.error?.stack==='string')error.stack=result.error.stack;
            error.matrixPhase=phase;throw error;
        }
        if(result?.state!=='pending')throw Error('Original browser programme owner missing');
        await wait(Math.min(250,Math.max(0,deadline-now())));
    }
}
