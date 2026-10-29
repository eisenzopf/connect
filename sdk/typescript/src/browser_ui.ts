import {
    BrowserSdkError,
    type BrowserSdkRequestOptions,
    type BrowserConnection,
    type BrowserCaptureHandle,
    type BrowserOperation,
    type BrowserCompletion,
} from "./browser_sdk.js";
import { BrowserDeadline } from "./browser_deadline.js";
import { browserCaptureSupport } from "./browser_media_capture.js";
import { ConnectError } from "./errors.js";

function budget(options: BrowserSdkRequestOptions): BrowserDeadline {
    if (
        !options ||
        !Number.isFinite(options.timeoutMs) ||
        options.timeoutMs < 1 ||
        options.timeoutMs > 60000
    )
        throw new BrowserSdkError("validation", "read-only");
    return new BrowserDeadline(options.timeoutMs);
}
function localError(error: unknown): BrowserSdkError {
    if (error instanceof BrowserSdkError) return error;
    return new BrowserSdkError(
        error instanceof ConnectError && error.code === "cancelled"
            ? "cancelled"
            : error instanceof ConnectError && error.code === "timeout"
              ? "deadline"
              : "unsupported",
        "read-only",
    );
}
export interface BrowserInputDevice {
    readonly id: string;
    readonly label: string;
}
/** Enumerates local input choices without prompting for capture permission. */
export async function browserInputDevices(
    options: BrowserSdkRequestOptions,
): Promise<{
    readonly microphones: readonly BrowserInputDevice[];
    readonly cameras: readonly BrowserInputDevice[];
}> {
    const deadline = budget(options);
    if (
        typeof navigator === "undefined" ||
        typeof navigator.mediaDevices?.enumerateDevices !== "function"
    )
        throw new BrowserSdkError("unsupported", "read-only");
    try {
        const devices = await deadline.run(
            () => navigator.mediaDevices.enumerateDevices(),
            options.signal,
        );
        if (devices.length > 128)
            throw new BrowserSdkError("unsupported", "read-only");
        const inputs = (kind: MediaDeviceKind) =>
            Object.freeze(
                devices
                    .filter((v) => v.kind === kind)
                    .map((v, i) => {
                        if (v.deviceId.length > 4096 || v.label.length > 1024)
                            throw new BrowserSdkError(
                                "unsupported",
                                "read-only",
                            );
                        return Object.freeze({
                            id: v.deviceId,
                            label:
                                v.label ||
                                `${kind === "audioinput" ? "Microphone" : "Camera"} ${i + 1}`,
                        });
                    }),
            );
        return Object.freeze({
            microphones: inputs("audioinput"),
            cameras: inputs("videoinput"),
        });
    } catch (error) {
        throw localError(error);
    }
}
export interface BrowserSourceSelection {
    readonly audio: boolean;
    readonly video?: "camera" | "screen";
    readonly audioDeviceId?: string;
    readonly videoDeviceId?: string;
}
function select(parent: HTMLElement, label: string): HTMLSelectElement {
    const wrapper = document.createElement("label"),
        input = document.createElement("select");
    wrapper.append(document.createTextNode(label), input);
    parent.append(wrapper);
    return input;
}
function option(
    parent: HTMLSelectElement,
    value: string,
    text: string,
    disabled = false,
) {
    const node = document.createElement("option");
    node.value = value;
    node.textContent = text;
    node.disabled = disabled;
    if (!parent.options.length) node.selected = true;
    parent.append(node);
}
/** App-owned source choice. Saving intent and offering the Connection remain explicit. */
export class BrowserSourcePicker {
    readonly element = document.createElement("fieldset");
    private readonly audio: HTMLSelectElement;
    private readonly video: HTMLSelectElement;
    private readonly camera: HTMLSelectElement;
    private readonly events = new AbortController();
    private stopped = false;
    private generation = 0;
    private readonly videoRequired: boolean;
    constructor(allowed: {
        readonly audio: boolean;
        readonly video: readonly ("camera" | "screen")[];
    }) {
        if (
            typeof allowed.audio !== "boolean" ||
            !Array.isArray(allowed.video) ||
            allowed.video.length > 2 ||
            new Set(allowed.video).size !== allowed.video.length ||
            allowed.video.some((v) => v !== "camera" && v !== "screen")
        )
            throw new BrowserSdkError("validation", "read-only");
        this.videoRequired = allowed.video.length > 0;
        const legend = document.createElement("legend");
        legend.textContent = "Media sources";
        this.element.append(legend);
        this.audio = select(this.element, "Microphone");
        this.audio.size = 4;
        option(this.audio, "", "Default microphone");
        this.audio.disabled = !allowed.audio;
        this.video = select(this.element, "Video source");
        const support = browserCaptureSupport();
        if (!allowed.video.length) option(this.video, "", "No video");
        for (const source of allowed.video)
            option(
                this.video,
                source,
                source === "camera" ? "Camera" : "Screen or tab",
                !support[source as "camera" | "screen"],
            );
        this.video.disabled = !allowed.video.length;
        this.camera = select(this.element, "Camera");
        this.camera.size = 4;
        option(this.camera, "", "Default camera");
        this.video.addEventListener("change", () => this.update(), {
            signal: this.events.signal,
        });
        this.update();
    }
    private update() {
        this.camera.disabled = this.video.value !== "camera";
    }
    selection(): BrowserSourceSelection {
        if (this.stopped) throw new BrowserSdkError("cancelled", "read-only");
        const video = this.video.value;
        if (this.videoRequired && !video)
            throw new BrowserSdkError("unsupported", "read-only");
        if (
            video &&
            ((video !== "camera" && video !== "screen") ||
                this.video.selectedOptions[0]?.disabled)
        )
            throw new BrowserSdkError("unsupported", "read-only");
        return Object.freeze({
            audio: !this.audio.disabled,
            ...(video ? { video: video as "camera" | "screen" } : {}),
            ...(!this.audio.disabled && this.audio.value
                ? { audioDeviceId: this.audio.value }
                : {}),
            ...(video === "camera" && this.camera.value
                ? { videoDeviceId: this.camera.value }
                : {}),
        });
    }
    async refresh(options: BrowserSdkRequestOptions): Promise<void> {
        if (this.stopped) throw new BrowserSdkError("cancelled", "read-only");
        const generation = ++this.generation,
            devices = await browserInputDevices(options);
        if (this.stopped || generation !== this.generation) return;
        for (const [input, choices, label] of [
            [this.audio, devices.microphones, "microphone"],
            [this.camera, devices.cameras, "camera"],
        ] as const) {
            const chosen = input.value;
            input.replaceChildren();
            option(input, "", `Default ${label}`);
            for (const device of choices)
                if (device.id) option(input, device.id, device.label);
            if ([...input.options].some((v) => v.value === chosen))
                input.value = chosen;
        }
        this.update();
    }
    close() {
        if (this.stopped) return;
        this.stopped = true;
        this.generation++;
        this.events.abort();
        this.element.disabled = true;
    }
}
export type BrowserUiState = "idle" | "pending" | "ready" | "failed" | "closed";
function status(parent: HTMLElement): HTMLElement {
    const node = document.createElement("p");
    node.setAttribute("role", "status");
    node.setAttribute("aria-live", "polite");
    node.setAttribute("aria-atomic", "true");
    parent.append(node);
    return node;
}
function button(parent: HTMLElement, text: string): HTMLButtonElement {
    const node = document.createElement("button");
    node.type = "button";
    node.textContent = text;
    parent.append(node);
    return node;
}
/** Capture-only UI. A retry requests permission for the same Connection, never a new offer. */
export class BrowserCaptureControls {
    readonly element = document.createElement("section");
    private readonly message = status(this.element);
    private readonly startButton = button(this.element, "Start capture");
    private readonly stopButton = button(this.element, "Stop capture");
    private readonly events = new AbortController();
    private pending?: Promise<BrowserCaptureHandle>;
    private abort?: AbortController;
    private owner?: BrowserCaptureHandle;
    private phase: BrowserUiState = "idle";
    get state(): BrowserUiState {
        return this.phase;
    }
    constructor(
        private readonly connection: BrowserConnection,
        private readonly selection: BrowserSourceSelection,
        private readonly options: BrowserSdkRequestOptions,
    ) {
        budget(options);
        this.options = Object.freeze({ ...options });
        this.selection = Object.freeze({ ...selection });
        this.element.setAttribute("aria-label", "Capture controls");
        this.message.textContent = "Capture is off.";
        this.stopButton.disabled = true;
        this.startButton.addEventListener(
            "click",
            () => {
                void this.start().catch(() => {});
            },
            { signal: this.events.signal },
        );
        this.stopButton.addEventListener("click", () => this.stop(), {
            signal: this.events.signal,
        });
        options.signal?.addEventListener("abort", () => this.close(), {
            once: true,
            signal: this.events.signal,
        });
        if (options.signal?.aborted) this.close();
    }
    start(): Promise<BrowserCaptureHandle> {
        if (this.phase === "closed")
            return Promise.reject(
                new BrowserSdkError("cancelled", "read-only"),
            );
        if (this.pending) return this.pending;
        if (this.owner && !this.owner.closed)
            return Promise.resolve(this.owner);
        const abort = new AbortController();
        this.abort = abort;
        this.phase = "pending";
        const focused = document.activeElement === this.startButton;
        this.element.setAttribute("aria-busy", "true");
        this.startButton.disabled = true;
        this.stopButton.disabled = false;
        this.message.textContent = "Waiting for capture permission.";
        if (focused) this.stopButton.focus();
        const work = Promise.resolve()
            .then(() =>
                this.connection.capture(
                    {
                        ...this.selection,
                        state: (state) => {
                            if (
                                state === "closed" &&
                                this.abort === abort &&
                                this.phase !== "closed"
                            )
                                this.off();
                        },
                    },
                    { timeoutMs: this.options.timeoutMs, signal: abort.signal },
                ),
            )
            .then((owner) => {
                if (
                    abort.signal.aborted ||
                    this.phase === "closed" ||
                    owner.closed
                ) {
                    owner.close();
                    throw new BrowserSdkError("cancelled", "read-only");
                }
                this.owner = owner;
                this.phase = "ready";
                this.message.textContent = "Capture is active.";
                return owner;
            })
            .catch((error) => {
                if (this.abort === abort && this.phase !== "closed") {
                    this.phase = abort.signal.aborted ? "idle" : "failed";
                    this.message.textContent = abort.signal.aborted
                        ? "Capture is off."
                        : error instanceof BrowserSdkError &&
                            error.problem?.category === "capture" &&
                            error.problem.reason === "permission_denied"
                          ? "Capture permission was denied. Allow microphone or camera access, then retry."
                          : "Capture could not start. Check the selected sources, then retry.";
                    const focused = document.activeElement === this.stopButton;
                    this.startButton.textContent = "Retry capture";
                    this.startButton.disabled = false;
                    this.stopButton.disabled = true;
                    if (focused) this.startButton.focus();
                }
                throw error;
            })
            .finally(() => {
                if (this.abort === abort) {
                    this.pending = undefined;
                    this.element.setAttribute("aria-busy", "false");
                }
            });
        this.pending = work;
        return work;
    }
    private off() {
        const focused = document.activeElement === this.stopButton;
        this.phase = "idle";
        this.message.textContent = "Capture is off.";
        this.startButton.disabled = false;
        this.stopButton.disabled = true;
        this.startButton.textContent = "Start capture";
        if (focused) this.startButton.focus();
    }
    stop() {
        this.abort?.abort();
        this.owner?.close();
        this.owner = undefined;
        if (this.phase !== "closed") this.off();
    }
    close() {
        if (this.phase === "closed") return;
        this.phase = "closed";
        this.abort?.abort();
        this.owner?.close();
        this.owner = undefined;
        this.events.abort();
        this.startButton.disabled = true;
        this.stopButton.disabled = true;
        this.element.setAttribute("aria-busy", "false");
        this.message.textContent = "Capture controls closed.";
    }
}
/** Reads one original Operation. Rechecking or stopping a wait never mutates it. */
export class BrowserOperationProgress<T> {
    readonly element = document.createElement("section");
    private readonly message = status(this.element);
    private readonly checkButton = button(this.element, "Check progress");
    private readonly stopButton = button(this.element, "Stop waiting");
    private readonly events = new AbortController();
    private abort?: AbortController;
    private pending?: Promise<BrowserCompletion<T>>;
    private phase: BrowserUiState = "idle";
    get state(): BrowserUiState {
        return this.phase;
    }
    constructor(
        private readonly operation: BrowserOperation<T>,
        private readonly options: BrowserSdkRequestOptions,
    ) {
        budget(options);
        this.options = Object.freeze({ ...options });
        this.element.setAttribute("aria-label", "Operation progress");
        this.message.textContent = "Progress has not been checked.";
        this.stopButton.disabled = true;
        this.checkButton.addEventListener(
            "click",
            () => {
                void this.observe().catch(() => {});
            },
            { signal: this.events.signal },
        );
        this.stopButton.addEventListener("click", () => this.abort?.abort(), {
            signal: this.events.signal,
        });
        options.signal?.addEventListener("abort", () => this.close(), {
            once: true,
            signal: this.events.signal,
        });
        if (options.signal?.aborted) this.close();
    }
    observe(): Promise<BrowserCompletion<T>> {
        if (this.phase === "closed")
            return Promise.reject(
                new BrowserSdkError("cancelled", "read-only"),
            );
        if (this.pending) return this.pending;
        const abort = new AbortController();
        this.abort = abort;
        this.phase = "pending";
        const focused = document.activeElement === this.checkButton;
        this.message.textContent = "Waiting for the original operation.";
        this.element.setAttribute("aria-busy", "true");
        this.checkButton.disabled = true;
        this.stopButton.disabled = false;
        if (focused) this.stopButton.focus();
        const work = Promise.resolve()
            .then(() =>
                this.operation.wait({
                    timeoutMs: this.options.timeoutMs,
                    signal: abort.signal,
                }),
            )
            .then((completion) => {
                if (this.phase === "closed" || abort.signal.aborted)
                    throw new BrowserSdkError("cancelled", "read-only");
                this.phase = "ready";
                this.message.textContent = "Operation completed.";
                return completion;
            })
            .catch((error) => {
                if (this.phase !== "closed") {
                    this.phase = "failed";
                    this.message.textContent = abort.signal.aborted
                        ? "Stopped waiting. Check progress to observe the original operation."
                        : "Could not observe completion. Check the original operation again.";
                }
                throw error;
            })
            .finally(() => {
                this.pending = undefined;
                if (this.phase !== "closed") {
                    const focused = document.activeElement === this.stopButton;
                    this.element.setAttribute("aria-busy", "false");
                    this.checkButton.disabled = false;
                    this.stopButton.disabled = true;
                    if (focused) this.checkButton.focus();
                }
            });
        this.pending = work;
        return work;
    }
    close() {
        if (this.phase === "closed") return;
        this.phase = "closed";
        this.abort?.abort();
        this.events.abort();
        this.checkButton.disabled = true;
        this.stopButton.disabled = true;
        this.element.setAttribute("aria-busy", "false");
        this.message.textContent = "Operation progress closed.";
    }
}
