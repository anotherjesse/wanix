import * as vscode from 'vscode';
import { WanixBridge } from './bridge.js';

export const WANIX_INSPECT_SCHEME = "wanix-inspect";

export class WanixServiceInspector implements vscode.TextDocumentContentProvider, vscode.DocumentLinkProvider, vscode.Disposable {
	private readonly emitter = new vscode.EventEmitter<vscode.Uri>();
	readonly onDidChange = this.emitter.event;

	constructor(
		private readonly fsys: any,
		private readonly bridge: WanixBridge,
	) {}

	async open(path: string): Promise<void> {
		const uri = inspectUri(path);
		this.emitter.fire(uri);
		const document = await vscode.workspace.openTextDocument(uri);
		await vscode.window.showTextDocument(document, { preview: false });
	}

	async provideTextDocumentContent(uri: vscode.Uri): Promise<string> {
		const path = inspectPath(uri);
		const displayPath = displayWanixPath(path);
		const fsPath = this.bridge.normalizePath(uriPath(path));
		let stat: any;
		try {
			stat = await this.fsys.stat(fsPath);
		} catch (error) {
			return [
				`# Wanix Inspect: ${displayPath}`,
				"",
				`Could not stat ${displayPath}.`,
				"",
				String(error instanceof Error ? error.message : error),
			].join("\n");
		}

		if (!stat?.IsDir) {
			return [
				`# Wanix Inspect: ${displayPath}`,
				"",
				"Kind: file",
				`Size: ${stat?.Size ?? "?"}`,
				"",
				"Open this path through the normal Wanix file provider to read it.",
			].join("\n");
		}

		let entries: string[] = [];
		try {
			entries = normalizeEntries(await this.fsys.readDir(fsPath));
		} catch (error) {
			return [
				`# Wanix Inspect: ${displayPath}`,
				"",
				"Kind: directory",
				"",
				`Could not read directory entries for ${displayPath}.`,
				"",
				String(error instanceof Error ? error.message : error),
			].join("\n");
		}

		return [
			`# Wanix Inspect: ${displayPath}`,
			"",
			"Kind: directory",
			`Backing path: ${fsPath}`,
			"",
			"## Entries",
			"",
			...(entries.length > 0
				? entries.map((entry) => renderEntry(displayPath, entry))
				: ["- empty"]),
			"",
			"## Note",
			"",
			"Service directories are reachable even when they are hidden from ordinary root listings.",
			"This snapshot lists paths without reading files that allocate resources.",
			"Safe metadata files and child directories are document links.",
			"Allocator, control, and stream files stay plain text with reasons.",
		].join("\n");
	}

	provideDocumentLinks(document: vscode.TextDocument): vscode.DocumentLink[] {
		const links: vscode.DocumentLink[] = [];
		for (let lineNumber = 0; lineNumber < document.lineCount; lineNumber += 1) {
			const line = document.lineAt(lineNumber);
			const entry = serviceEntryFromLine(line.text);
			if (!entry) {
				continue;
			}
			const target = entryTarget(entry);
			if (!target) {
				continue;
			}
			const start = line.text.indexOf(entry.path);
			const range = new vscode.Range(lineNumber, start, lineNumber, start + entry.path.length);
			const link = new vscode.DocumentLink(range, target);
			link.tooltip = entry.kind === "dir" ? `Inspect ${entry.path}` : `Open ${entry.path}`;
			links.push(link);
		}
		return links;
	}

	dispose(): void {
		this.emitter.dispose();
	}
}

function inspectUri(path: string): vscode.Uri {
	return vscode.Uri.from({
		scheme: WANIX_INSPECT_SCHEME,
		path: uriPath(path),
	});
}

function inspectPath(uri: vscode.Uri): string {
	return uri.path || "/";
}

function uriPath(path: string): string {
	const trimmed = path.trim();
	if (!trimmed || trimmed === ".") {
		return "/";
	}
	return trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
}

function displayWanixPath(path: string): string {
	const trimmed = path.trim();
	if (!trimmed || trimmed === "." || trimmed === "/") {
		return "/";
	}
	return trimmed.startsWith("/") && trimmed.startsWith("/#") ? trimmed.slice(1) : trimmed;
}

function normalizeEntries(entries: unknown): string[] {
	if (!Array.isArray(entries)) {
		return [];
	}
	return entries
		.map((entry) => typeof entry === "string" ? entry : entryNameFromObject(entry))
		.filter((entry): entry is string => !!entry)
		.sort((left, right) => entryName(left).localeCompare(entryName(right), undefined, { numeric: true }));
}

type ServiceEntry = {
	kind: "dir" | "file";
	path: string;
};

function serviceEntryFromLine(line: string): ServiceEntry | undefined {
	const match = line.match(/^- (dir|file) (\S+)/);
	if (!match) {
		return undefined;
	}
	return {
		kind: match[1] as "dir" | "file",
		path: match[2].trim(),
	};
}

