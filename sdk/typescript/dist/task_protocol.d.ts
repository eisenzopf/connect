type ControlValidator = (kind: unknown, data: unknown, depth: number) => boolean;
/** Validate nested results without confusing pending/uncertain with completion. */
export declare function validTaskOutcome(value: unknown, validate: ControlValidator, depth: number): boolean;
export {};
