type Validator = (kind: unknown, data: unknown, depth: number) => boolean;
export declare function validEvents(kind: unknown, v: unknown, validate: Validator, depth: number): boolean;
export {};
