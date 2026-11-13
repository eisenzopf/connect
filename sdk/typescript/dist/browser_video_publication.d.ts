export interface VideoPublication {
    active: boolean;
    epoch: 0 | 1;
    needed: boolean;
    boundaryMs: number;
}
export interface CaptureRegistration {
    readonly publication?: VideoPublication;
    readonly epoch?: 0 | 1;
    readonly registeredMs: number;
}
export interface VideoPublicationInput {
    readonly publication: VideoPublication;
    readonly epoch: 0 | 1;
    readonly timestampUs: number;
    readonly basis: "explicit-sdk-submission" | "owned-capture-clock" | "owned-new-callback-presentation-only";
    readonly captureTimeMs?: number;
    accepted: boolean;
    output: boolean;
}
export declare function videoPublication(): VideoPublication;
export declare function completeVideoPublication(value: VideoPublication, at?: number): void;
export declare function closeVideoPublication(value: VideoPublication | undefined): void;
export declare function publicationLookup(peer: object, lookup: (id: string, stream: string) => VideoPublication | undefined): void;
export declare function publicationConnection(connection: object, peer: object, id: string): void;
export declare function capturePublicationRegistration(connection: object, stream: string): CaptureRegistration;
export declare function ownPublicationFrame(frame: object, registration: CaptureRegistration, captureTimeMs?: number, presentationTimeMs?: number): void;
export declare function publicationInput(publication: VideoPublication | undefined, frame: VideoFrame): VideoPublicationInput | false | undefined;
export declare function originalPublicationInput(frame: object): VideoPublicationInput | undefined;
export declare function acceptPublicationInput(input: VideoPublicationInput | undefined): void;
export declare function encodedPublication(bytes: Uint8Array, input: VideoPublicationInput | undefined): void;
export declare function takeEncodedPublication(bytes: Uint8Array): VideoPublicationInput | undefined;
export declare function currentPublicationOutput(publication: VideoPublication, input: VideoPublicationInput | undefined, timestampUs: number, key: boolean): boolean;
export declare function publicationPackets(input: VideoPublicationInput, key: boolean, bytes: Uint8Array[], createdMs: number): void;
export declare function clonePublicationPacket(original: Uint8Array, clone: Uint8Array): void;
export declare function currentPublicationPacket(bytes: Uint8Array, expected?: VideoPublication): boolean;
export declare function settlePublicationPacket(bytes: Uint8Array, sent: boolean, at?: number): void;
