import { identifier } from "./uctp_wire.js";
export function replacementVideo(value, bad) {
    const closed = (v, keys) => !!v &&
        typeof v === "object" &&
        !Array.isArray(v) &&
        Object.keys(v).length === keys.length &&
        Object.keys(v).every((k) => keys.includes(k));
    if (!closed(value, ["publications", "receivers"]) ||
        !Array.isArray(value.publications) ||
        !Array.isArray(value.receivers) ||
        value.publications.length + value.receivers.length < 1 ||
        value.publications.length + value.receivers.length > 32)
        return bad();
    const targets = new Set();
    const group = (items) => {
        const previous = new Set();
        return items.map((v) => {
            if (!closed(v, ["previous", "replacement"]) ||
                !identifier(v.previous) ||
                !identifier(v.replacement) ||
                previous.has(v.previous) ||
                targets.has(v.replacement))
                return bad();
            previous.add(v.previous);
            targets.add(v.replacement);
            return { previous: v.previous, replacement: v.replacement };
        });
    };
    return {
        publications: group(value.publications),
        receivers: group(value.receivers),
    };
}
