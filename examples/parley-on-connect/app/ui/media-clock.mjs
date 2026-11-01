// Application playout timing over the public SDK's typed RTP/SR observations.
const fail = () => { throw Object.assign(Error("Invalid media clock"), { code: "app-media-clock" }); };
const uint = n => Number.isInteger(n) && n >= 0 && n <= 0xffffffff;
// MediaTimeline already maps RTP/NTP into this page's monotonic clock.
// Convert that clock into current WebAudio time; the first decoded frame must
// not introduce a second origin that survives an initial SR correction.
export function playoutTime(timestampUs, nowUs, audioNow) {
    if (!Number.isSafeInteger(timestampUs) || !Number.isSafeInteger(nowUs) ||
        !Number.isFinite(audioNow) || audioNow < 0) fail();
    return audioNow + (timestampUs - nowUs) / 1000000 + 0.08;
}
export function rtpTimestamp(timestampUs, rate) {
    if (!Number.isSafeInteger(timestampUs) || timestampUs < 0 || ![48000, 90000].includes(rate)) fail();
    return Number(BigInt.asUintN(32, (BigInt(timestampUs) * BigInt(rate) + 500000n) / 1000000n));
}
export function senderReport(timestampUs, rate, stats, cname, unixOriginUs = Math.round(performance.timeOrigin * 1000)) {
    const rtp = rtpTimestamp(timestampUs, rate);
    if (!Number.isSafeInteger(unixOriginUs) || unixOriginUs < 0 ||
        ![stats.ssrc, stats.packets, stats.octets].every(uint) || typeof cname !== "string" || !cname || cname.length > 128) fail();
    const unix = BigInt(unixOriginUs) + BigInt(timestampUs);
    const ntp = BigInt.asUintN(64, ((unix / 1000000n + 2208988800n) << 32n) + ((unix % 1000000n) << 32n) / 1000000n);
    return { ...stats, rtp, ntp: ntp.toString(), cname };
}
export class MediaTimeline {
    #tracks = new Map(); #base; #now;
    constructor(nowUs = () => Math.round(performance.now() * 1000)) { this.#now = nowUs; }
    register(id, rate) {
        if (this.#tracks.has(id) || this.#tracks.size >= 8 || ![48000, 90000].includes(rate)) fail();
        this.#tracks.set(id, { rate });
    }
    report(id, report) {
        const t = this.#tracks.get(id);
        if (!t || ![report.ssrc, report.rtp, report.packets, report.octets].every(uint) ||
            typeof report.ntp !== "string" || !/^(0|[1-9][0-9]{0,19})$/.test(report.ntp) ||
            BigInt(report.ntp) >= 2n ** 64n || typeof report.cname !== "string" || report.cname.length > 255) fail();
        const ntp = BigInt(report.ntp), now = this.#now();
        if (ntp === 0n || !report.packets) return false;
        if (t.report?.ssrc === report.ssrc && BigInt.asIntN(64, ntp - t.report.ntp) <= 0n) return false;
        t.report = { ...report, ntp, seen: now };
        this.#base ??= { ntp, local: now };
        return true;
    }
    timestamp(id, ssrc, timestamp) {
        const t = this.#tracks.get(id);
        if (!t || !uint(ssrc) || !uint(timestamp)) fail();
        const now = this.#now(), changed = t.ssrc !== undefined && t.ssrc !== ssrc;
        if (t.ssrc !== ssrc) {
            t.ssrc = ssrc; t.first = t.newest = timestamp; t.local = now;
            if (t.report?.ssrc !== ssrc) t.report = undefined;
        }
        const extended = t.newest + ((timestamp - (t.newest >>> 0)) | 0);
        t.newest = Math.max(t.newest, extended);
        const fallback = t.local + Math.round((extended - t.first) * 1000000 / t.rate);
        if (t.report?.ssrc === ssrc && now - t.report.seen <= 3000000) {
            const sample = BigInt.asUintN(64, t.report.ntp + (BigInt((timestamp - t.report.rtp) | 0) << 32n) / BigInt(t.rate));
            const mapped = this.#base.local + Number(BigInt.asIntN(64, sample - this.#base.ntp) * 1000000n / (1n << 32n));
            if (Number.isSafeInteger(mapped) && Math.abs(mapped - now) <= 10000000)
                return { timestampUs: mapped, synchronized: true, changed };
        }
        return { timestampUs: fallback, synchronized: false, changed };
    }
    clear() { this.#tracks.clear(); this.#base = undefined; }
}
