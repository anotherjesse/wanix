import * as vscode from 'vscode';
import { WanixBridge } from './bridge.js';
import { WanixSystemView } from './system-view.js';

const APP_DIR = "/apps";
const APP_NAME = "hello";
const APP_PATH = `${APP_DIR}/${APP_NAME}.js`;
const APP_PREVIEW_PATH = `${APP_DIR}/${APP_NAME}.response.txt`;
const COUNTER_NAME = "counter";
const COUNTER_PATH = `${APP_DIR}/${COUNTER_NAME}.js`;
const COUNTER_PREVIEW_PATH = `${APP_DIR}/${COUNTER_NAME}.response.txt`;
const COUNTER_STATE_PATH = `${APP_DIR}/${COUNTER_NAME}.count.txt`;
const WASM_ROUTE_NAME = "wasm";
const WASM_ROUTE_PATH = `${APP_DIR}/${WASM_ROUTE_NAME}.wasm`;
const WASM_ROUTE_PREVIEW_PATH = `${APP_DIR}/${WASM_ROUTE_NAME}.response.txt`;
const WASM_ROUTE_BYTES_BASE64 = "AGFzbQEAAAABDAJgBH9/f38Bf2AAAAIjARZ3YXNpX3NuYXBzaG90X3ByZXZpZXcxCGZkX3dyaXRlAAADAgEBBQMBAAEHEwIGbWVtb3J5AgAGX3N0YXJ0AAEKHQEbAEEAQQg2AgBBBEEONgIAQQFBAEEBQRgQABoLCxQBAEEICw53YXNtIHJvdXRlIG9rCg==";

export type HttpAppRouteConfig = {
	url?: string;
	status?: string;
};

export type HttpAppDemoConfig = {
	discoveryUrl?: string;
	httpApp?: HttpAppRouteConfig;
};

type HttpAppPreviewTarget = {
	name: string;
	sourcePath: string;
	previewPath: string;
	message: string;
	errorLabel: string;
	artifacts?: Array<{ label: string; path: string; icon?: string }>;
};

type HttpAppTrace = {
	taskId?: string;
	stdoutPath?: string;
	stderrPath?: string;
};

const HELLO_PREVIEW: HttpAppPreviewTarget = {
	name: APP_NAME,
	sourcePath: APP_PATH,
	previewPath: APP_PREVIEW_PATH,
	message: "Previewed Wanix HTTP app response in /apps",
	errorLabel: "Wanix HTTP app",
};

const COUNTER_PREVIEW: HttpAppPreviewTarget = {
	name: COUNTER_NAME,
	sourcePath: COUNTER_PATH,
	previewPath: COUNTER_PREVIEW_PATH,
	message: "Previewed Wanix HTTP counter response in /apps",
	errorLabel: "Wanix HTTP counter",
	artifacts: [{ label: "State File", path: COUNTER_STATE_PATH, icon: "database" }],
};

const WASM_ROUTE_PREVIEW: HttpAppPreviewTarget = {
	name: WASM_ROUTE_NAME,
	sourcePath: WASM_ROUTE_PATH,
	previewPath: WASM_ROUTE_PREVIEW_PATH,
	message: "Previewed Wanix HTTP WASM response in /apps",
	errorLabel: "Wanix HTTP WASM app",
};

const APP_JS = `import * as std from "qjs:std";

const app = std.getenv("WANIX_HTTP_APP");
const target = std.getenv("WANIX_HTTP_TARGET");

std.out.puts("wanix http app " + app + " saw " + target + "\\n");
`;

const COUNTER_JS = `import * as std from "qjs:std";

const target = std.getenv("WANIX_HTTP_TARGET");
const countPath = "counter.count.txt";
let count = 0;

try {
  const previous = std.loadFile(countPath).trim();
  const parsed = parseInt(previous, 10);
  if (!isNaN(parsed)) {
    count = parsed;
  }
} catch (_) {
}

count += 1;
std.writeFile(countPath, String(count) + "\\n");
std.out.puts("counter=" + count + "\\n");
std.out.puts("target=" + target + "\\n");
`;

