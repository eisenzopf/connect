// Read-only fixture observation of original HTTP media requests/responses. No packet or
// control submission, clock correction, decoder input or browser evaluation.
import { MediaTimeline, playoutTime } from "../../examples/parley-on-connect/app/ui/media-clock.mjs";
const paths = new Set(["/api/media/open", "/api/media/send", "/api/media/receive"]);
const labels = new Set(["owner-audio", "organizer-audio", "owner-av", "organizer-av"]);
const uint = n => Number.isSafeInteger(n) && n >= 0 && n <= 0xffffffff;
const id = s => typeof s === "string" && /^[A-Za-z0-9._~-]{1,128}$/.test(s);
function retain(first, tail, value, initial, recent) {
    if (first.length < initial) first.push(value);
    else { tail.push(value); if (tail.length > recent) tail.shift(); }
}

export class AppMediaClockTrace {
    #now; #start; #pending = new Set(); #pages = []; #detach = []; #closed = false;
    constructor(now = () => performance.now()) { this.#now = now; this.#start = now(); }
    attach(page, label, origin) {
        if (this.#closed || !labels.has(label) || this.#pages.length >= 4 || new URL(origin).origin !== origin)
            throw Error("invalid-owned-clock-trace-page");
        const state = { label, leases: new Map(), routes: new Map(), ntpOrigin: undefined,
            observed_responses: 0, skipped_pending: 0, observation_failures: 0, response_errors: 0 };
        this.#pages.push(state);
        const listener = response => {
            let url;
            try { url = new URL(response.url()); } catch { return; }
            if (this.#closed || url.origin !== origin || !paths.has(url.pathname)) return;
            if (response.status() !== 200) { state.response_errors++; return; }
            if (this.#pending.size >= 4) { state.skipped_pending++; return; }
            const elapsed = this.#now() - this.#start;
            const reading = (async () => {
                const request = response.request(), input = request.postDataJSON();
                const bytes = await response.body();
                if (this.#closed) return;
                if (bytes.length > 65536) throw Error("trace-body-bound");
                const output = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
                if (url.pathname === "/api/media/open") {
                    if (!id(output.leaseId) || !Array.isArray(output.negotiation?.streams) || output.negotiation.streams.length > 8)
                        throw Error("trace-open-shape");
                    const lease = this.#lease(state, output.leaseId);
                    for (const stream of output.negotiation.streams) {
                        if (!["audio", "video"].includes(stream.medium)) continue;
                        const route = this.#route(state, lease, stream.id);
                        route.medium = stream.medium; route.rate = stream.medium === "audio" ? 48000 : 90000;
                    }
                    return;
                }
                const lease = this.#lease(state, input.leaseId), route = this.#route(state, lease, input.streamId);
                const direction = url.pathname === "/api/media/send" ? "send" : "receive";
                const packet = direction === "send" ? input.packet : output;
                if (["idle", "gap"].includes(packet?.kind)) return;
                const flow = route[direction], timing = request.timing();
                const duration = Number.isFinite(timing.responseEnd) && timing.responseEnd >= 0 ? timing.responseEnd : null;
                const row = { observed_elapsed_ms: elapsed, request_duration_ms: duration };
                if (packet?.kind === "clock") {
                    const p = packet.report;
                    if (!p || ![p.ssrc, p.rtp, p.packets, p.octets].every(uint) || typeof p.ntp !== "string" ||
                        !/^(0|[1-9][0-9]{0,19})$/.test(p.ntp) || BigInt(p.ntp) >= 2n ** 64n) throw Error("trace-clock-shape");
                    state.ntpOrigin ??= BigInt(p.ntp);
                    Object.assign(row, { source_slot: this.#source(flow, p.ssrc), rtp: p.rtp,
                        ntp_delta_ms: Number(BigInt.asIntN(64, BigInt(p.ntp) - state.ntpOrigin) * 1000000n / (1n << 32n)) / 1000,
                        packets: p.packets, octets: p.octets, usable: BigInt(p.ntp) !== 0n && p.packets > 0 });
                    flow.clocks_seen++; retain(flow.first_clocks, flow.last_clocks, row, 8, 16);
                } else if (packet?.kind === "rtp") {
                    if (typeof packet.base64 !== "string" || packet.base64.length > 1576) throw Error("trace-rtp-bound");
                    const b = Buffer.from(packet.base64, "base64");
                    if (b.length < 12 || b.length > 1180 || b[0] >>> 6 !== 2) throw Error("trace-rtp-shape");
                    Object.assign(row, { source_slot: this.#source(flow, b.readUInt32BE(8)),
                        rtp: b.readUInt32BE(4), sequence: b.readUInt16BE(2), payload_type: b[1] & 127 });
                    flow.rtp_seen++; retain(flow.first_rtp, flow.last_rtp, row, 16, 32);
                } else throw Error("trace-packet-kind");
                state.observed_responses++;
            })().catch(() => { if (!this.#closed) state.observation_failures++; })
                .finally(() => this.#pending.delete(reading));
            this.#pending.add(reading);
        };
        page.on("response", listener); this.#detach.push(() => page.off("response", listener));
    }
    #lease(state, name) {
        if (!id(name)) throw Error("trace-lease-id");
        if (!state.leases.has(name)) {
            if (state.leases.size >= 16) throw Error("trace-lease-bound");
            state.leases.set(name, state.leases.size);
        }
        return { id: name, slot: state.leases.get(name) };
    }
    #route(state, lease, name) {
        if (!id(name)) throw Error("trace-stream-id");
        const key = lease.id + ":" + name;
        if (!state.routes.has(key)) {
            if (state.routes.size >= 16) throw Error("trace-route-bound");
            const flow = () => ({ sources: new Map(), rtp_seen: 0, clocks_seen: 0,
                first_rtp: [], last_rtp: [], first_clocks: [], last_clocks: [] });
            state.routes.set(key, { route_slot: state.routes.size, lease_slot: lease.slot,
                medium: "unknown", rate: null, send: flow(), receive: flow() });
        }
        return state.routes.get(key);
    }
    #source(flow, ssrc) {
        if (!flow.sources.has(ssrc)) {
            if (flow.sources.size >= 8) throw Error("trace-source-bound");
            flow.sources.set(ssrc, flow.sources.size);
        }
        return flow.sources.get(ssrc);
    }
    async finish() {
        for (const detach of this.#detach.splice(0)) detach();
        let timer;
        try { await Promise.race([Promise.allSettled([...this.#pending]), new Promise(done => { timer = setTimeout(done, 2000); })]); }
        finally { clearTimeout(timer); this.#closed = true; }
        const pages = this.#pages.map(s => ({ label: s.label, observed_responses: s.observed_responses,
            skipped_pending: s.skipped_pending, observation_failures: s.observation_failures, response_errors: s.response_errors,
            routes: [...s.routes.values()].map(route => {
                const flow = value => ({ sources_seen: value.sources.size, rtp_seen: value.rtp_seen, clocks_seen: value.clocks_seen,
                    rtp: [...value.first_rtp, ...value.last_rtp], clocks: [...value.first_clocks, ...value.last_clocks] });
                return { route_slot: route.route_slot, lease_slot: route.lease_slot, medium: route.medium, rate: route.rate,
                    send: flow(route.send), receive: flow(route.receive) };
            }) }));
        const result = { format: "connect/app-media-clock-trace/1", original_requests_and_responses_only: true,
            pending_reads_at_finish: this.#pending.size, pages,
            scope: "Bounded original HTTP request/response observations from the fixture process. NTP deltas share the first observed page clock; stream/lease/SSRC values are replaced by local slots. No payloads, CNAMEs, identifiers, clock correction, decoder completion or browser presentation measurement. No media qualification." };
        result.observer_clock_replay = replayMediaClockTrace(result);
        return result;
    }
}

function distribution(values) {
    const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
    const at = p => sorted.length ? Math.round(sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * p) - 1)] * 1000) / 1000 : null;
    return { samples: sorted.length, minimum_ms: sorted[0] ?? null, p50_ms: at(.5), p95_ms: at(.95), maximum_ms: sorted.at(-1) ?? null };
}

// Replay the same application mapper on the fixture's original response times.
// This locates delay before browser decoding; it does not observe presentation.
export function replayMediaClockTrace(trace) {
    const pages = [];
    for (const page of trace.pages) {
        let now = 0;
        const leases = new Map(), summaries = new Map(), events = [];
        for (const route of page.routes) {
            if (![48000, 90000].includes(route.rate)) continue;
            if (!leases.has(route.lease_slot)) leases.set(route.lease_slot, new MediaTimeline(() => Math.round(now * 1000)));
            leases.get(route.lease_slot).register(String(route.route_slot), route.rate);
            summaries.set(route.route_slot, { route_slot: route.route_slot, lease_slot: route.lease_slot, medium: route.medium,
                synchronized_retained_packets: 0, unsynchronized_retained_packets: 0, ages: [], offsets: [],
                send_http: route.send.rtp.map(r => r.request_duration_ms).filter(n => n !== null),
                receive_http: route.receive.rtp.map(r => r.request_duration_ms).filter(n => n !== null) });
            for (const row of route.receive.clocks) if (row.usable)
                events.push({ route, row, clock: true });
            for (const row of route.receive.rtp) events.push({ route, row, clock: false });
        }
        events.sort((a, b) => a.row.observed_elapsed_ms - b.row.observed_elapsed_ms || Number(b.clock) - Number(a.clock));
        for (const { route, row, clock } of events) {
            now = row.observed_elapsed_ms;
            const timeline = leases.get(route.lease_slot), id = String(route.route_slot), summary = summaries.get(route.route_slot);
            if (clock) {
                const ntp = BigInt.asUintN(64, (1n << 63n) + (BigInt(Math.round(row.ntp_delta_ms * 1000)) << 32n) / 1000000n);
                timeline.report(id, { ssrc: row.source_slot, rtp: row.rtp, ntp: ntp.toString(),
                    packets: row.packets, octets: row.octets, cname: "trace" });
            } else {
                const timing = timeline.timestamp(id, row.source_slot, row.rtp);
                if (timing.synchronized) {
                    summary.synchronized_retained_packets++; summary.ages.push(now - timing.timestampUs / 1000);
                    summary.offsets.push(playoutTime(timing.timestampUs, Math.round(now * 1000), 0) * 1000);
                } else summary.unsynchronized_retained_packets++;
            }
        }
        pages.push({ label: page.label, routes: [...summaries.values()].map(({ ages, offsets, send_http, receive_http, ...summary }) => ({
            ...summary, received_source_age: distribution(ages),
            estimated_playout_offset: distribution(offsets),
            send_http_duration: distribution(send_http), receive_http_duration: distribution(receive_http) })) });
    }
    return { pages, scope: "Unchanged MediaTimeline replay at retained original response-event times, with the unchanged80ms playout allowance. Retained samples only; response-event clocks differ from actual browser decode/presentation. No runtime clock or packet is changed." };
}
