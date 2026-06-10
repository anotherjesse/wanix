// chatroom verb: post a message to the room this verb shipped with.
//
// Invoked from a shell that mounted the room: `room:post hello world`.
// Runs CONFINED (ADR 0007 §Confinement contract): the namespace is exactly
// the room at /res plus stdio and argv — this code cannot see anything else.
//
// Input convention: argv joined with spaces is the message body; with no argv
// the body is read from stdin, so `echo hi | room:post` composes.
//
// Self-contained by design: a /bin verb resolves imports against the confined
// namespace root, so it must not import local libraries.
import * as std from "qjs:std";

const args = scriptArgs.slice(1);
const body = args.length ? args.join(" ") : std.in.readAsString();
if (!body) {
  std.err.puts("post: empty message (pass words or pipe a body)\n");
  std.exit(1);
}
const file = std.open("/res/post", "w");
if (!file) {
  std.err.puts("post: cannot open /res/post\n");
  std.exit(1);
}
file.puts(body);
file.close();
