print("before allocation");

globalThis.tooBig = new ArrayBuffer(16 * 1024 * 1024);

print("after allocation");
