import { inspect } from "node:util";
import { spawnSync } from "node:child_process";
import { canonicalJson, parseCanonicalJson, jsonObject, } from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
import { SdkHelper, NodeSdkError, object } from "./sdk_helper.js";
import { createAdministrativeOperations, } from "./node_sdk.js";
const commands = new WeakMap();
function freeze(value) {
    if (value && typeof value === "object") {
        Object.values(value).forEach(freeze);
        Object.freeze(value);
    }
    return value;
}
function copy(value, maximum = 65536) {
    return parseCanonicalJson(Buffer.from(canonicalJson(value, maximum)), maximum);
}
/** Explicit trusted backend. No provisioning route is exposed on NodeClient. */
export class NodeAdministrativeClient {
    [inspect.custom]() {
        return "NodeAdministrativeClient(redacted)";
    }
    toJSON() {
        return { kind: "administrative-client" };
    }
    helper;
    executable;
    operationOwner;
    operations;
    constructor(config) {
        const captured = {
            config: config?.config,
            executable: config?.executable ?? "connect",
        };
        this.helper = new SdkHelper(captured, "administrative");
        this.executable = captured.executable;
        this.operationOwner = createAdministrativeOperations(this.helper, (value) => this.validate(value, true));
        this.operations = Object.freeze({
            restore: this.operationOwner.restore,
            recover: this.operationOwner.recover,
        });
    }
    validate(value, reference) {
        let bytes;
        try {
            bytes = Buffer.from(canonicalJson(value, reference ? 4096 : 65536));
        }
        catch {
            throw new NodeSdkError("validation", "not-submitted");
        }
        const result = spawnSync(this.executable, [
            "sdk-validate-provisioning",
            ...(reference ? ["--reference"] : []),
        ], {
            input: bytes,
            timeout: 2000,
            killSignal: "SIGKILL",
            maxBuffer: 128 * 1024,
            stdio: ["pipe", "pipe", "ignore"],
        });
        if (result.error || result.status !== 0)
            throw new NodeSdkError("configuration", "not-submitted");
        try {
            const answer = parseCanonicalJson(result.stdout, 128 * 1024, 40);
            if (!object(answer, [
                "profile",
                "valid",
                "value",
                "reference",
                "kind",
            ]) ||
                answer.profile !== "connect-sdk-provisioning-validation/1" ||
                typeof answer.valid !== "boolean")
                throw new Error();
            if (!answer.valid) {
                if (Object.keys(answer).length !== 2)
                    throw new Error();
                throw new NodeSdkError("validation", "not-submitted");
            }
            if (!object(answer.reference, [
                "schema",
                "operation_id",
                "command",
                "scope",
            ]) ||
                answer.reference.schema !== "connect-operation-reference/1" ||
                !identifier(answer.reference.operation_id) ||
                typeof answer.reference.command !== "string" ||
                !/^[a-z][a-z0-9.-]{1,127}$/.test(answer.reference.command) ||
                !object(answer.reference.scope, []) ||
                Object.keys(answer.reference.scope).length ||
                typeof answer.kind !== "string" ||
                ![
                    "administrative",
                    "operation_cancel",
                    "operation_reconcile",
                ].includes(answer.kind))
                throw new Error();
            if (!reference &&
                (answer.kind !== "administrative" ||
                    !jsonObject(answer.value) ||
                    answer.value.operation_id !== answer.reference.operation_id))
                throw new Error();
            if ((answer.kind === "operation_cancel" &&
                answer.reference.command !== "operation.cancel") ||
                (answer.kind === "operation_reconcile" &&
                    answer.reference.command !== "operation.reconcile"))
                throw new Error();
            if (answer.kind === "administrative" &&
                ["operation.cancel", "operation.reconcile"].includes(answer.reference.command))
                throw new Error();
            const captured = copy(answer.value, reference ? 4096 : 65536), saved = copy(answer.reference, 4096);
            if (reference && canonicalJson(captured) !== canonicalJson(saved))
                throw new Error();
            return freeze({
                value: captured,
                reference: saved,
                kind: answer.kind,
            });
        }
        catch (error) {
            if (error instanceof NodeSdkError)
                throw error;
            this.helper.fence("protocol");
            throw new NodeSdkError("protocol", "not-submitted");
        }
    }
    validateProvisioning(value) {
        const checked = this.validate(value, false);
        const token = Object.freeze({
            [inspect.custom]: () => "SdkTrustedProvisioningCommand(redacted)",
            toJSON: () => ({ kind: "trusted-provisioning-command" }),
            value: checked.value,
            operationId: checked.reference.operation_id,
            referenceJson: () => copy(checked.reference, 4096),
        });
        commands.set(token, checked);
        return token;
    }
    provision(command, options) {
        const checked = commands.get(command);
        if (!checked || !identifier(options?.operationId))
            throw new NodeSdkError("validation", "not-submitted");
        const key = freeze({
            operationId: options.operationId,
            command: checked.reference.command,
            scope: {},
        });
        if (key.operationId !== checked.reference.operation_id)
            throw new NodeSdkError("validation", "not-submitted", key);
        if (options.extensions !== undefined) {
            if (!jsonObject(options.extensions))
                throw new NodeSdkError("validation", "not-submitted", key);
            if (Object.keys(options.extensions).length)
                throw new NodeSdkError("unsupported", "not-submitted", key);
        }
        return this.helper.request({
            type: "administrative_provision",
            command: checked.value,
            mutation: { operation_id: key.operationId },
        }, options, (value) => this.operationOwner.fromReply(value, key), key, true);
    }
    close(options) {
        return this.helper.close(options);
    }
}
