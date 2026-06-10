// chatroom verb: pretty-print the room's nick roster.
//
// Invoked from a shell that mounted the room: `room:roster`. Runs CONFINED
// (ADR 0007 §Confinement contract): only the room at /res is visible.
//
// /res/roster is the raw principal->nick JSON map; display attribution is
// derived here the same way the room's `latest` renders it — "nick (shorthex)"
// — because a nick is display sugar, never authority.
import * as std from "qjs:std";

const text = std.loadFile("/res/roster");
if (text === null) {
  std.err.puts("roster: cannot read /res/roster\n");
  std.exit(1);
}
let roster = {};
try {
  roster = JSON.parse(text);
} catch (_corrupt) {
  std.err.puts("roster: /res/roster is not JSON\n");
  std.exit(1);
}
const shortHex = (principal) => {
  const colon = principal.indexOf(":");
  return (colon >= 0 ? principal.slice(colon + 1) : principal).slice(0, 8);
};
const entries = Object.entries(roster).sort((a, b) => (a[1] < b[1] ? -1 : 1));
if (entries.length === 0) {
  std.out.puts("(no nicks claimed)\n");
} else {
  for (const [principal, nick] of entries) {
    std.out.puts(nick + " (" + shortHex(principal) + ")\n");
  }
}