export async function installHttpAppDemo(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
	options: { openHandler?: boolean; notify?: boolean } = {},
): Promise<void> {
	const openHandler = options.openHandler ?? true;
	const notify = options.notify ?? true;
	await fsys.makeDirAll(APP_DIR);
	await fsys.writeFile(APP_PATH, APP_JS);
	refreshWanixFile(bridge, APP_PATH);
	systemView.filesystemActivity("http app demo reset");
	await Promise.resolve(vscode.commands.executeCommand("workbench.files.action.refreshFilesExplorer")).catch((error: unknown) => {
		console.warn("Wanix explorer refresh failed", error);
	});
	if (openHandler) {
		await openWanixFile(APP_PATH);
	}
	if (notify) {
		vscode.window.showInformationMessage("Reset Wanix HTTP app demo in /apps");
	}
}

async function ensureHttpAppDemo(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
): Promise<void> {
	await fsys.makeDirAll(APP_DIR);
	if (await wanixPathExists(fsys, APP_PATH)) {
		return;
	}
	await fsys.writeFile(APP_PATH, APP_JS);
	refreshWanixFile(bridge, APP_PATH);
	systemView.filesystemActivity("http app demo installed");
}

async function ensureHttpCounterDemo(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
): Promise<void> {
	let installed = false;
	await fsys.makeDirAll(APP_DIR);
	if (!await wanixPathExists(fsys, COUNTER_PATH)) {
		await fsys.writeFile(COUNTER_PATH, COUNTER_JS);
		installed = true;
		refreshWanixFile(bridge, COUNTER_PATH);
	}
	if (!await wanixPathExists(fsys, COUNTER_STATE_PATH)) {
		await fsys.writeFile(COUNTER_STATE_PATH, "0\n");
		installed = true;
		refreshWanixFile(bridge, COUNTER_STATE_PATH);
	}
	if (installed) {
		systemView.filesystemActivity("http counter demo installed");
	}
}

async function ensureHttpWasmDemo(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
): Promise<void> {
	await fsys.makeDirAll(APP_DIR);
	if (await wanixPathExists(fsys, WASM_ROUTE_PATH)) {
		return;
	}
	await fsys.writeFile(WASM_ROUTE_PATH, base64Bytes(WASM_ROUTE_BYTES_BASE64));
	refreshWanixFile(bridge, WASM_ROUTE_PATH);
	systemView.filesystemActivity("http wasm demo installed");
}

export async function openHttpAppHandler(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
): Promise<void> {
	await ensureHttpAppDemo(fsys, bridge, systemView);
	systemView.filesystemActivity("http app handler opened");
	await openWanixFile(APP_PATH);
}

export async function copyHttpAppUrl(
	config: HttpAppDemoConfig,
	systemView: WanixSystemView,
): Promise<void> {
	const url = httpAppDemoUrl(config);
	await vscode.env.clipboard.writeText(url);
	systemView.filesystemActivity("http app url copied");
	vscode.window.showInformationMessage("Copied Wanix HTTP app URL");
}

export async function openHttpAppDemo(
	fsys: any,
	bridge: WanixBridge,
	config: HttpAppDemoConfig,
	systemView: WanixSystemView,
): Promise<void> {
	await ensureHttpAppDemo(fsys, bridge, systemView);
	await previewHttpApp(fsys, bridge, config, systemView, HELLO_PREVIEW);
}

export async function openHttpCounterDemo(
	fsys: any,
	bridge: WanixBridge,
	config: HttpAppDemoConfig,
	systemView: WanixSystemView,
): Promise<void> {
	await ensureHttpCounterDemo(fsys, bridge, systemView);
	await previewHttpApp(fsys, bridge, config, systemView, COUNTER_PREVIEW);
}

export async function openHttpWasmDemo(
	fsys: any,
	bridge: WanixBridge,
	config: HttpAppDemoConfig,
	systemView: WanixSystemView,
): Promise<void> {
	await ensureHttpWasmDemo(fsys, bridge, systemView);
	await previewHttpApp(fsys, bridge, config, systemView, WASM_ROUTE_PREVIEW);
}

