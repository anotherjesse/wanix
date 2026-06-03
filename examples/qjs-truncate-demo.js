import * as os from "qjs:os";
import * as std from "qjs:std";

std.writeFile("resize.txt", "abcdef");

const fd = os.open("resize.txt", os.O_RDWR);
std.out.puts("ftruncate: " + os.ftruncate(fd, 3) + "\n");
os.close(fd);
std.out.puts("small: " + std.loadFile("resize.txt") + "\n");

std.out.puts("truncate: " + os.truncate("resize.txt", 5) + "\n");

const fd2 = os.open("resize.txt", os.O_RDONLY);
const bytes = new Uint8Array(8);
const n = os.read(fd2, bytes.buffer, 0, bytes.length);
os.close(fd2);

std.out.puts("len: " + n + "\n");
std.out.puts("codes: " + Array.from(bytes.slice(0, n)).join(",") + "\n");
std.out.flush();
