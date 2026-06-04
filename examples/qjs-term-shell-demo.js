import * as std from "qjs:std";
import * as os from "qjs:os";

function stringFromBytes(bytes, count) {
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

let pending = "";
let running = true;
let terminalSize = "";
let lastStatus = 0;
const rawInput = std.getenv("WANIX_QJS_SHELL_RAW") === "1";
const termId = std.getenv("WANIX_TERM_ID") || "1";
let cwd = normalizeNamespacePath(std.loadFile("#task/self/dir").trim() || ".");

function prompt() {
  if (running) {
    std.out.puts("$ ");
  }
}

function bytesFromString(text) {
  const bytes = new Uint8Array(text.length);
  for (let i = 0; i < text.length; i++) {
    bytes[i] = text.charCodeAt(i) & 0xff;
  }
  return bytes;
}

function writeText(path, text) {
  const fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o666);
  if (fd < 0) {
    return fd;
  }
  const bytes = bytesFromString(text);
  const count = os.write(fd, bytes.buffer, 0, bytes.length);
  os.close(fd);
  return count;
}

function writeServiceText(path, text) {
  const fd = os.open(path, os.O_WRONLY);
  if (fd < 0) {
    return fd;
  }
  const bytes = bytesFromString(text);
  const count = os.write(fd, bytes.buffer, 0, bytes.length);
  os.close(fd);
  return count;
}

function readServiceText(path) {
  const fd = os.open(path, os.O_RDONLY);
  if (fd < 0) {
    throw new Error("open " + path + ": " + fd);
  }
  const chunks = [];
  let total = 0;
  const bytes = new Uint8Array(4096);
  while (true) {
    const count = os.read(fd, bytes.buffer, 0, bytes.length);
    if (count < 0) {
      os.close(fd);
      throw new Error("read " + path + ": " + count);
    }
    if (count === 0) {
      break;
    }
    chunks.push(bytes.slice(0, count));
    total += count;
    if (count < bytes.length) {
      break;
    }
  }
  os.close(fd);
  const output = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    output.set(chunk, offset);
    offset += chunk.length;
  }
  return stringFromBytes(output, output.length);
}

function writeRequiredServiceText(path, text) {
  const count = writeServiceText(path, text);
  if (count !== text.length) {
    throw new Error("write " + path + ": " + count + "/" + text.length);
  }
}

function writeTerminalInput(text) {
  writeRequiredServiceText("#term/" + termId + "/data", text);
}

function parseWords(line) {
  const words = [];
  let current = "";
  let quote = "";
  let escaping = false;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (escaping) {
      current += ch;
      escaping = false;
      continue;
    }
    if (ch === "\\") {
      escaping = true;
      continue;
    }
    if (quote) {
      if (ch === quote) {
        quote = "";
      } else {
        current += ch;
      }
      continue;
    }
    if (ch === "'" || ch === '"') {
      quote = ch;
      continue;
    }
    if (/\s/.test(ch)) {
      if (current.length > 0) {
        words.push(current);
        current = "";
      }
      continue;
    }
    current += ch;
  }
  if (escaping) {
    current += "\\";
  }
  if (quote) {
    return { error: "unterminated quote" };
  }
  if (current.length > 0) {
    words.push(current);
  }
  return { words };
}

function normalizeNamespacePath(path, base) {
  if (!path || path === ".") {
    return base || ".";
  }
  if (path[0] === "#") {
    return path;
  }
  const parts = [];
  if (path[0] !== "/" && base && base !== ".") {
    for (const part of base.split("/")) {
      if (part) {
        parts.push(part);
      }
    }
  }
  for (const part of path.split("/")) {
    if (!part || part === ".") {
      continue;
    }
    if (part === "..") {
      if (parts.length > 0) {
        parts.pop();
      }
      continue;
    }
    parts.push(part);
  }
  return parts.length > 0 ? parts.join("/") : ".";
}

