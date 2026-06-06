// Byte<->string helpers shared by the Wanix guest SDK modules.
// Mirrors the Latin-1 charCode encoding the qjs examples use (the quickjs
// fixture has no TextEncoder/TextDecoder).

export function bytesFromString(text) {
  return new Uint8Array(Array.from(text).map((char) => char.charCodeAt(0)));
}

export function stringFromBytes(bytes, count) {
  return Array.from(bytes.slice(0, count))
    .map((byte) => String.fromCharCode(byte))
    .join("");
}
