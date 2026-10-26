export class CanonicalError extends Error {
    kind;
    certainty;
    operation;
    problem;
    constructor(kind, certainty, operation, problem) {
        super(`Connect SDK ${kind} (${certainty})`);
        this.kind = kind;
        this.certainty = certainty;
        this.operation = operation;
        this.problem = problem;
        this.name = "CanonicalError";
    }
}
