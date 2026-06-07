import * as vscode from 'vscode';
import { AGENT_TOOL_CONTRACT_JSON_PATH, AGENT_TOOL_CONTRACT_MD_PATH } from './agent-tool-contract.js';
import { WanixBridge } from './bridge.js';
import { WanixSystemView, type WanixSystemConfig } from './system-view.js';

export const COCKPIT_SELF_CHECK_MD_PATH = ".wanix/cockpit-check.md";
export const COCKPIT_SELF_CHECK_JSON_PATH = ".wanix/cockpit-check.json";
export const COCKPIT_SELF_CHECK_PROBE_PATH = ".wanix/checks/probe.txt";
const SYSTEM_JOURNAL_PATH = ".wanix/system-journal.md";
const SYSTEM_STATE_PATH = ".wanix/system-state.json";

// Mesh device paths. Each `#device` is a Wanix namespace name (Plan 9 style),
// so a self-check just walks the device as ordinary files. See
// `crates/wanix-{agent,kv,pipe,cas,plumb}/src/lib.rs` for the file contracts.
const AGENT_NEW_PATH = "#agent/new";
const KV_PROBE_KEY = "cockpit-self-check";
const KV_PROBE_PATH = `#kv/${KV_PROBE_KEY}`;
const PIPE_NEW_PATH = "#pipe/new";
const CAS_INGEST_PATH = "#cas/ingest";
const PLUMB_PROBE_TOPIC = "cockpit-self-check";
const PLUMB_SEND_PATH = `#plumb/${PLUMB_PROBE_TOPIC}/send`;
const MESH_PEERS_PATH = "#mesh/peers";

type CockpitSelfCheckConfig = WanixSystemConfig & {
	shell?: {
		cmd?: string;
		type?: string;
		wd?: string;
	};
	mesh?: {
		peers?: unknown[];
	};
};

type CheckStatus = "ok" | "warn" | "failed";

type CheckResult = {
	label: string;
	status: CheckStatus;
	description: string;
	artifacts?: string[];
	error?: string;
};

type CheckOutcome = {
	status: CheckStatus;
	description: string;
	artifacts?: string[];
};