function entryTarget(entry: ServiceEntry): vscode.Uri | undefined {
	if (entry.kind === "dir") {
		return inspectUri(entry.path);
	}
	if (!isSafeFileLink(entry.path)) {
		return undefined;
	}
	return vscode.Uri.from({
		scheme: WanixBridge.scheme,
		path: uriPath(entry.path),
	});
}

function isSafeFileLink(path: string): boolean {
	return unsafeFileReason(path) === undefined;
}

/**
 * Classifies a Wanix service path against the device file taxonomy:
 *
 * - ALLOCATOR: reading the file mints a resource (e.g. `#agent/new`,
 *   `#pipe/new`, `#cas/ingest`). Returning a reason keeps the inspector from
 *   silently spawning sessions/channels/blob handles when a user clicks a link.
 * - METADATA: safe read-only one-shot snapshots (`id`, `status`, `kind`, `exit`,
 *   `cmd`, `dir`, `env`, `pending`). These return `undefined` so the inspector
 *   renders them as plain links.
 * - CONTROL: write-only verbs (`ctl`). Linking a read open is meaningless and
 *   sometimes refused by the device, so they stay plain text.
 * - STREAMS: blocking/live I/O (`events`, `data`, `prompt`, `reply`, `send`,
 *   `recv`, `program`, `winch`). Opening these can consume live data or block
 *   indefinitely, so they stay plain text with an explicit reason.
 *
 * For an unrecognized service device (`#mesh`, `#cpu`, future devices), the
 * classifier falls back to safe-by-default: it links recognized metadata names
 * and leaves everything else plain.
 */
function unsafeFileReason(path: string): string | undefined {
	const normalized = displayWanixPath(path).replace(/^\/+/, "");
	const parts = normalized.split("/");
	if (!parts[0]?.startsWith("#")) {
		return undefined;
	}
	const device = parts[0];
	const leaf = parts[parts.length - 1] ?? "";
	switch (device) {
		case "#task":
			return taskReason(parts);
		case "#term":
			return termReason(parts);
		case "#agent":
			return agentReason(parts);
		case "#pipe":
			return pipeReason(parts);
		case "#kv":
			return kvReason(parts);
		case "#plumb":
			return plumbReason(parts);
		case "#cas":
			return casReason(parts);
		case "#mesh":
			return meshReason(parts);
		case "#cpu":
			return cpuReason(parts);
		default:
			return genericReason(leaf);
	}
}

// `#task`: per-task metadata (cmd/dir/env/exit/id/kind) is a safe snapshot; the
// task fd table is a stream directory; ctl is write-only.
function taskReason(parts: string[]): string | undefined {
	if (parts.length === 3 && TASK_METADATA.includes(parts[2])) {
		return undefined;
	}
	if (parts[2] === "ctl") {
		return "control file; write-only operations are not linked";
	}
	if (parts[2] === "fd") {
		return "task fd stream; inspect the fd directory first";
	}
	return "task service file; left plain until its read semantics are explicit";
}
const TASK_METADATA = ["cmd", "dir", "env", "exit", "id", "kind"];

// `#term`: `new` allocates, `data`/`program` are live streams, `winch` is a
// resize feed, `ctl` is write-only.
function termReason(parts: string[]): string | undefined {
	if (parts.length === 2 && parts[1] === "new") {
		return "allocator file; reading it creates a terminal";
	}
	if (parts[2] === "ctl") {
		return "control file; write-only operations are not linked";
	}
	if (["data", "program"].includes(parts[2])) {
		return "terminal stream; opening it can consume live I/O";
	}
	if (parts[2] === "winch") {
		return "terminal resize feed; left plain until stream reads are explicit";
	}
	return "terminal service file; left plain until its read semantics are explicit";
}

// `#agent`: `new` allocates a session, `events`/`prompt`/`reply` are streaming,
// `ctl` is write-only, `id`/`status`/`pending` are safe snapshots.
function agentReason(parts: string[]): string | undefined {
	if (parts.length === 2 && parts[1] === "new") {
		return "allocator file; reading it creates an agent session";
	}
	if (parts.length === 3 && ["id", "status", "pending"].includes(parts[2])) {
		return undefined;
	}
	if (parts[2] === "ctl") {
		return "control file; write-only operations are not linked";
	}
	if (parts[2] === "events") {
		return "agent event stream; opening it blocks for live events";
	}
	if (parts[2] === "prompt") {
		return "agent prompt sink; write-only turn submission";
	}
	if (parts[2] === "reply") {
		return "agent reply stream; opening it blocks for the next reply";
	}
	return "agent service file; left plain until its read semantics are explicit";
}

