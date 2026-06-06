// Headline SDK win: spawn(kind, {cmd,env,dir,binds}) -> Task with
// start()/wait()/bindFd(). Encodes the exact #task/new/<kind>, cmd/env/dir
// writes (trailing newline), ctl "bind SRC fd/N" / "start" verbs, and
// #task/<id>/exit read that the qjs examples and wanix-qjs tests prove.

import { readText, writeText } from "./fs.js";
import { self } from "./process.js";

function ctlPathArg(path) {
  return /\s/.test(path) ? "'" + path + "'" : path;
}

class Task {
  constructor(id) {
    this.id = id;
  }
  base() {
    return "#task/" + this.id;
  }
  setCmd(line) {
    writeText(this.base() + "/cmd", line + "\n");
    return this;
  }
  setEnv(line) {
    writeText(this.base() + "/env", line + "\n");
    return this;
  }
  setDir(dir) {
    writeText(this.base() + "/dir", dir + "\n");
    return this;
  }
  ctl(verb) {
    writeText(this.base() + "/ctl", verb + "\n");
    return this;
  }
  bindFd(src, n) {
    return this.ctl("bind " + ctlPathArg(src) + " fd/" + n);
  }
  start() {
    return this.ctl("start");
  }
  wait() {
    return parseInt(readText(this.base() + "/exit").trim(), 10);
  }
}

function envToLines(env) {
  return Object.keys(env)
    .map((key) => key + "=" + env[key])
    .join("\n");
}

// spawn allocates+configures+binds but does NOT auto-start (callers print ids
// first, like the examples). binds: array of [src, fdNumber]. inheritStdio:true
// binds the parent's fd 1->1 and 2->2.
export function spawn(kind, opts) {
  opts = opts || {};
  const id = readText("#task/new/" + kind).trim();
  const task = new Task(id);
  if (opts.cmd !== undefined) task.setCmd(opts.cmd);
  if (opts.env !== undefined) {
    task.setEnv(typeof opts.env === "string" ? opts.env : envToLines(opts.env));
  }
  if (opts.dir !== undefined) task.setDir(opts.dir);
  if (opts.inheritStdio) {
    const parent = self.id();
    task.bindFd("#task/" + parent + "/fd/1", 1);
    task.bindFd("#task/" + parent + "/fd/2", 2);
  }
  const binds = opts.binds || [];
  for (let i = 0; i < binds.length; i++) {
    task.bindFd(binds[i][0], binds[i][1]);
  }
  return task;
}

export { Task };
