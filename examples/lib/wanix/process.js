// Current-task introspection + stdio convenience. Wraps #task/self/* service
// files, scriptArgs, and std stdio so guests stop poking them inline. Pure
// functions over qjs:std and service files — no globalThis.Wanix.

import * as std from "qjs:std";
import { readText } from "./fs.js";

export const self = {
  id() {
    return readText("#task/self/id").trim();
  },
  cmd() {
    return readText("#task/self/cmd").trim();
  },
  dir() {
    return readText("#task/self/dir").trim();
  },
  fd(n) {
    return readText("#task/self/fd/" + n);
  },
};

export function args() {
  return scriptArgs.slice();
}

export function arg(i) {
  return scriptArgs[i];
}

export function programArgs() {
  return scriptArgs.slice(1);
}

export function getenv(name, fallback) {
  const value = std.getenv(name);
  return value === undefined ? fallback : value;
}

export function exit(code) {
  std.exit(code);
}

export function print(line) {
  std.out.puts(line + "\n");
  std.out.flush();
}

export function write(text) {
  std.out.puts(text);
  std.out.flush();
}

export function eprint(line) {
  std.err.puts(line + "\n");
  std.err.flush();
}
