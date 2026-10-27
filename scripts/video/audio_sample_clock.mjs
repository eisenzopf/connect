/** One actual sample per100ms deadline; never manufacture catch-up samples. */
export function audioSampleClock() {
    let next;
    return observedAt => {
        if(!Number.isFinite(observedAt)||observedAt<0)return false;
        next??=observedAt;
        if(observedAt<next)return false;
        next+=100;
        if(next<=observedAt)next=observedAt+100;
        return true;
    };
}
