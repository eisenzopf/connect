/** Online decoded-envelope observations for the fixed 100ms AV pulse programme.
 * This validates the observer; it never declares transport or AV timing passed.
 */
export class AudioEnvelope {
    constructor() {
        // Low generated RMS is .05/sqrt(2), high RMS is .7/sqrt(2).
        // Preserve the original rising threshold; rearm below its hysteresis band.
        this.high = .055;
        this.low = .045;
        // Minimum source interval500ms minus pulse100ms leaves400ms quiet.
        // Half that quiet interval proves rearm. Decoded PLC can prolong high
        // amplitude; duration alone does not invalidate the established onset.
        this.rearm = .2;
        this.envelopeDiagnostic = .2;
        this.oversized = [];
        this.clusterOversized = false;
        this.maxGap = .05;
        this.last = null;
        this.lowSince = null;
        this.clusterStart = null;
        this.armed = false;
        this.region = null;
        this.edges = [];
        this.transitions = [];
        this.invalid = [];
        this.overflow = false;
        this.invalidInput = false;
        this.samples = 0;
        this.maximumGap = 0;
    }
    fail(at, reason) {
        if (this.invalid.length < 64) this.invalid.push({at, reason});
        else this.overflow = true;
    }
    sample(at, rms) {
        if (!Number.isFinite(at) || !Number.isFinite(rms) || rms < 0 ||
            (this.last !== null && at <= this.last)) {
            this.invalidInput = true;
            this.fail(this.last ?? 0, "invalid or unordered decoded RMS sample");
            return false;
        }
        this.samples++;
        const gap = this.last === null ? 0 : at - this.last;
        this.maximumGap = Math.max(this.maximumGap, gap);
        const missing = gap > this.maxGap + 1e-9;
        const invalidBefore = this.invalid.length;
        const oversizedBefore = this.oversized.length;
        if (missing) {
            if (this.clusterStart !== null)
                this.fail(at, "unobserved interval inside an active pulse envelope");
            this.clusterStart = null;
            this.armed = false;
            this.lowSince = null;
        }
        this.last = at;
        const region = rms > this.high ? "high" : rms <= this.low ? "low" : "band";
        let accepted = false;
        let reason = null;
        if (region === "low") {
            if (this.lowSince === null) this.lowSince = at;
            if (at - this.lowSince + 1e-9 >= this.rearm) {
                this.armed = true;
                this.clusterStart = null;
            }
        } else {
            this.lowSince = null;
            if (region === "high") {
                if (this.region !== "high") {
                    accepted = this.armed;
                    reason = accepted ? "qualified rising edge" : "continuous low not proven";
                    if (accepted) {
                        if (this.edges.length < 512) this.edges.push(at);
                        else this.overflow = true;
                        this.clusterStart = at;
                        this.clusterOversized = false;
                    }
                }
                if (this.clusterStart !== null && at - this.clusterStart > this.envelopeDiagnostic + 1e-9 &&
                    !this.clusterOversized) {
                    this.clusterOversized = true;
                    if (this.oversized.length < 64)
                        this.oversized.push({at, onset: this.clusterStart, rms,
                            reason: "extended decoded envelope; first onset retained, rebounds suppressed until rearm"});
                    else this.overflow = true;
                }
                this.armed = false;
            }
        }
        if (region !== this.region || missing || this.invalid.length !== invalidBefore || this.oversized.length !== oversizedBefore) {
            if (this.transitions.length < 2048)
                this.transitions.push({at, rms, from: this.region, to: region, accepted,
                    reason, gap_seconds: gap, cluster_start_seconds: this.clusterStart,
                    extended_envelope: this.oversized.length !== oversizedBefore,
                    observer_invalid: this.invalid.length !== invalidBefore});
            else this.overflow = true;
        }
        this.region = region;
        return accepted && !this.overflow && !this.invalidInput;
    }
    report(since = 0) {
        // Warmup establishes the initial envelope state. An ambiguity that
        // begins before the measured window may continue into it, so retain it.
        const invalid = [...this.invalid];
        return {
            status: !this.overflow && !this.invalidInput && !invalid.length ? "observer-valid" : "observer-invalid",
            profile: "connect-decoded-audio-envelope/2",
            scope: "Decoded RMS edge validity only; no transport success or AV skew assertion",
            high_rms_above: this.high, low_rms_at_most: this.low,
            continuous_low_rearm_seconds: this.rearm,
            extended_envelope_diagnostic_seconds: this.envelopeDiagnostic,
            extended_envelopes: this.oversized, extended_envelope_bound: 64,
            maximum_observed_interval_seconds: this.maxGap,
            fixed_source_pulse_seconds: .1, minimum_source_interval_seconds: .5,
            samples: this.samples, maximum_sample_gap_seconds: this.maximumGap,
            observation_start_seconds: since, invalid, overflow: this.overflow,
            invalid_input: this.invalidInput,
            accepted_edge_seconds: this.edges.filter(at => at >= since),
            raw_threshold_transitions: this.transitions,
            transition_bound: 2048, edge_bound: 512,
        };
    }
}

/** Preserve calibration diagnostics but never let valid skew override an invalid observer. */
export function combineAudioEnvelopeSync(sync, envelope) {
    const result = {...sync, audio_envelope: envelope};
    if (envelope?.status !== "observer-valid") {
        result.status = "failed";
        result.observer_failure = "Decoded audio envelope is invalid or ambiguous; calibration cannot qualify timing";
    }
    return result;
}

/** The browser verdict consumes the combined status as well as the frozen numeric limits. */
export function audioVideoSyncPassed(sync, seconds) {
    return sync?.status === "passed" && sync.audio_envelope?.status === "observer-valid" &&
        Number.isInteger(sync.paired) && sync.paired >= seconds * .7 &&
        Number.isFinite(sync.median_abs_skew_ms) && sync.median_abs_skew_ms >= 0 && sync.median_abs_skew_ms <= 80 &&
        Number.isFinite(sync.p95_abs_skew_ms) && sync.p95_abs_skew_ms >= 0 && sync.p95_abs_skew_ms <= 120;
}