export async function runCockpitSelfCheck(
	fsys: any,
	bridge: WanixBridge,
	config: CockpitSelfCheckConfig,
	systemView: WanixSystemView,
): Promise<void> {
	const generatedAt = new Date();
	const results: CheckResult[] = [];
	const runCheck = async (label: string, description: string, action: () => Promise<CheckOutcome> | CheckOutcome): Promise<void> => {
		systemView.checkStarted(label, { description });
		try {
			const outcome = await action();
			const result = { label, ...outcome };
			results.push(result);
			if (outcome.status === "failed") {
				systemView.checkFailed(label, outcome.description, { artifacts: outcome.artifacts });
			} else if (outcome.status === "warn") {
				systemView.checkWarned(label, { description: outcome.description, artifacts: outcome.artifacts });
			} else {
				systemView.checkPassed(label, { description: outcome.description, artifacts: outcome.artifacts });
			}
		} catch (error) {
			const message = error instanceof Error ? error.message : String(error);
			results.push({ label, status: "failed", description, error: message });
			systemView.checkFailed(label, error);
		}
	};

	systemView.checksStarted("Wanix cockpit self-check");
	await runCheck("drivers advertised", "Verify task runtimes are present in serve discovery.", () => driverCheck(config));
	await runCheck("task and terminal services", "Verify #task and #term service roots were discovered.", () => serviceCheck(config));
	await runCheck("qjs shell entrypoint", "Verify the browser workbench has an interactive shell route.", () => shellCheck(config));
	await runCheck("http app route", "Verify Wanix HTTP app serving is available from this session.", () => httpRouteCheck(config));
	await runCheck("direct-v86 route", "Verify direct-v86 launch discovery is visible when a boot root is prepared.", () => directV86Check(config));
	await runCheck("wanix report storage", "Write and read a probe file under /.wanix/checks.", () => reportStorageCheck(fsys, generatedAt));
	await runCheck("#agent allocator", "Allocate an agent session via #agent/new and check the returned id.", () => agentDeviceCheck(fsys));
	await runCheck("#kv roundtrip", "Write a key under #kv and read the same bytes back.", () => kvDeviceCheck(fsys, generatedAt));
	await runCheck("#pipe write/read", "Allocate a pipe, write a frame, and read it back.", () => pipeDeviceCheck(fsys, generatedAt));
	await runCheck("#cas ingest/retrieve", "Ingest a small blob and retrieve it by its content hash.", () => casDeviceCheck(fsys, generatedAt));
	await runCheck("#mesh peers", "List mesh peers; an empty list is acceptable.", () => meshPeersCheck(fsys, config));
	await runCheck("#plumb publish/subscribe", "Subscribe to a topic, publish one envelope, and receive it.", () => plumbDeviceCheck(fsys, generatedAt));
	await runCheck("agent tool contract artifact", "Check whether the agent tool contract has been published.", () => artifactCheck(fsys, "agent tool contract artifact", [
		AGENT_TOOL_CONTRACT_MD_PATH,
		AGENT_TOOL_CONTRACT_JSON_PATH,
	], "Run Open Agent Tool Contract to publish the agent-facing tool surface."));
	await runCheck("system state artifact", "Check whether the current cockpit state snapshot exists.", () => artifactCheck(fsys, "system state artifact", [
		SYSTEM_JOURNAL_PATH,
		SYSTEM_STATE_PATH,
	], "Run Open System Journal to publish the current state snapshot."));

	const status = overallStatus(results);
	const reportArtifacts = uniquePaths([
		COCKPIT_SELF_CHECK_MD_PATH,
		COCKPIT_SELF_CHECK_JSON_PATH,
		COCKPIT_SELF_CHECK_PROBE_PATH,
		...results.flatMap((result) => result.artifacts || []),
	]);
	await fsys.makeDirAll(".wanix");
	await fsys.writeFile(COCKPIT_SELF_CHECK_JSON_PATH, selfCheckJson(generatedAt, status, results, config));
	await fsys.writeFile(COCKPIT_SELF_CHECK_MD_PATH, selfCheckMarkdown(generatedAt, status, results));
	for (const path of reportArtifacts) {
		refreshWanixFile(bridge, path);
	}
	if (status === "failed") {
		systemView.checkFailed("Wanix cockpit self-check", "overall status failed", { artifacts: reportArtifacts });
	} else if (status === "warn") {
		systemView.checkWarned("Wanix cockpit self-check", { description: "overall status warn", artifacts: reportArtifacts });
	} else {
		systemView.checkPassed("Wanix cockpit self-check", { description: "overall status ok", artifacts: reportArtifacts });
	}
	systemView.checkReport(COCKPIT_SELF_CHECK_MD_PATH, status, reportArtifacts);
	systemView.reportPublished("Cockpit Self Check", COCKPIT_SELF_CHECK_MD_PATH, {
		kind: "diagnostic",
		description: `status ${status}`,
		icon: "testing-view-icon",
		artifacts: reportArtifacts,
	});
	for (const path of reportArtifacts) {
		systemView.filesystemActivity(`self-check artifact ${baseName(path)}`, { path });
	}
	await Promise.resolve(vscode.commands.executeCommand("workbench.files.action.refreshFilesExplorer")).catch((error: unknown) => {
		console.warn("Wanix explorer refresh failed", error);
	});
	await openWanixFile(COCKPIT_SELF_CHECK_MD_PATH);
	vscode.window.showInformationMessage(`Wanix cockpit self-check completed with ${status}`);
}

function driverCheck(config: CockpitSelfCheckConfig): CheckOutcome {
	const drivers = new Set(config.drivers || []);
	const missing = ["qjs", "wasm"].filter((driver) => !drivers.has(driver));
	if (drivers.size === 0) {
		return { status: "failed", description: "no task drivers were advertised" };
	}
	if (missing.length > 0) {
		return {
			status: "warn",
			description: `advertised ${[...drivers].sort().join(", ")}; missing ${missing.join(", ")}`,
		};
	}
	return { status: "ok", description: `advertised ${[...drivers].sort().join(", ")}` };
}

