/** Resolve only flat built SDK assets used by the owned browser fixture.
 * The caller retains its realpath containment check before reading bytes. */
export function matrixSdkAssetName(pathname) {
    if (typeof pathname !== "string") throw new TypeError("Invalid matrix SDK asset path");
    const match = /^\/sdk\/(?:typescript\/dist\/)?([a-z][a-z0-9_]*\.js)$/.exec(pathname);
    if (!match) throw new TypeError("Invalid matrix SDK asset path");
    return match[1];
}
