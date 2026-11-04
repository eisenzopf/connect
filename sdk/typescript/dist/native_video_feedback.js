/** Strict allocation metadata carried by the signed native negotiation owner. */
import { jsonObject } from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
export function captureNativeVideoFeedback(value) {
    const fields = [
        "profile",
        "original_operation",
        "original_offer",
        "stream_generation",
        "connection_generation",
    ];
    if (!jsonObject(value) ||
        Object.keys(value).length !== fields.length ||
        !fields.every((field) => Object.hasOwn(value, field)) ||
        value.profile !== "rtcp-pli-epoch/1" ||
        !identifier(value.original_operation) ||
        !identifier(value.original_offer) ||
        !Number.isSafeInteger(value.stream_generation) ||
        Number(value.stream_generation) < 1 ||
        !Number.isSafeInteger(value.connection_generation) ||
        Number(value.connection_generation) < 1)
        throw new Error("Invalid native video allocation");
    return Object.freeze({
        profile: "rtcp-pli-epoch/1",
        originalOperation: value.original_operation,
        originalOffer: value.original_offer,
        streamGeneration: value.stream_generation,
        connectionGeneration: value.connection_generation,
    });
}