// `#pipe`: `new` allocates a channel, `<id>/data` is a live stream, `<id>/id`
// is a safe snapshot.
function pipeReason(parts: string[]): string | undefined {
	if (parts.length === 2 && parts[1] === "new") {
		return "allocator file; reading it creates a pipe channel";
	}
	if (parts.length === 3 && parts[2] === "id") {
		return undefined;
	}
	if (parts[2] === "data") {
		return "pipe data stream; opening it consumes or blocks for bytes";
	}
	return "pipe service file; left plain until its read semantics are explicit";
}

// `#kv`: each entry is a value file; the device is a flat key/value namespace
// with no allocator, control, or stream files, so every key is safe to link.
function kvReason(parts: string[]): string | undefined {
	if (parts.length === 2) {
		return undefined;
	}
	return "kv service file; left plain until its read semantics are explicit";
}

// `#plumb`: `<topic>/send` is write-only, `<topic>/recv` is a blocking
// subscription stream. The topic directory itself has no metadata files.
function plumbReason(parts: string[]): string | undefined {
	if (parts[2] === "send") {
		return "plumber publish sink; write-only envelope submission";
	}
	if (parts[2] === "recv") {
		return "plumber subscription stream; opening it blocks for envelopes";
	}
	return "plumb service file; left plain until its read semantics are explicit";
}

// `#cas`: `<hash>` blob reads are safe one-shots, `have/<hash>` is a safe
// presence probe, but `ingest` is write-then-read-hash (an allocator).
function casReason(parts: string[]): string | undefined {
	if (parts.length === 2 && parts[1] === "ingest") {
		return "allocator file; writing then reading ingests a new blob";
	}
	if (parts.length === 2) {
		return undefined;
	}
	if (parts.length === 3 && parts[1] === "have") {
		return undefined;
	}
	return "cas service file; left plain until its read semantics are explicit";
}

// `#mesh`: peer/identity surfaces are read-only snapshots; anything resembling
// `new`/`dial`/`accept` is an allocator and stays plain.
function meshReason(parts: string[]): string | undefined {
	const leaf = parts[parts.length - 1] ?? "";
	if (["new", "dial", "accept", "import", "export"].includes(leaf)) {
		return "allocator file; reading it can open a mesh resource";
	}
	if (leaf === "ctl") {
		return "control file; write-only operations are not linked";
	}
	if (["events", "data", "messages"].includes(leaf)) {
		return "mesh stream; opening it can consume live I/O";
	}
	if (["id", "status", "kind", "peers", "peer", "addr", "alpn"].includes(leaf)) {
		return undefined;
	}
	return "mesh service file; left plain until its read semantics are explicit";
}

// `#cpu`: jobs are spawned by reading `new`/`dial`; control/event streams are
// live; `status`/`id`/`exit` snapshots are safe.
function cpuReason(parts: string[]): string | undefined {
	const leaf = parts[parts.length - 1] ?? "";
	if (["new", "dial", "submit"].includes(leaf)) {
		return "allocator file; reading it can dispatch a remote job";
	}
	if (leaf === "ctl") {
		return "control file; write-only operations are not linked";
	}
	if (["events", "stdout", "stderr", "stdin", "control"].includes(leaf)) {
		return "cpu job stream; opening it can consume live I/O";
	}
	if (["id", "status", "exit", "kind", "spec"].includes(leaf)) {
		return undefined;
	}
	return "cpu service file; left plain until its read semantics are explicit";
}

// Unknown `#device`: link the small set of universally safe metadata names and
// leave everything else plain rather than guessing at allocator semantics.
function genericReason(leaf: string): string | undefined {
	if (["id", "status", "kind", "exit"].includes(leaf)) {
		return undefined;
	}
	return "service file; left plain until its read semantics are explicit";
}

function entryNameFromObject(entry: unknown): string | undefined {
	if (!entry || typeof entry !== "object") {
		return undefined;
	}
	const candidate = entry as { Name?: string; IsDir?: boolean };
	if (!candidate.Name) {
		return undefined;
	}
	return candidate.IsDir ? `${candidate.Name}/` : candidate.Name;
}

function entryName(entry: string): string {
	return entry.replace(/\/$/, "");
}

function entryKind(entry: string): string {
	return entry.endsWith("/") ? "dir" : "file";
}

function renderEntry(parent: string, entry: string): string {
	const path = joinWanixPath(parent, entryName(entry));
	const reason = entry.endsWith("/") ? undefined : unsafeFileReason(path);
	const suffix = reason ? ` -- ${reason}` : "";
	return `- ${entryKind(entry)} ${path}${suffix}`;
}

function joinWanixPath(parent: string, child: string): string {
	const cleanParent = displayWanixPath(parent).replace(/\/+$/, "");
	if (!cleanParent || cleanParent === "/") {
		return `/${child}`;
	}
	return `${cleanParent}/${child}`;
}
