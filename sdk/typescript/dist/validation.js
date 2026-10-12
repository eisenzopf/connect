/** Primitive wire validators shared by control and task results. */
export function object(value) {
    return typeof value === "object" && value !== null && !Array.isArray(value);
}
export function fields(value, strings = [], nullable = []) {
    return (object(value) &&
        strings.every((k) => typeof value[k] === "string") &&
        nullable.every((k) => k in value &&
            (value[k] === null || typeof value[k] === "string")));
}
export function array(value, test) {
    return Array.isArray(value) && value.every(test);
}
export function integer(value) {
    return Number.isSafeInteger(value) && Number(value) >= 0;
}
/** Task contracts are closed shapes; optional fields still need explicit checks. */
export function shape(value, strings = [], nullable = [], extra = []) {
    const allowed = new Set([...strings, ...nullable, ...extra]);
    return (fields(value, strings, nullable) &&
        Object.keys(value).every((k) => allowed.has(k)));
}
