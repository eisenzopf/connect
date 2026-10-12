/** Primitive wire validators shared by control and task results. */
export declare function object(value: unknown): value is Record<string, unknown>;
export declare function fields(value: unknown, strings?: string[], nullable?: string[]): value is Record<string, unknown>;
export declare function array(value: unknown, test: (v: unknown) => boolean): boolean;
export declare function integer(value: unknown): boolean;
/** Task contracts are closed shapes; optional fields still need explicit checks. */
export declare function shape(value: unknown, strings?: string[], nullable?: string[], extra?: string[]): value is Record<string, unknown>;
