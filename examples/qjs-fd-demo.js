import * as std from "qjs:std";
import * as os from "qjs:os";

function stringFromBytes(bytes, count) {
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

function bytesFromString(text) {
  return new Uint8Array(Array.from(text).map((char) => char.charCodeAt(0)));
}

const input = os.open("main.js", os.O_RDONLY);
const inputBytes = new Uint8Array(1024);
const inputCount = os.read(input, inputBytes.buffer, 0, inputBytes.length);
os.close(input);
std.out.puts("read fd: " + input + "\n");
std.out.puts("saw fd API: " + stringFromBytes(inputBytes, inputCount).includes("os.open") + "\n");

const output = os.open("fd-output.txt", os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o666);
const outputBytes = bytesFromString("hello from a Wanix fd");
const outputCount = os.write(output, outputBytes.buffer, 0, outputBytes.length);
os.close(output);
std.out.puts("write fd: " + output + "\n");
std.out.puts("bytes: " + outputCount + "\n");
std.out.puts(std.loadFile("fd-output.txt") + "\n");
std.out.flush();
