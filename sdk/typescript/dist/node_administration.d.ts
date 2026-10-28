import { inspect } from "node:util";
import { type Json } from "./uctp_json.js";
import { type SdkOperation, type SdkMutationOptions, type OperationId } from "./node_sdk.js";
import type { CanonicalRequestOptions } from "./canonical_validation.js";
declare const identity: unique symbol;
export interface SdkTrustedProvisioningCommand {
    readonly value: Json;
    readonly operationId: OperationId;
    readonly [identity]: "validated-trusted-provisioning";
    /** Save this reference and the explicit command before submission. Contains no grant. */
    referenceJson(): Json;
}
/** Explicit trusted backend. No provisioning route is exposed on NodeClient. */
export declare class NodeAdministrativeClient {
    [inspect.custom](): string;
    toJSON(): {
        kind: string;
    };
    private readonly helper;
    private readonly executable;
    private readonly operationOwner;
    readonly operations: Readonly<{
        restore: (saved: Json) => SdkOperation<Json>;
        recover: <T>(reference: import("./node_sdk.js").SdkOperationReference<T>) => SdkOperation<T>;
    }>;
    constructor(config: {
        readonly config: string;
        readonly executable?: string;
    });
    private validate;
    validateProvisioning(value: Json): SdkTrustedProvisioningCommand;
    provision(command: SdkTrustedProvisioningCommand, options: SdkMutationOptions): Promise<SdkOperation<Json>>;
    close(options: CanonicalRequestOptions): Promise<void>;
}
export {};
