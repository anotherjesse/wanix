import * as vscode from 'vscode';
import { WanixBridge } from './bridge.js';
import { WanixSystemView } from './system-view.js';

// The agent device follows the same `#name` service-path convention as `#term`
// and `#task`. Inside the wanix workspace it appears at the top of the
// namespace, so the bridge addresses it as `/#agent/...`.
const AGENT_DEVICE = "/#agent";
const AGENT_NEW = `${AGENT_DEVICE}/new`;
const AGENT_DIR = "/agent";
const AGENT_OUT_DIR = `${AGENT_DIR}/out`;

export const AGENT_BROKEN_PATH = `${AGENT_DIR}/broken.js`;
export const AGENT_RESULT_PATH = `${AGENT_OUT_DIR}/result.txt`;

const BROKEN_JS = `import * as std from "qjs:std";
import * as os from "qjs:os";

os.mkdir("out", 0o777);
if (typeof missingValue === "undefined") {
  std.err.puts("ReferenceError: missingValue is not defined\\n");
  std.exit(1);
}
std.writeFile("out/result.txt", missingValue.toUpperCase() + "\\n");
std.out.puts("wrote out/result.txt\\n");
`;

const REPAIR_PROMPT = [
	"The file /agent/broken.js fails with ReferenceError: missingValue is not defined.",
	"Propose a minimal patch that declares missingValue so the script writes out/result.txt.",
	"When you would apply a change, pause for approval via #agent ctl.",
].join(" ");

const VALUE_DECLARATION = `const missingValue = "fixed by Wanix agent";`;

// Deterministic synchronous repair used by the qjs command handler in
// extension.ts. The #agent-driven flow above streams events and gates on
// approvals; this thin transform mirrors the same fix (insert the missing
// `missingValue` declaration after the import block) so the command can apply
// the repair in-process without a live #agent session.
export function repairQjsProgram(source: string): string {
	if (source.includes(VALUE_DECLARATION)) return source;
	if (!source.includes("missingValue.toUpperCase()")) {
		throw new Error("Deterministic repair only knows how to fix missingValue.toUpperCase() demos");
	}
	const lines = source.split("\n");
	let insertAt = 0;
	while (insertAt < lines.length && lines[insertAt].startsWith("import ")) insertAt += 1;
	if (lines[insertAt] === "") insertAt += 1;
	lines.splice(insertAt, 0, VALUE_DECLARATION);
	return lines.join("\n");
}

const decoder = new TextDecoder();
const encoder = new TextEncoder();

type AgentStep = (label: string, options?: { icon?: string; path?: string; description?: string }) => void;

// Drive the #agent device to mediate a repair: open a session, submit the patch
// plan as an `approve:`-prefixed prompt so the device parks an approval request,
// resolve it through `ctl`, then return the approved patch for the caller to
// apply. The served #agent runs the deterministic FakeEngine (no filesystem
// access), so the cockpit computes the concrete patch while the device owns the
// session identity and the approval trust-gate. `onStep` records each device
// interaction in the Agent trace. Falls back to the in-process patch only if the
// device is unreachable (services disabled), so the demo never hard-fails.
export async function repairViaAgent(
	fsys: any,
	source: string,
	onStep: AgentStep = () => {},
): Promise<string> {
	const repaired = repairQjsProgram(source);
	if (repaired === source) {
		return source;
	}
	let id: string;
	try {
		id = await allocSession(fsys);
	} catch (error: unknown) {
		log(`#agent/new unavailable, applying patch directly: ${String(error)}`);
		onStep("#agent unavailable; applied patch directly", { icon: "warning" });
		return repaired;
	}
	const sessionDir = `${AGENT_DEVICE}/${id}`;
	onStep(`open #agent session ${id}`, { icon: "comment-discussion", path: `${sessionDir}/events` });
	const plan = "declare missingValue so /agent/broken.js writes out/result.txt";
	await fsys.writeFile(stripLeading(`${sessionDir}/prompt`), encoder.encode(`approve: ${plan}`));
	onStep("propose patch to #agent", { icon: "comment", path: `${sessionDir}/prompt` });
	try {
		await approveAgentPlan(fsys, id, sessionDir, onStep);
	} finally {
		try {
			await fsys.writeFile(stripLeading(`${sessionDir}/ctl`), encoder.encode("close\n"));
		} catch (error: unknown) {
			console.warn("Wanix agent close failed", error);
		}
	}
	return repaired;
}

