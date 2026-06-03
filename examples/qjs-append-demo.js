import * as std from "qjs:std";
import * as os from "qjs:os";

std.writeFile("append-log.txt", "start");

const fd = os.open("append-log.txt", os.O_WRONLY | os.O_APPEND);
const bytes = new Uint8Array([45, 111, 115]);
os.seek(fd, 0, std.SEEK_SET);
std.out.puts("os bytes: " + os.write(fd, bytes.buffer, 0, bytes.length) + "\n");
os.close(fd);

const file = std.open("append-log.txt", "a");
file.puts("-std");
file.close();

const fd2 = os.open("append-log.txt", os.O_WRONLY);
const file2 = std.fdopen(fd2, "a");
file2.puts("-fdopen");
file2.close();

std.out.puts("log: " + std.loadFile("append-log.txt") + "\n");
std.out.flush();