async function previewHttpApp(
	fsys: any,
	bridge: WanixBridge,
	config: HttpAppDemoConfig,
	systemView: WanixSystemView,
	target: HttpAppPreviewTarget,
): Promise<void> {
	const url = `${httpAppDemoUrl(config, target.name)}?from=workbench`;
	const response = await fetch(url, { cache: "no-store" });
	const body = await response.text();
	const trace = httpAppTrace(response);
	const artifacts = [
		...(target.artifacts || []),
		...httpAppTraceArtifacts(trace),
	];
	const preview = httpAppPreviewReport({
		url,
		status: response.status,
		statusText: response.statusText,
		contentType: response.headers.get("content-type"),
		trace,
		body,
		generatedAt: new Date().toISOString(),
	});
	await fsys.writeFile(target.previewPath, preview);
	refreshWanixFile(bridge, target.previewPath);
	for (const artifact of artifacts) {
		refreshWanixFile(bridge, artifact.path);
	}
	systemView.routePreviewed("http-app", {
		status: response.status,
		statusText: response.statusText,
		previewPath: target.previewPath,
		sourcePath: target.sourcePath,
		url,
		label: `/.wanix/app/${target.name}`,
		artifacts,
	});
	await Promise.resolve(vscode.commands.executeCommand("workbench.files.action.refreshFilesExplorer")).catch((error: unknown) => {
		console.warn("Wanix explorer refresh failed", error);
	});
	await openWanixFile(target.previewPath);
	if (!response.ok) {
		throw new Error(`${target.errorLabel} returned ${response.status}; preview saved to ${target.previewPath}`);
	}
	vscode.window.showInformationMessage(target.message);
}

function httpAppDemoUrl(config: HttpAppDemoConfig, name = APP_NAME): string {
	const template = config.httpApp?.url;
	if (!template || config.httpApp?.status === "disabled") {
		throw new Error("Wanix HTTP app route was not advertised by serve");
	}
	return template.replace("{name}", encodeURIComponent(name));
}

function refreshWanixFile(bridge: WanixBridge, path: string): void {
	bridge.refresh(path);
	bridge.refresh(APP_DIR);
}

async function wanixPathExists(fsys: any, path: string): Promise<boolean> {
	try {
		await fsys.stat(path);
		return true;
	} catch {
		return false;
	}
}

function base64Bytes(value: string): Uint8Array {
	const binary = atob(value);
	const bytes = new Uint8Array(binary.length);
	for (let index = 0; index < binary.length; index += 1) {
		bytes[index] = binary.charCodeAt(index);
	}
	return bytes;
}

function httpAppTrace(response: Response): HttpAppTrace {
	return {
		taskId: nonEmptyHeader(response, "x-wanix-task-id"),
		stdoutPath: nonEmptyHeader(response, "x-wanix-stdout-path"),
		stderrPath: nonEmptyHeader(response, "x-wanix-stderr-path"),
	};
}

function nonEmptyHeader(response: Response, name: string): string | undefined {
	return response.headers.get(name)?.trim() || undefined;
}

function httpAppTraceArtifacts(trace: HttpAppTrace): Array<{ label: string; path: string; icon?: string }> {
	const artifacts: Array<{ label: string; path: string; icon?: string }> = [];
	if (trace.stdoutPath) {
		artifacts.push({ label: "Task Stdout", path: trace.stdoutPath, icon: "output" });
	}
	if (trace.stderrPath) {
		artifacts.push({ label: "Task Stderr", path: trace.stderrPath, icon: "warning" });
	}
	return artifacts;
}

function httpAppPreviewReport(preview: {
	url: string;
	status: number;
	statusText: string;
	contentType: string | null;
	trace: HttpAppTrace;
	body: string;
	generatedAt: string;
}): string {
	const statusText = preview.statusText ? ` ${preview.statusText}` : "";
	const contentType = preview.contentType || "unknown";
	const lines = [
		"Wanix HTTP app preview",
		`URL: ${preview.url}`,
		`Status: ${preview.status}${statusText}`,
		`Content-Type: ${contentType}`,
		`Generated: ${preview.generatedAt}`,
	];
	if (preview.trace.taskId || preview.trace.stdoutPath || preview.trace.stderrPath) {
		lines.push(
			`Task: ${preview.trace.taskId || "unknown"}`,
			`Stdout Trace: ${preview.trace.stdoutPath || "unknown"}`,
			`Stderr Trace: ${preview.trace.stderrPath || "unknown"}`,
		);
	}
	lines.push(
		"",
		"Body:",
		preview.body,
	);
	return lines.join("\n");
}

async function openWanixFile(path: string): Promise<void> {
	const document = await vscode.workspace.openTextDocument(vscode.Uri.from({
		scheme: WanixBridge.scheme,
		path,
	}));
	await vscode.window.showTextDocument(document, { preview: false });
}