// Poll the non-blocking `pending` snapshot until the device parks the approval,
// then resolve it via `ctl approve`. Uses snapshot reads (pending) rather than
// the streaming events file so it never blocks the extension host.
async function approveAgentPlan(
	fsys: any,
	id: string,
	sessionDir: string,
	onStep: AgentStep,
): Promise<void> {
	const deadline = Date.now() + 15_000;
	while (Date.now() < deadline) {
		const raw = decoder.decode(await fsys.readFile(stripLeading(`${sessionDir}/pending`)));
		const requests = parsePending(raw);
		if (requests.length > 0) {
			for (const request of requests) {
				onStep(`approval needed: ${request.action}`, { icon: "shield", path: `${sessionDir}/pending` });
				log(`#agent/${id}/ctl <- approve ${request.id}`);
				await fsys.writeFile(
					stripLeading(`${sessionDir}/ctl`),
					encoder.encode(`approve ${request.id}\n`),
				);
				onStep("approved via #agent ctl", { icon: "pass", path: `${sessionDir}/ctl` });
			}
			return;
		}
		await sleep(120);
	}
	throw new Error("Wanix #agent did not surface an approval request in time");
}

type PendingRequest = { id: string; action: string };
type StreamEvent = { t: string; [k: string]: unknown };

let output: vscode.OutputChannel | undefined;

function log(line: string): void {
	if (!output) {
		output = vscode.window.createOutputChannel("Wanix Agent");
	}
	output.appendLine(line);
	output.show(true);
}

// Keep the installer that seeds broken.js so the demo has something to repair.
export async function installAgentRepairDemo(
	fsys: any,
	bridge: WanixBridge,
	systemView?: WanixSystemView,
): Promise<void> {
	await fsys.makeDirAll(AGENT_DIR);
	await removeIfExists(fsys, AGENT_RESULT_PATH);
	await fsys.writeFile(AGENT_BROKEN_PATH, BROKEN_JS);
	bridge.refresh(AGENT_DIR);
	bridge.refresh(AGENT_BROKEN_PATH);
	await refreshExplorer();
	await openWanixFile(AGENT_BROKEN_PATH);
	log(`installed ${AGENT_BROKEN_PATH}`);
	systemView?.filesystemActivity("agent repair demo installed");
	vscode.window.showInformationMessage("Installed Wanix agent repair demo in /agent");
}

// Drive a full repair via the #agent device: allocate a session, submit the
// repair prompt, stream events, gate on pending approvals, then show the
// resulting reply and refresh the tree.
export async function runAgentRepair(
	fsys: any,
	bridge: WanixBridge,
): Promise<void> {
	const before = decoder.decode(await fsys.readFile(stripLeading(AGENT_BROKEN_PATH)));

	const id = await allocSession(fsys);
	log(`#agent/new -> session ${id}`);
	const sessionDir = `${AGENT_DEVICE}/${id}`;

	await fsys.writeFile(stripLeading(`${sessionDir}/prompt`), encoder.encode(REPAIR_PROMPT));
	log(`#agent/${id}/prompt <- repair request`);

	await streamUntilCompletion(fsys, id, sessionDir);

	// Close the session — the device removes it and EOFs any open readers.
	try {
		await fsys.writeFile(stripLeading(`${sessionDir}/ctl`), encoder.encode("close\n"));
	} catch (error: unknown) {
		console.warn("Wanix agent close failed", error);
	}

	const after = decoder.decode(await fsys.readFile(stripLeading(AGENT_BROKEN_PATH)));
	await showDiff(before, after);
	bridge.refresh(AGENT_DIR);
	bridge.refresh(AGENT_BROKEN_PATH);
	await refreshExplorer();
}

async function allocSession(fsys: any): Promise<string> {
	const bytes: Uint8Array = await fsys.readFile(stripLeading(AGENT_NEW));
	const id = decoder.decode(bytes).trim();
	if (!id) {
		throw new Error("Wanix #agent/new returned an empty session id");
	}
	return id;
}

