import * as vscode from 'vscode';
import { AGENT_TOOL_CONTRACT_JSON_PATH, AGENT_TOOL_CONTRACT_MD_PATH } from './agent-tool-contract.js';
import { WanixBridge } from './bridge.js';
import { WanixSystemView, type WanixSystemConfig } from './system-view.js';

export const COCKPIT_SELF_CHECK_MD_PATH = ".wanix/cockpit-check.md";
export const COCKPIT_SELF_CHECK_JSON_PATH = ".wanix/cockpit-check.json";

const COCKPIT_SELF_CHECK_PROBE_PATH = ".wanix/checks/probe.txt";
const SYSTEM_JOURNAL_PATH = ".wanix/system-journal.md";
const SYSTEM_STATE_PATH = ".wanix/system-state.json";

type CockpitSelfCheckConfig = WanixSystemConfig & {
	shell?: {
		cmd?: string;
		type?: string;
		wd?: string;
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
