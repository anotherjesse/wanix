import * as os from "qjs:os";
import * as std from "qjs:std";

const root = scriptArgs[1] || "host";
const targetPath = root + "/target.txt";
const linkPath = root + "/link.txt";

std.writeFile(targetPath, "linked data");

const symlinkErr = os.symlink("target.txt", linkPath);
const [target, readlinkErr] = os.readlink(linkPath);
const [linkStat, lstatErr] = os.lstat(linkPath);
const [targetStat, statErr] = os.stat(linkPath);

std.out.puts("symlink: " + symlinkErr + "\n");
std.out.puts("readlink: " + readlinkErr + " " + target + "\n");
std.out.puts(
  "lstat link: " + lstatErr + " " + ((linkStat.mode & os.S_IFMT) === os.S_IFLNK) + "\n",
);
std.out.puts(
  "stat target: " + statErr + " " + ((targetStat.mode & os.S_IFMT) === os.S_IFREG) + "\n",
);
std.out.puts("load: " + std.loadFile(linkPath) + "\n");
std.out.flush();