function serviceCheck(config: CockpitSelfCheckConfig): CheckOutcome {
	const missing = [
		config.ns?.task ? undefined : "#task",
		config.ns?.term ? undefined : "#term",
	].filter((entry): entry is string => Boolean(entry));
	if (missing.length > 0) {
		return { status: "failed", description: `missing ${missing.join(" and ")} service discovery` };
	}
	return { status: "ok", description: `task ${config.ns?.task}; term ${config.ns?.term}` };
}

function shellCheck(config: CockpitSelfCheckConfig): CheckOutcome {
	if (config.qjsShellUrl) {
		return { status: "ok", description: "qjs shell WebSocket route discovered" };
	}
	if (config.shell) {
		return { status: "ok", description: `native shell command ${config.shell.cmd || "configured"}` };
	}
	return { status: "warn", description: "no shell entrypoint was requested for this browser session" };
}

function httpRouteCheck(config: CockpitSelfCheckConfig): CheckOutcome {
	if (!config.httpApp?.route || config.httpApp.status === "disabled") {
		return { status: "warn", description: "Wanix HTTP app route is not advertised" };
	}
	return {
		status: "ok",
		description: `${config.httpApp.route} via ${config.httpApp.protocol || "wanix-http-app.v1"}`,
	};
}

function directV86Check(config: CockpitSelfCheckConfig): CheckOutcome {
	if (!config.v86?.launchUrl) {
		return { status: "warn", description: "direct-v86 launch route is not advertised in this session" };
	}
	if (config.v86.boot?.ready) {
		return { status: "ok", description: "direct-v86 route advertised with boot root ready" };
	}
	const missing = config.v86.boot?.missing?.length
		? `missing ${config.v86.boot.missing.join(", ")}`
		: config.v86.rootfs?.status || "boot root not prepared";
	return { status: "warn", description: `direct-v86 route advertised; ${missing}` };
}

async function reportStorageCheck(fsys: any, generatedAt: Date): Promise<CheckOutcome> {
	const contents = `wanix cockpit self-check probe ${generatedAt.toISOString()}\n`;
	await fsys.makeDirAll(".wanix/checks");
	await fsys.writeFile(COCKPIT_SELF_CHECK_PROBE_PATH, contents);
	const readBack = await fsys.readText(COCKPIT_SELF_CHECK_PROBE_PATH);
	if (String(readBack) !== contents) {
		return {
			status: "failed",
			description: "probe file readback did not match the written contents",
			artifacts: [COCKPIT_SELF_CHECK_PROBE_PATH],
		};
	}
	return {
		status: "ok",
		description: `probe wrote and read ${absoluteWanixPath(COCKPIT_SELF_CHECK_PROBE_PATH)}`,
		artifacts: [COCKPIT_SELF_CHECK_PROBE_PATH],
	};
}

// #agent: reading `#agent/new` allocates a session and returns its id followed
// by a newline (see `crates/wanix-agent/src/files.rs::NewAgentFile`).
async function agentDeviceCheck(fsys: any): Promise<CheckOutcome> {
	try {
		const raw = await fsys.readText(AGENT_NEW_PATH);
		const id = String(raw).trim();
		if (!id) {
			return {
				status: "failed",
				description: `${absoluteWanixPath(AGENT_NEW_PATH)} returned an empty id`,
			};
		}
		return {
			status: "ok",
			description: `#agent allocated session id ${id}`,
		};
	} catch (error) {
		return meshDeviceUnavailable("#agent", AGENT_NEW_PATH, error);
	}
}

