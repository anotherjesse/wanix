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

export type HttpAppRouteConfig = {
	url?: string;
	status?: string;
};

export type HttpAppDemoConfig = {
	discoveryUrl?: string;
	httpApp?: HttpAppRouteConfig;
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
	const url = `${httpAppDemoUrl(config)}?from=workbench`;
	const response = await fetch(url, { cache: "no-store" });
	const body = await response.text();
	const preview = httpAppPreviewReport({
		url,
		status: response.status,
		statusText: response.statusText,
		contentType: response.headers.get("content-type"),
		body,
		generatedAt: new Date().toISOString(),
	});
	await fsys.writeFile(APP_PREVIEW_PATH, preview);
	refreshWanixFile(bridge, APP_PREVIEW_PATH);
	systemView.routePreviewed("http-app", {
		status: response.status,
		statusText: response.statusText,
		previewPath: APP_PREVIEW_PATH,
		sourcePath: APP_PATH,
		url,
	});
	await Promise.resolve(vscode.commands.executeCommand("workbench.files.action.refreshFilesExplorer")).catch((error: unknown) => {
		console.warn("Wanix explorer refresh failed", error);
	});
	await openWanixFile(APP_PREVIEW_PATH);
	if (!response.ok) {
		throw new Error(`Wanix HTTP app returned ${response.status}; preview saved to ${APP_PREVIEW_PATH}`);
	}
	vscode.window.showInformationMessage("Previewed Wanix HTTP app response in /apps");
}

export async function openHttpCounterDemo(
	fsys: any,
	bridge: WanixBridge,
	config: HttpAppDemoConfig,
	systemView: WanixSystemView,
): Promise<void> {
	await ensureHttpCounterDemo(fsys, bridge, systemView);
	const url = `${httpAppDemoUrl(config, COUNTER_NAME)}?from=workbench`;
	const response = await fetch(url, { cache: "no-store" });
	const body = await response.text();
	const preview = httpAppPreviewReport({
		url,
		status: response.status,
		statusText: response.statusText,
		contentType: response.headers.get("content-type"),
		body,
		generatedAt: new Date().toISOString(),
	});
	await fsys.writeFile(COUNTER_PREVIEW_PATH, preview);
	refreshWanixFile(bridge, COUNTER_PREVIEW_PATH);
	refreshWanixFile(bridge, COUNTER_STATE_PATH);
	systemView.routePreviewed("http-app", {
		status: response.status,
		statusText: response.statusText,
		previewPath: COUNTER_PREVIEW_PATH,
		sourcePath: COUNTER_PATH,
		url,
	});
	await Promise.resolve(vscode.commands.executeCommand("workbench.files.action.refreshFilesExplorer")).catch((error: unknown) => {
		console.warn("Wanix explorer refresh failed", error);
	});
	await openWanixFile(COUNTER_PREVIEW_PATH);
	if (!response.ok) {
		throw new Error(`Wanix HTTP counter returned ${response.status}; preview saved to ${COUNTER_PREVIEW_PATH}`);
	}
	vscode.window.showInformationMessage("Previewed Wanix HTTP counter response in /apps");
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

function httpAppPreviewReport(preview: {
	url: string;
	status: number;
	statusText: string;
	contentType: string | null;
	body: string;
	generatedAt: string;
}): string {
	const statusText = preview.statusText ? ` ${preview.statusText}` : "";
	const contentType = preview.contentType || "unknown";
	return [
		"Wanix HTTP app preview",
		`URL: ${preview.url}`,
		`Status: ${preview.status}${statusText}`,
		`Content-Type: ${contentType}`,
		`Generated: ${preview.generatedAt}`,
		"",
		"Body:",
		preview.body,
	].join("\n");
}

async function openWanixFile(path: string): Promise<void> {
	const document = await vscode.workspace.openTextDocument(vscode.Uri.from({
		scheme: WanixBridge.scheme,
		path,
	}));
	await vscode.window.showTextDocument(document, { preview: false });
}
