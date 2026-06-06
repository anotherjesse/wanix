// Typed surface for the Wanix guest SDK modules, so editor completion works
// when a guest imports them. Declares both the root-relative bare specifier the
// qjs module loader resolves and the "./lib/wanix/*.js" relative form.

declare module "lib/wanix/fs.js" {
  export function readText(path: string): string;
  export function readBytes(path: string): Uint8Array;
  export function writeText(path: string, text: string): number;
  export function createText(path: string, text: string): number;
  export function list(path: string): string[];
  export function exists(path: string): boolean;
  export function mkdir(path: string, mode?: number): void;
  export function remove(path: string): void;
  export function writeFile(path: string, text: string): void;
  export function loadFile(path: string): string;
}

declare module "lib/wanix/process.js" {
  export const self: {
    id(): string;
    cmd(): string;
    dir(): string;
    fd(n: number): string;
  };
  export function args(): string[];
  export function arg(i: number): string;
  export function programArgs(): string[];
  export function getenv(name: string, fallback?: string): string | undefined;
  export function exit(code: number): never;
  export function print(line: string): void;
  export function write(text: string): void;
  export function eprint(line: string): void;
}

declare module "lib/wanix/task.js" {
  export interface SpawnOptions {
    cmd?: string;
    env?: Record<string, string> | string;
    dir?: string;
    binds?: Array<[string, number]>;
    inheritStdio?: boolean;
  }
  export class Task {
    readonly id: string;
    setCmd(line: string): this;
    setEnv(env: string): this;
    setDir(dir: string): this;
    ctl(verb: string): this;
    bindFd(src: string, n: number): this;
    start(): this;
    wait(): number;
  }
  export function spawn(kind: string, opts?: SpawnOptions): Task;
}

declare module "./lib/wanix/fs.js" {
  export * from "lib/wanix/fs.js";
}
declare module "./lib/wanix/task.js" {
  export * from "lib/wanix/task.js";
}
declare module "./lib/wanix/process.js" {
  export * from "lib/wanix/process.js";
}