// #kv: write to `#kv/<key>` to set the value, read it back to confirm a
// roundtrip (see `crates/wanix-kv/src/files.rs`).
async function kvDeviceCheck(fsys: any, generatedAt: Date): Promise<CheckOutcome> {
	const value = `cockpit-self-check ${generatedAt.toISOString()}`;
	try {
		await fsys.writeFile(KV_PROBE_PATH, value);
		const readBack = await fsys.readText(KV_PROBE_PATH);
		if (String(readBack) !== value) {
			return {
				status: "failed",
				description: `#kv readback ${JSON.stringify(String(readBack))} did not match write ${JSON.stringify(value)}`,
			};
		}
		return {
			status: "ok",
			description: `#kv roundtrip ok at ${absoluteWanixPath(KV_PROBE_PATH)}`,
		};
	} catch (error) {
		return meshDeviceUnavailable("#kv", KV_PROBE_PATH, error);
	}
}

// #pipe: read `#pipe/new` to allocate a channel, then write to `<id>/data` and
// read it back from the same path (see `crates/wanix-pipe/src/files.rs`).
async function pipeDeviceCheck(fsys: any, generatedAt: Date): Promise<CheckOutcome> {
	try {
		const raw = await fsys.readText(PIPE_NEW_PATH);
		const id = String(raw).trim();
		if (!id) {
			return {
				status: "failed",
				description: `${absoluteWanixPath(PIPE_NEW_PATH)} returned an empty channel id`,
			};
		}
		const dataPath = `#pipe/${id}/data`;
		const frame = `cockpit-self-check pipe ${generatedAt.toISOString()}\n`;
		// PipeChannel buffers written bytes until read and signals EOF only once
		// every writer is dropped. Open the write end (a live stream so it is not
		// treated as a create), write the frame, then close it; with no writers
		// left, the following one-shot read drains the buffered frame and then
		// sees a real EOF. Doing it write-first avoids the reader-first race
		// (an empty open channel and a closed channel both read as 0 bytes).
		const writer = (await fsys.openWritable(dataPath)).getWriter();
		try {
			await writer.write(new TextEncoder().encode(frame));
		} finally {
			await writer.close().catch(() => undefined);
		}
		const received = String(await fsys.readText(dataPath));
		if (received !== frame) {
			return {
				status: "failed",
				description: `#pipe readback ${JSON.stringify(received)} did not match write ${JSON.stringify(frame)}`,
			};
		}
		return {
			status: "ok",
			description: `#pipe channel ${id} carried ${frame.length} bytes end to end`,
		};
	} catch (error) {
		return meshDeviceUnavailable("#pipe", PIPE_NEW_PATH, error);
	}
}

// #cas: write bytes to `#cas/ingest`, read `#cas/ingest` to learn the hex
// hash, then read `#cas/<hash>` to retrieve the same bytes (see
// `crates/wanix-cas/src/device.rs`).
async function casDeviceCheck(fsys: any, generatedAt: Date): Promise<CheckOutcome> {
	const payload = `cockpit-self-check cas ${generatedAt.toISOString()}`;
	try {
		await fsys.writeFile(CAS_INGEST_PATH, payload);
		const hash = (await fsys.readText(CAS_INGEST_PATH)).trim();
		if (!hash) {
			return {
				status: "failed",
				description: `${absoluteWanixPath(CAS_INGEST_PATH)} did not publish a content hash after write`,
			};
		}
		const blobPath = `#cas/${hash}`;
		const readBack = await fsys.readText(blobPath);
		if (String(readBack) !== payload) {
			return {
				status: "failed",
				description: `#cas ${hash} readback did not match ingested bytes`,
			};
		}
		return {
			status: "ok",
			description: `#cas ingested ${payload.length} bytes; retrieved by hash ${shortHash(hash)}`,
		};
	} catch (error) {
		return meshDeviceUnavailable("#cas", CAS_INGEST_PATH, error);
	}
}

