import assert from 'node:assert/strict';
let current;
/** Installed only by the one actual programme Worker before importing the SDK. */
export function installNativeProgrammeBridge(bridge){assert.equal(current,undefined);current=bridge;}
export function nativeProgrammeBridge(){return current;}