async function streamUntilCompletion(
	fsys: any,
	id: string,
	sessionDir: string,
): Promise<void> {
	const eventsPath = stripLeading(`${sessionDir}/events`);
	const seen = new Set<string>();
	let buffer = "";
	const deadline = Date.now() + 30_000;
	while (Date.now() < deadline) {
		// The events file is a streaming read in the device; a single readFile
		// returns whatever JSONL lines are currently available.
		const chunk: Uint8Array = await fsys.readFile(eventsPath);
		buffer += decoder.decode(chunk);
		const lines = buffer.split("\n");
		buffer = lines.pop() || "";
		for (const line of lines) {
			if (!line.trim()) continue;
			const event = parseEvent(line);
			if (!event) continue;
			log(`event: ${line}`);
			if (event.t === "turn.completed") {
				return;
			}
			if (event.t === "approval.needed") {
				const requestId = String(event.id ?? "");
				if (!requestId || seen.has(requestId)) continue;
				seen.add(requestId);
				await handlePending(fsys, id, sessionDir);
			}
		}
		await sleep(150);
	}
	throw new Error("Wanix agent repair timed out before turn.completed");
}

async function handlePending(
	fsys: any,
	id: string,
	sessionDir: string,
): Promise<void> {
	const raw = decoder.decode(await fsys.readFile(stripLeading(`${sessionDir}/pending`)));
	const requests = parsePending(raw);
	for (const request of requests) {
		const choice = await vscode.window.showInformationMessage(
			`Wanix agent wants to: ${request.action}`,
			{ modal: true },
			"Approve",
			"Deny",
		);
		const decision = choice === "Approve" ? "approve" : "deny";
		log(`#agent/${id}/ctl <- ${decision} ${request.id}`);
		await fsys.writeFile(
			stripLeading(`${sessionDir}/ctl`),
			encoder.encode(`${decision} ${request.id}\n`),
		);
	}
}

function parseEvent(line: string): StreamEvent | undefined {
	try {
		const parsed = JSON.parse(line) as StreamEvent;
		return typeof parsed?.t === "string" ? parsed : undefined;
	} catch {
		return undefined;
	}
}

function parsePending(raw: string): PendingRequest[] {
	try {
		const parsed = JSON.parse(raw);
		if (!Array.isArray(parsed)) return [];
		return parsed
			.filter((entry) => entry && typeof entry === "object")
			.map((entry: any) => ({
				id: String(entry.id ?? ""),
				action: String(entry.action ?? ""),
			}))
			.filter((entry) => entry.id);
	} catch {
		return [];
	}
}

async function showDiff(before: string, after: string): Promise<void> {
	if (before === after) {
		vscode.window.showInformationMessage("Wanix agent: no edits applied");
		return;
	}
	const beforeUri = vscode.Uri.parse(`untitled:Wanix Agent - broken.js (before)`);
	const afterUri = vscode.Uri.parse(`untitled:Wanix Agent - broken.js (after)`);
	const beforeDoc = await vscode.workspace.openTextDocument(beforeUri.with({ scheme: "untitled" }));
	const afterDoc = await vscode.workspace.openTextDocument(afterUri.with({ scheme: "untitled" }));
	const beforeEdit = new vscode.WorkspaceEdit();
	beforeEdit.insert(beforeDoc.uri, new vscode.Position(0, 0), before);
	const afterEdit = new vscode.WorkspaceEdit();
	afterEdit.insert(afterDoc.uri, new vscode.Position(0, 0), after);
	await vscode.workspace.applyEdit(beforeEdit);
	await vscode.workspace.applyEdit(afterEdit);
	await vscode.commands.executeCommand("vscode.diff", beforeDoc.uri, afterDoc.uri, "Wanix Agent repair");
}

function stripLeading(path: string): string {
	return path.startsWith("/") ? path.slice(1) : path;
}

function sleep(ms: number): Promise<void> {
	return new Promise((resolve) => setTimeout(resolve, ms));
}

async function refreshExplorer(): Promise<void> {
	await Promise.resolve(vscode.commands.executeCommand("workbench.files.action.refreshFilesExplorer")).catch((error: unknown) => {
		console.warn("Wanix explorer refresh failed", error);
	});
}

async function removeIfExists(fsys: any, path: string): Promise<void> {
	try {
		await fsys.remove(path);
	} catch {
		// The demo reset path should tolerate first use.
	}
}

async function openWanixFile(path: string): Promise<void> {
	const document = await vscode.workspace.openTextDocument(vscode.Uri.from({
		scheme: WanixBridge.scheme,
		path,
	}));
	await vscode.window.showTextDocument(document, { preview: false });
}