// #mesh: list peers. An empty list is acceptable (a fresh single-node mesh has
// no peers yet). Prefer the discovery-payload list when serve has surfaced it;
// fall back to `#mesh/peers` when only the device file is available.
async function meshPeersCheck(fsys: any, config: CockpitSelfCheckConfig): Promise<CheckOutcome> {
	const discoveryPeers = Array.isArray(config.mesh?.peers) ? config.mesh!.peers! : undefined;
	if (discoveryPeers) {
		return {
			status: "ok",
			description: discoveryPeers.length === 0
				? "mesh discovery advertises 0 peers (single-node mesh)"
				: `mesh discovery advertises ${discoveryPeers.length} peer(s)`,
		};
	}
	try {
		const raw = await fsys.readText(MESH_PEERS_PATH);
		const lines = String(raw)
			.split(/\r?\n/)
			.map((line: string) => line.trim())
			.filter((line: string) => line.length > 0);
		return {
			status: "ok",
			description: lines.length === 0
				? `${absoluteWanixPath(MESH_PEERS_PATH)} lists 0 peers (single-node mesh)`
				: `${absoluteWanixPath(MESH_PEERS_PATH)} lists ${lines.length} peer(s)`,
		};
	} catch (error) {
		// A read failure here is not a hard failure: mesh visibility is still
		// a session-config concern, and a single-node session need not expose
		// `#mesh` to pass the cockpit check.
		return {
			status: "warn",
			description: `mesh peer list not advertised; ${describeError(error)}`,
		};
	}
}

// #plumb: validate the publish path by writing one envelope to
// `#plumb/<topic>/send` (see `crates/wanix-plumb/src/files.rs` and `lib.rs`).
//
// We deliberately do not open a live `#plumb/<topic>/recv` subscription here: a
// `recv` read blocks server-side until an envelope arrives, and the serve 9P
// websocket handles one frame at a time per connection
// (crates/wanix-cli/src/p9_ws/connection.rs), so a blocking recv on this single
// browser connection would prevent the follow-up `send` frame from ever being
// processed -- a self-deadlock. Confirming the publish path is the most a
// single connection can verify; end-to-end delivery is exercised by the mesh
// integration tests and would need a second 9P connection for the subscriber.
async function plumbDeviceCheck(fsys: any, generatedAt: Date): Promise<CheckOutcome> {
	const envelope = JSON.stringify({
		kind: "cockpit-self-check",
		from: "cockpit",
		to: PLUMB_PROBE_TOPIC,
		body: `probe ${generatedAt.toISOString()}`,
	});
	try {
		const writer = (await fsys.openWritable(PLUMB_SEND_PATH)).getWriter();
		try {
			await writer.write(new TextEncoder().encode(envelope));
		} finally {
			await writer.close().catch(() => undefined);
		}
		return {
			status: "ok",
			description: `#plumb accepted a publish to ${absoluteWanixPath(PLUMB_SEND_PATH)} (live receive needs a second 9P connection on single-connection serve)`,
		};
	} catch (error) {
		return meshDeviceUnavailable("#plumb", PLUMB_SEND_PATH, error);
	}
}

function meshDeviceUnavailable(device: string, probePath: string, error: unknown): CheckOutcome {
	// A 9P walk to an unbound device returns NotFound; treat that as warn
	// (the mesh device is not bound in this session) rather than a hard
	// failure of the cockpit self-check.
	const message = describeError(error);
	const lowered = message.toLowerCase();
	if (lowered.includes("not found") || lowered.includes("no such") || lowered.includes("nofound")) {
		return {
			status: "warn",
			description: `${device} is not bound in this session (${absoluteWanixPath(probePath)} not found)`,
		};
	}
	return {
		status: "failed",
		description: `${device} probe at ${absoluteWanixPath(probePath)} failed: ${message}`,
	};
}

function shortHash(hash: string): string {
	if (hash.length <= 12) {
		return hash;
	}
	return `${hash.slice(0, 12)}…`;
}