function resolveShellPath(path) {
  return normalizeNamespacePath(path || ".", cwd);
}

function namespaceDir(path) {
  const parts = path.split("/").filter(Boolean);
  if (parts.length <= 1) {
    return ".";
  }
  return parts.slice(0, -1).join("/");
}

function namespaceBase(path) {
  const parts = path.split("/").filter(Boolean);
  if (parts.length === 0) {
    return ".";
  }
  return parts[parts.length - 1];
}

function quoteCommandWord(word) {
  if (word.length > 0 && !/[\s'"\\]/.test(word)) {
    return word;
  }
  return "'" + word.replace(/'/g, "'\"'\"'") + "'";
}

function parseQjsLaunch(words) {
  const launch = {
    script: words[1],
    args: [],
    stdinPath: null,
    stdoutPath: null,
    stderrPath: null
  };
  for (let i = 2; i < words.length; i++) {
    const word = words[i];
    if (word !== "<" && word !== ">" && word !== "2>") {
      launch.args.push(word);
      continue;
    }
    if (i + 1 >= words.length) {
      return { error: "qjs: missing path after " + word };
    }
    const path = resolveShellPath(words[i + 1]);
    if (word === "<") {
      if (launch.stdinPath) {
        return { error: "qjs: duplicate stdin redirection" };
      }
      launch.stdinPath = path;
    } else if (word === ">") {
      if (launch.stdoutPath) {
        return { error: "qjs: duplicate stdout redirection" };
      }
      launch.stdoutPath = path;
    } else {
      if (launch.stderrPath) {
        return { error: "qjs: duplicate stderr redirection" };
      }
      launch.stderrPath = path;
    }
    i += 1;
  }
  return launch;
}

function commandUsesTerminalStdin(line) {
  const trimmed = line.trim();
  if (trimmed === "") {
    return false;
  }
  const parsed = parseWords(trimmed);
  if (parsed.error || parsed.words[0] !== "qjs" || parsed.words.length < 2) {
    return false;
  }
  const launch = parseQjsLaunch(parsed.words);
  return !launch.error && !launch.stdinPath;
}

function readEnvLines() {
  return readServiceText("#task/self/env")
    .split("\n")
    .filter((line) => line.length > 0);
}

function envKey(line) {
  const equals = line.indexOf("=");
  return equals < 0 ? line : line.slice(0, equals);
}

function validEnvKey(key) {
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(key);
}

function writeEnvLines(lines) {
  writeRequiredServiceText("#task/self/env", lines.join("\n") + (lines.length > 0 ? "\n" : ""));
}

function runEnv(words) {
  if (words.length > 2) {
    std.out.puts("env: usage: env [KEY]\n");
    prompt();
    return;
  }
  const lines = readEnvLines();
  if (words.length === 1) {
    for (const line of lines.slice().sort()) {
      std.out.puts(line + "\n");
    }
    prompt();
    return;
  }
  const key = words[1];
  if (!validEnvKey(key)) {
    std.out.puts("env: " + key + ": invalid key\n");
    prompt();
    return;
  }
  for (const line of lines) {
    if (envKey(line) === key) {
      std.out.puts(line + "\n");
      break;
    }
  }
  prompt();
}

function runSetenv(words) {
  if (words.length < 2) {
    std.out.puts("setenv: usage: setenv KEY [VALUE...]\n");
    prompt();
    return;
  }
  const key = words[1];
  const value = words.slice(2).join(" ");
  if (!validEnvKey(key) || value.indexOf("\n") >= 0) {
    std.out.puts("setenv: " + key + ": invalid KEY or VALUE\n");
    prompt();
    return;
  }
  const lines = readEnvLines().filter((line) => envKey(line) !== key);
  lines.push(key + "=" + value);
  writeEnvLines(lines);
  prompt();
}

function runUnsetenv(words) {
  if (words.length !== 2) {
    std.out.puts("unsetenv: usage: unsetenv KEY\n");
    prompt();
    return;
  }
  const key = words[1];
  if (!validEnvKey(key)) {
    std.out.puts("unsetenv: " + key + ": invalid key\n");
    prompt();
    return;
  }
  writeEnvLines(readEnvLines().filter((line) => envKey(line) !== key));
  prompt();
}

function visibleEntries(path) {
  const [entries, err] = os.readdir(path);
  if (err !== 0) {
    return { err };
  }
  return {
    entries: entries
      .filter((name) => name !== "." && name !== ".." && name !== "__wanix_qjs_shell.js")
      .sort()
  };
}

function runLs(words) {
  const path = resolveShellPath(words[1] || ".");
  const result = visibleEntries(path);
  if (result.err !== undefined) {
    std.out.puts("ls: " + (words[1] || ".") + ": errno " + result.err + "\n");
    prompt();
    return;
  }
  std.out.puts(result.entries.join(" ") + "\n");
  prompt();
}

function runCd(words) {
  const requested = words[1] || ".";
  const path = resolveShellPath(requested);
  if (path[0] === "#") {
    std.out.puts("cd: " + requested + ": service paths are not directories\n");
    prompt();
    return;
  }
  const result = visibleEntries(path);
  if (result.err !== undefined) {
    std.out.puts("cd: " + requested + ": errno " + result.err + "\n");
    prompt();
    return;
  }
  const count = writeServiceText("#task/self/dir", path + "\n");
  if (count < 0) {
    std.out.puts("cd: " + requested + ": failed to update cwd " + count + "\n");
    prompt();
    return;
  }
  cwd = path;
  prompt();
}

function runCat(words) {
  if (words.length < 2) {
    std.out.puts("cat: missing path\n");
    prompt();
    return;
  }
  for (const requested of words.slice(1)) {
    const path = resolveShellPath(requested);
    try {
      const text = std.loadFile(path);
      if (text === null || text === undefined) {
        std.out.puts("cat: " + requested + ": not found\n");
      } else {
        std.out.puts(text);
      }
    } catch (error) {
      std.out.puts("cat: " + requested + ": " + error.message + "\n");
    }
  }
  prompt();
}

function runWrite(words) {
  if (words.length < 3) {
    std.out.puts("write: usage: write PATH TEXT...\n");
    prompt();
    return;
  }
  const path = resolveShellPath(words[1]);
  if (path[0] === "#") {
    std.out.puts("write: " + words[1] + ": service paths are read by command-specific helpers\n");
    prompt();
    return;
  }
  const count = writeText(path, words.slice(2).join(" ") + "\n");
  if (count < 0) {
    std.out.puts("write: " + words[1] + ": errno " + count + "\n");
  } else {
    std.out.puts("wrote " + words[1] + "\n");
  }
  prompt();
}

function runMkdir(words) {
  if (words.length < 2) {
    std.out.puts("mkdir: missing path\n");
    prompt();
    return;
  }
  for (const requested of words.slice(1)) {
    const path = resolveShellPath(requested);
    if (path[0] === "#") {
      std.out.puts("mkdir: " + requested + ": service paths are not namespace directories\n");
      continue;
    }
    const result = os.mkdir(path, 0o777);
    if (result !== 0) {
      std.out.puts("mkdir: " + requested + ": errno " + result + "\n");
    }
  }
  prompt();
}

function runRm(words) {
  if (words.length < 2) {
    std.out.puts("rm: missing path\n");
    prompt();
    return;
  }
  for (const requested of words.slice(1)) {
    const path = resolveShellPath(requested);
    if (path[0] === "#") {
      std.out.puts("rm: " + requested + ": service paths are not removed by shell commands\n");
      continue;
    }
    const result = os.remove(path);
    if (result !== 0) {
      std.out.puts("rm: " + requested + ": errno " + result + "\n");
    }
  }
  prompt();
}

function runRmdir(words) {
  if (words.length < 2) {
    std.out.puts("rmdir: missing path\n");
    prompt();
    return;
  }
  for (const requested of words.slice(1)) {
    const path = resolveShellPath(requested);
    if (path[0] === "#") {
      std.out.puts("rmdir: " + requested + ": service paths are not namespace directories\n");
      continue;
    }
    const result = visibleEntries(path);
    if (result.err !== undefined) {
      std.out.puts("rmdir: " + requested + ": not a directory\n");
      continue;
    }
    if (result.entries.length > 0) {
      std.out.puts("rmdir: " + requested + ": directory not empty\n");
      continue;
    }
    const removeResult = os.remove(path);
    if (removeResult !== 0) {
      std.out.puts("rmdir: " + requested + ": errno " + removeResult + "\n");
    }
  }
  prompt();
}

function runMv(words) {
  if (words.length !== 3) {
    std.out.puts("mv: usage: mv OLD NEW\n");
    prompt();
    return;
  }
  const oldPath = resolveShellPath(words[1]);
  const newPath = resolveShellPath(words[2]);
  if (oldPath[0] === "#" || newPath[0] === "#") {
    std.out.puts("mv: service paths are not renamed by shell commands\n");
    prompt();
    return;
  }
  const result = os.rename(oldPath, newPath);
  if (result !== 0) {
    std.out.puts("mv: " + words[1] + ": errno " + result + "\n");
  }
  prompt();
}

function runCp(words) {
  if (words.length !== 3) {
    std.out.puts("cp: usage: cp SOURCE DEST\n");
    prompt();
    return;
  }
  const source = resolveShellPath(words[1]);
  const destination = resolveShellPath(words[2]);
  if (destination[0] === "#") {
    std.out.puts("cp: " + words[2] + ": service paths are not written by shell commands\n");
    prompt();
    return;
  }
  try {
    const text = std.loadFile(source);
    if (text === null || text === undefined) {
      std.out.puts("cp: " + words[1] + ": not found\n");
      prompt();
      return;
    }
    const count = writeText(destination, text);
    if (count < 0) {
      std.out.puts("cp: " + words[2] + ": errno " + count + "\n");
    }
  } catch (error) {
    std.out.puts("cp: " + words[1] + ": " + error.message + "\n");
  }
  prompt();
}

function runLn(words) {
  if (words.length !== 4 || words[1] !== "-s") {
    std.out.puts("ln: usage: ln -s TARGET LINK\n");
    prompt();
    return;
  }
  const linkPath = resolveShellPath(words[3]);
  if (linkPath[0] === "#") {
    std.out.puts("ln: " + words[3] + ": service paths are not written by shell commands\n");
    prompt();
    return;
  }
  const result = os.symlink(words[2], linkPath);
  if (result !== 0) {
    std.out.puts("ln: " + words[3] + ": errno " + result + "\n");
  }
  prompt();
}

function runReadlink(words) {
  if (words.length !== 2) {
    std.out.puts("readlink: usage: readlink PATH\n");
    prompt();
    return;
  }
  const path = resolveShellPath(words[1]);
  const [target, err] = os.readlink(path);
  if (err !== 0) {
    std.out.puts("readlink: " + words[1] + ": errno " + err + "\n");
  } else {
    std.out.puts(target + "\n");
  }
  prompt();
}

function runQjs(words) {
  if (words.length < 2) {
    std.out.puts("qjs: usage: qjs SCRIPT [ARGS...] [< STDIN] [> STDOUT] [2> STDERR]\n");
    prompt();
    return;
  }
  const launch = parseQjsLaunch(words);
  if (launch.error) {
    std.out.puts(launch.error + "\n");
    prompt();
    return;
  }
  const scriptPath = resolveShellPath(launch.script);
  if (scriptPath[0] === "#") {
    std.out.puts("qjs: " + launch.script + ": service paths are not executable scripts\n");
    prompt();
    return;
  }
  const childDir = namespaceDir(scriptPath);
  const scriptName = namespaceBase(scriptPath);
  const command = [scriptName].concat(launch.args).map(quoteCommandWord).join(" ") + "\n";
  try {
    const parent = readServiceText("#task/self/id").trim();
    const child = readServiceText("#task/new/qjs").trim();
    const taskPath = "#task/" + child;
    writeRequiredServiceText(taskPath + "/cmd", command);
    writeRequiredServiceText(taskPath + "/env", readServiceText("#task/self/env"));
    writeRequiredServiceText(taskPath + "/dir", childDir + "\n");
    if (launch.stdinPath) {
      writeRequiredServiceText(taskPath + "/ctl", "bind " + quoteCommandWord(launch.stdinPath) + " fd/0\n");
    } else {
      writeRequiredServiceText(taskPath + "/ctl", "bind #task/" + parent + "/fd/0 fd/0\n");
    }
    if (launch.stdoutPath) {
      if (launch.stdoutPath[0] !== "#") {
        writeText(launch.stdoutPath, "");
      }
      writeRequiredServiceText(taskPath + "/ctl", "bind " + quoteCommandWord(launch.stdoutPath) + " fd/1\n");
    } else {
      writeRequiredServiceText(taskPath + "/ctl", "bind #task/" + parent + "/fd/1 fd/1\n");
    }
    if (launch.stderrPath) {
      if (launch.stderrPath[0] !== "#") {
        writeText(launch.stderrPath, "");
      }
      writeRequiredServiceText(taskPath + "/ctl", "bind " + quoteCommandWord(launch.stderrPath) + " fd/2\n");
    } else {
      writeRequiredServiceText(taskPath + "/ctl", "bind #task/" + parent + "/fd/2 fd/2\n");
    }
    writeRequiredServiceText(taskPath + "/ctl", "start\n");
    const exit = readServiceText(taskPath + "/exit").trim();
    lastStatus = exit ? Number(exit) : 0;
    if (!Number.isFinite(lastStatus)) {
      lastStatus = 1;
    }
    if (lastStatus !== 0) {
      std.out.puts("qjs exit " + exit + "\n");
    }
  } catch (error) {
    lastStatus = 1;
    std.out.puts("qjs: " + error.message + "\n");
  }
  prompt();
}

function runCommand(line) {
  const trimmed = line.trim();
  if (trimmed === "") {
    prompt();
    return;
  }
  const parsed = parseWords(trimmed);
  if (parsed.error) {
    std.out.puts(parsed.error + "\n");
    prompt();
    return;
  }
  const words = parsed.words;
  if (trimmed === "exit") {
    running = false;
    os.setReadHandler(0, null);
    if (winchFd >= 0) {
      os.close(winchFd);
    }
    std.out.puts("bye\n");
    std.out.flush();
    std.exit(0);
    return;
  }
  if (trimmed === "id") {
    std.out.puts(std.loadFile("#task/self/id").trim() + "\n");
    prompt();
    return;
  }
  if (trimmed === "pwd") {
    std.out.puts(cwd + "\n");
    prompt();
    return;
  }
  if (trimmed === "status") {
    std.out.puts("status " + lastStatus + "\n");
    prompt();
    return;
  }
  if (words[0] === "env") {
    runEnv(words);
    return;
  }
  if (words[0] === "setenv") {
    runSetenv(words);
    return;
  }
  if (words[0] === "unsetenv") {
    runUnsetenv(words);
    return;
  }
  if (words[0] === "ls") {
    runLs(words);
    return;
  }
  if (words[0] === "cd") {
    runCd(words);
    return;
  }
  if (words[0] === "cat") {
    runCat(words);
    return;
  }
  if (words[0] === "write") {
    runWrite(words);
    return;
  }
  if (words[0] === "mkdir") {
    runMkdir(words);
    return;
  }
  if (words[0] === "rm") {
    runRm(words);
    return;
  }
  if (words[0] === "rmdir") {
    runRmdir(words);
    return;
  }
  if (words[0] === "mv") {
    runMv(words);
    return;
  }
  if (words[0] === "cp") {
    runCp(words);
    return;
  }
  if (words[0] === "ln") {
    runLn(words);
    return;
  }
  if (words[0] === "readlink") {
    runReadlink(words);
    return;
  }
  if (words[0] === "qjs") {
    runQjs(words);
    return;
  }
  if (trimmed === "size") {
    drainWinch();
    std.out.puts("size " + (terminalSize || "unknown") + "\n");
    prompt();
    return;
  }
  if (trimmed.startsWith("echo ")) {
    std.out.puts(trimmed.slice(5) + "\n");
    prompt();
    return;
  }
  if (trimmed.startsWith("later ")) {
    const message = trimmed.slice(6);
    std.out.puts("scheduled\n");
    os.setTimeout(() => {
      if (!running) {
        return;
      }
      std.out.puts("later: " + message + "\n");
      prompt();
      std.out.flush();
    }, 1);
    return;
  }
  std.out.puts("unknown: " + trimmed + "\n");
  prompt();
}

function runInputLine(line, bufferedRemainder) {
  const handoff = bufferedRemainder.length > 0 && commandUsesTerminalStdin(line);
  if (handoff) {
    writeTerminalInput(bufferedRemainder);
  }
  runCommand(line);
  return handoff;
}

function handleRawByte(byte) {
  if (byte === 0x03) {
    pending = "";
    std.out.puts("^C\n");
    prompt();
    return;
  }
  if (byte === 0x04 && pending.length === 0) {
    runCommand("exit");
    return;
  }
  if (byte === 0x08 || byte === 0x7f) {
    if (pending.length > 0) {
      pending = pending.slice(0, -1);
      std.out.puts("\x08 \x08");
    }
    return;
  }
  if (byte === 0x09 || byte >= 0x20) {
    const char = String.fromCharCode(byte);
    pending += char;
    std.out.puts(char);
  }
}

function handleRawInput(bytes, count) {
  for (let i = 0; i < count; i++) {
    if (!running) {
      return;
    }
    if (bytes[i] === 0x0d || bytes[i] === 0x0a) {
      std.out.puts("\n");
      const line = pending;
      pending = "";
      const remainder = stringFromBytes(bytes.slice(i + 1, count), count - i - 1);
      if (runInputLine(line, remainder)) {
        return;
      }
      continue;
    }
    handleRawByte(bytes[i]);
  }
}

function handleLineInput(bytes, count) {
  pending += stringFromBytes(bytes, count);
  let newline;
  while ((newline = pending.indexOf("\n")) >= 0) {
    const line = pending.slice(0, newline);
    const remainder = pending.slice(newline + 1);
    pending = remainder;
    if (runInputLine(line, remainder)) {
      pending = "";
      return;
    }
  }
}

const winchFd = os.open("#term/" + termId + "/winch", os.O_RDONLY);
function drainWinch() {
  if (winchFd < 0) {
    return;
  }
  const bytes = new Uint8Array(64);
  let count;
  while ((count = os.read(winchFd, bytes.buffer, 0, bytes.length)) > 0) {
    const lines = stringFromBytes(bytes, count).trim().split("\n").filter(Boolean);
    if (lines.length > 0) {
      terminalSize = lines[lines.length - 1];
    }
  }
}

std.out.puts("shell task: " + std.loadFile("#task/self/id").trim() + "\n");
prompt();

os.setReadHandler(0, () => {
  const bytes = new Uint8Array(256);
  const count = os.read(0, bytes.buffer, 0, bytes.length);
  if (count < 0) {
    throw new Error("shell terminal read failed: " + count);
  }
  if (rawInput) {
    handleRawInput(bytes, count);
  } else {
    handleLineInput(bytes, count);
  }
  std.out.flush();
});

std.out.flush();
