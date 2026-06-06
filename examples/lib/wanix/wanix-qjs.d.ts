// Ambient TypeScript declarations for the Wanix QuickJS (quickjs-ng) task
// runtime. Grounded in examples/qjs-*.js usage and the wanix-qjs tests; these
// are the type contract the Code OSS / workbench TS service reads when
// checking guest scripts. Only members the runtime actually exposes are
// declared (no Node/DOM).

declare module "qjs:std" {
  export function loadFile(path: string): string;
  export function writeFile(path: string, contents: string): void;
  export function getenv(name: string): string | undefined;
  export function getenviron(): Record<string, string>;
  export function exit(code: number): never;
  export const SEEK_SET: number;
  export const SEEK_CUR: number;
  export const SEEK_END: number;
  export interface FILE {
    puts(text: string): void;
    flush(): void;
    close(): void;
    getByte(): number;
    putByte(byte: number): void;
    readAsString(maxLen?: number): string;
    seek(offset: number, whence: number): number;
  }
  export const out: FILE;
  export const err: FILE;
  export function open(path: string, mode: string): FILE;
  export function fdopen(fd: number, mode: string): FILE;
}

declare module "qjs:os" {
  export const O_RDONLY: number;
  export const O_WRONLY: number;
  export const O_RDWR: number;
  export const O_APPEND: number;
  export const O_CREAT: number;
  export const O_TRUNC: number;
  export const S_IFMT: number;
  export const S_IFDIR: number;
  export const S_IFREG: number;
  export const S_IFLNK: number;
  export const SEEK_SET: number;
  export const SEEK_CUR: number;
  export const SEEK_END: number;
  export function open(path: string, flags: number, mode?: number): number;
  export function read(fd: number, buffer: ArrayBuffer, offset: number, length: number): number;
  export function write(fd: number, buffer: ArrayBuffer, offset: number, length: number): number;
  export function close(fd: number): number;
  export function seek(fd: number, offset: number, whence: number): number;
  export function readdir(path: string): [string[], number];
  export interface Stat {
    mode: number;
    size: number;
    mtime: number;
    atime: number;
    ctime: number;
  }
  export function stat(path: string): [Stat, number];
  export function lstat(path: string): [Stat, number];
  export function readlink(path: string): [string, number];
  export function symlink(target: string, path: string): number;
  export function mkdir(path: string, mode?: number): number;
  export function remove(path: string): number;
  export function rename(oldPath: string, newPath: string): number;
  export function truncate(path: string, length: number): number;
  export function ftruncate(fd: number, length: number): number;
  export function utimes(path: string, atime: number, mtime: number): number;
  export function sleep(deltaMs: number): number;
  export function sleepAsync(deltaMs: number): Promise<void>;
  export function setTimeout(handler: () => void, deltaMs: number): number;
  export function setInterval(handler: () => void, deltaMs: number): number;
  export function clearInterval(handle: number): void;
  export function clearTimeout(handle: number): void;
  export function setReadHandler(fd: number, handler: (() => void) | null): void;
  export function setWriteHandler(fd: number, handler: (() => void) | null): void;
}

declare global {
  const scriptArgs: readonly string[];
  function print(...args: unknown[]): void;
  const console: {
    log(...args: unknown[]): void;
    error(...args: unknown[]): void;
  };
}

export {};