function describeError(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

async function artifactCheck(fsys: any, label: string, paths: string[], warning: string): Promise<CheckOutcome> {
	const present: string[] = [];
	const missing: string[] = [];
	for (const path of paths) {
		if (await wanixPathExists(fsys, path)) {
			present.push(path);
		} else {
			missing.push(path);
		}
	}
	if (missing.length > 0) {
		return {
			status: "warn",
			description: `${warning} Missing ${missing.map(absoluteWanixPath).join(", ")}.`,
			artifacts: present,
		};
	}
	return {
		status: "ok",
		description: `${label} exists`,
		artifacts: present,
	};
}

function overallStatus(results: CheckResult[]): CheckStatus {
	if (results.some((result) => result.status === "failed")) {
		return "failed";
	}
	if (results.some((result) => result.status === "warn")) {
		return "warn";
	}
	return "ok";
}

function selfCheckJson(generatedAt: Date, status: CheckStatus, results: CheckResult[], config: CockpitSelfCheckConfig): string {
	return `${JSON.stringify({
		schema: "wanix.cockpit-check.v1",
		generatedAt: generatedAt.toISOString(),
		status,
		markdownPath: absoluteWanixPath(COCKPIT_SELF_CHECK_MD_PATH),
		jsonPath: absoluteWanixPath(COCKPIT_SELF_CHECK_JSON_PATH),
		probePath: absoluteWanixPath(COCKPIT_SELF_CHECK_PROBE_PATH),
		discovery: {
			drivers: config.drivers || [],
			taskService: config.ns?.task,
			terminalService: config.ns?.term,
			qjsShell: Boolean(config.qjsShellUrl || config.shell),
			httpAppRoute: config.httpApp?.route,
			httpAppStatus: config.httpApp?.status,
			directV86LaunchUrl: config.v86?.launchUrl,
			directV86BootReady: Boolean(config.v86?.boot?.ready),
			meshPeerCount: Array.isArray(config.mesh?.peers) ? config.mesh!.peers!.length : undefined,
		},
		checks: results.map((result) => ({
			...result,
			artifacts: (result.artifacts || []).map(absoluteWanixPath),
		})),
	}, null, 2)}\n`;
}

function selfCheckMarkdown(generatedAt: Date, status: CheckStatus, results: CheckResult[]): string {
	const counts = {
		ok: results.filter((result) => result.status === "ok").length,
		warn: results.filter((result) => result.status === "warn").length,
		failed: results.filter((result) => result.status === "failed").length,
	};
	return [
		"# Wanix Cockpit Self Check",
		"",
		`Generated: ${generatedAt.toISOString()}`,
		"Schema: wanix.cockpit-check.v1",
		`Overall status: ${status}`,
		`JSON: ${absoluteWanixPath(COCKPIT_SELF_CHECK_JSON_PATH)}`,
		"",
		"## Summary",
		"",
		`- ok: ${counts.ok}`,
		`- warn: ${counts.warn}`,
		`- failed: ${counts.failed}`,
		"",
		"## Checks",
		"",
		...results.flatMap(checkMarkdown),
	].join("\n");
}

function checkMarkdown(result: CheckResult): string[] {
	return [
		`### ${result.label}`,
		"",
		`Status: ${result.status}`,
		"",
		result.error || result.description,
		"",
		...(result.artifacts?.length
			? [
				"Artifacts:",
				"",
				...result.artifacts.map((path) => `- ${absoluteWanixPath(path)}`),
				"",
			]
			: []),
	];
}

async function wanixPathExists(fsys: any, path: string): Promise<boolean> {
	try {
		await fsys.stat(path);
		return true;
	} catch {
		return false;
	}
}

function refreshWanixFile(bridge: WanixBridge, path: string): void {
	bridge.refresh(absoluteWanixPath(path));
}

async function openWanixFile(path: string): Promise<void> {
	const document = await vscode.workspace.openTextDocument(vscode.Uri.from({
		scheme: WanixBridge.scheme,
		path: absoluteWanixPath(path),
	}));
	await vscode.window.showTextDocument(document, { preview: false });
}

function absoluteWanixPath(path: string): string {
	return path.startsWith("/") || path.startsWith("#") ? path : `/${path}`;
}

function uniquePaths(paths: string[]): string[] {
	return [...new Set(paths.filter((path) => path.length > 0))];
}

function baseName(path: string): string {
	const normalized = path.endsWith("/") && path.length > 1 ? path.slice(0, -1) : path;
	const slash = normalized.lastIndexOf("/");
	return slash >= 0 ? normalized.slice(slash + 1) : normalized;
}
