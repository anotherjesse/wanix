import * as std from "qjs:std";
import * as os from "qjs:os";

function stringFromBytes(bytes, count) {
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

function bytesFromString(text) {
  return new Uint8Array(Array.from(text).map((char) => char.charCodeAt(0)));
}

function readServiceText(path) {
  const fd = os.open(path, os.O_RDONLY);
  if (fd < 0) {
    throw new Error("open " + path + ": " + fd);
  }
  const bytes = new Uint8Array(64);
  const count = os.read(fd, bytes.buffer, 0, bytes.length);
  os.close(fd);
  if (count < 0) {
    throw new Error("read " + path + ": " + count);
  }
  return stringFromBytes(bytes, count);
}

function writeServiceText(path, text) {
  const fd = os.open(path, os.O_WRONLY);
  if (fd < 0) {
    throw new Error("open " + path + ": " + fd);
  }
  const bytes = bytesFromString(text);
  const count = os.write(fd, bytes.buffer, 0, bytes.length);
  os.close(fd);
  if (count !== bytes.length) {
    throw new Error("short write " + path + ": " + count + "/" + bytes.length);
  }
}

const parent = readServiceText("#task/self/id").trim();
const child = readServiceText("#task/new/qjs").trim();
writeServiceText("#task/" + child + "/cmd", "qjs-task-spawn-child.js alpha beta\n");
writeServiceText("#task/" + child + "/env", "MODE=spawned\n");
writeServiceText("#task/" + child + "/dir", ".\n");
writeServiceText("#task/" + child + "/ctl", "bind #task/" + parent + "/fd/1 fd/1\n");
writeServiceText("#task/" + child + "/ctl", "bind #task/" + parent + "/fd/2 fd/2\n");
print("parent task: " + parent);
print("child task: " + child);
writeServiceText("#task/" + child + "/ctl", "start\n");
print("child exit: " + readServiceText("#task/" + child + "/exit").trim());
