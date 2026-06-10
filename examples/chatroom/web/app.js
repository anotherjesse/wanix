// wanix chatroom web client — same-origin files, no build step, no framework.
//
// The Wanix gateway (`wanix-rust serve --bind chat=...`) composes this static
// directory and the mesh-mounted room into ONE origin, so the room's files are
// plain same-origin URLs:
//
//   GET  /latest   bounded read: the last messages, display-rendered
//   GET  /stream   never-EOF feed; EventSource gets it as SSE `data:` lines
//   POST /post     write one message body
//   POST /nick     claim a display name for the WRITING principal
//   GET  /roster   the principal->nick map (display sugar)
//
// PRINCIPAL HONESTY (v0): the gateway dials the room with its own key, so the
// room sees one author — the gateway — for every browser user. This client
// never fakes per-user attribution: it renders messages as the room records
// them, and only locally marks the posts it sent itself this session.

const log = document.getElementById("log");
const status = document.getElementById("status");
let nicks = {}; // principal -> nick, display sugar only
const sentByMe = []; // bodies posted from this page, for the local "you" tag

function setStatus(text) {
  status.textContent = text;
}

// shorthex display of a principal like "iroh:<64-hex>" — mirrors the room.
function shortHex(principal) {
  const colon = principal.indexOf(":");
  return (colon >= 0 ? principal.slice(colon + 1) : principal).slice(0, 8);
}

function displayFrom(from) {
  // `latest` already renders "nick (shorthex)"; `stream` carries the raw
  // principal, so render it the same way with the fetched roster.
  if (from.indexOf(":") < 0) return from;
  const nick = nicks[from];
  const short = "(" + shortHex(from) + ")";
  return typeof nick === "string" ? nick + " " + short : short;
}

function appendMessage(message, mine) {
  const line = document.createElement("p");
  const from = document.createElement("span");
  from.className = "from";
  from.textContent = displayFrom(message.from);
  const body = document.createElement("span");
  body.textContent = message.body;
  line.append(from, body);
  if (mine) {
    const tag = document.createElement("span");
    tag.className = "you";
    tag.textContent = "via this gateway";
    line.append(tag);
  }
  log.append(line);
  log.scrollTop = log.scrollHeight;
}

function consumeLine(rawLine, fromStream) {
  if (!rawLine) return;
  let message;
  try {
    message = JSON.parse(rawLine);
  } catch (_notJson) {
    message = { from: "", body: rawLine };
  }
  let mine = false;
  if (fromStream) {
    const claimed = sentByMe.indexOf(message.body);
    if (claimed >= 0) {
      sentByMe.splice(claimed, 1);
      mine = true;
    }
  }
  appendMessage(message, mine);
}

async function loadRoster() {
  try {
    nicks = await (await fetch("roster")).json();
  } catch (_unavailable) {
    nicks = {};
  }
}

async function loadLatest() {
  const text = await (await fetch("latest")).text();
  for (const line of text.split("\n")) consumeLine(line, false);
}

function followStream() {
  // EventSource sends `Accept: text/event-stream`; the gateway answers the
  // never-EOF stream file as SSE `data:` events, one room line per event.
  const source = new EventSource("stream");
  source.onmessage = (event) => consumeLine(event.data, true);
  source.onopen = () => setStatus("live");
  source.onerror = () => setStatus("stream disconnected — reload to rejoin");
}

document.getElementById("post-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const input = document.getElementById("message");
  const body = input.value.trim();
  if (!body) return;
  sentByMe.push(body);
  const response = await fetch("post", { method: "POST", body });
  if (!response.ok) setStatus("post failed: " + response.status);
  input.value = "";
});

document.getElementById("nick-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const input = document.getElementById("nick");
  const nick = input.value.trim();
  if (!nick) return;
  const response = await fetch("nick", { method: "POST", body: nick });
  if (!response.ok) {
    setStatus("nick failed: " + response.status);
    return;
  }
  // The nick names the GATEWAY principal — every browser user of this
  // gateway shares it. Refresh the roster so new stream lines render it.
  await loadRoster();
  setStatus('nick set for the gateway principal (shared by all web users)');
  input.value = "";
});

(async () => {
  await loadRoster();
  await loadLatest();
  followStream();
})();
