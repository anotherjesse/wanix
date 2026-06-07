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
export const HTTP_APP_CATALOG_MD_PATH = ".wanix/http-apps.md";
export const HTTP_APP_CATALOG_JSON_PATH = ".wanix/http-apps.json";

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
	dataStore?: {
		label: string;
		path: string;
		kind: string;
		description: string;
		routeLabel?: string;
	};
};

type HttpAppTrace = {
	taskId?: string;
	stdoutPath?: string;
	stderrPath?: string;
};

type HttpAppRuntime = "qjs" | "wasm";

export type HttpAppCatalogEntry = {
	name: string;
	runtime: HttpAppRuntime;
	sourcePath: string;
	routeLabel: string;
	previewPath?: string;
	url?: string;
};

export type HttpAppCatalogTarget = {
	name?: string;
	sourcePath?: string;
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
	dataStore: {
		label: "HTTP Counter State",
		path: COUNTER_STATE_PATH,
		kind: "stateful http",
		description: "counter backing file",
		routeLabel: `/.wanix/app/${COUNTER_NAME}`,
	},
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

export async function publishHttpAppDataStores(
	fsys: any,
	systemView: WanixSystemView,
): Promise<void> {
	if (!await wanixPathExists(fsys, COUNTER_STATE_PATH)) {
		return;
	}
	const artifacts = [COUNTER_STATE_PATH];
	if (await wanixPathExists(fsys, COUNTER_PREVIEW_PATH)) {
		artifacts.push(COUNTER_PREVIEW_PATH);
	}
	systemView.dataStorePublished("HTTP Counter State", COUNTER_STATE_PATH, {
		kind: "stateful http",
		description: "counter backing file",
		sourcePath: await wanixPathExists(fsys, COUNTER_PATH) ? COUNTER_PATH : undefined,
		routeLabel: `/.wanix/app/${COUNTER_NAME}`,
		artifacts,
	});
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

export async function createHttpApp(
	fsys: any,
	bridge: WanixBridge,
	config: HttpAppDemoConfig,
	systemView: WanixSystemView,
): Promise<void> {
	await fsys.makeDirAll(APP_DIR);
	const name = await nextHttpAppName(fsys);
	const path = `${APP_DIR}/${name}.js`;
	await fsys.writeFile(path, starterHttpAppSource(name));
	refreshWanixFile(bridge, path);
	await publishHttpAppCatalog(fsys, bridge, config, systemView);
	systemView.filesystemActivity(`http app ${name} created`, { path });
	await Promise.resolve(vscode.commands.executeCommand("workbench.files.action.refreshFilesExplorer")).catch((error: unknown) => {
		console.warn("Wanix explorer refresh failed", error);
	});
	await openWanixFile(path);
	vscode.window.showInformationMessage(`Created Wanix HTTP app ${name}`);
}

export async function openHttpAppCatalog(
	fsys: any,
	bridge: WanixBridge,
	config: HttpAppDemoConfig,
	systemView: WanixSystemView,
): Promise<void> {
	const entries = await publishHttpAppCatalog(fsys, bridge, config, systemView);
	await openWanixFile(`/${HTTP_APP_CATALOG_MD_PATH}`);
	vscode.window.showInformationMessage(`Opened Wanix HTTP app catalog with ${entries.length} app${entries.length === 1 ? "" : "s"}`);
}

export async function publishHttpAppCatalog(
	fsys: any,
	bridge: WanixBridge,
	config: HttpAppDemoConfig,
	systemView: WanixSystemView,
): Promise<HttpAppCatalogEntry[]> {
	const generatedAt = new Date();
	await fsys.makeDirAll(".wanix");
	const entries = await publishHttpAppsToSystemView(fsys, config, systemView);
	systemView.reportPublished("HTTP App Catalog", HTTP_APP_CATALOG_MD_PATH, {
		kind: "apps",
		description: "discovered Wanix HTTP programs",
		icon: "globe",
		artifacts: [HTTP_APP_CATALOG_MD_PATH, HTTP_APP_CATALOG_JSON_PATH],
	});
	await fsys.writeFile(HTTP_APP_CATALOG_JSON_PATH, httpAppCatalogJson(generatedAt, config, entries));
	await fsys.writeFile(HTTP_APP_CATALOG_MD_PATH, httpAppCatalogMarkdown(generatedAt, config, entries));
	refreshWanixFile(bridge, HTTP_APP_CATALOG_MD_PATH);
	refreshWanixFile(bridge, HTTP_APP_CATALOG_JSON_PATH);
	return entries;
}

export async function publishHttpAppsToSystemView(
	fsys: any,
	config: HttpAppDemoConfig,
	systemView: WanixSystemView,
): Promise<HttpAppCatalogEntry[]> {
	const entries = await discoverHttpApps(fsys, config);
	systemView.httpAppCatalogPublished(entries);
	return entries;
}

export async function previewHttpCatalogApp(
	fsys: any,
	bridge: WanixBridge,
	config: HttpAppDemoConfig,
	systemView: WanixSystemView,
	target?: HttpAppCatalogTarget,
): Promise<void> {
	const app = await resolveHttpAppTarget(fsys, config, target);
	await previewHttpApp(fsys, bridge, config, systemView, {
		name: app.name,
		sourcePath: app.sourcePath,
		previewPath: httpAppPreviewPath(app.name),
		message: `Previewed Wanix HTTP app ${app.name}`,
		errorLabel: `Wanix HTTP app ${app.name}`,
	});
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
	const runtime = httpAppRuntimeForPath(target.sourcePath);
	systemView.httpAppIndexed({
		name: target.name,
		runtime,
		sourcePath: target.sourcePath,
		routeLabel: `/.wanix/app/${target.name}`,
		previewPath: target.previewPath,
	});
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
	systemView.routePreviewed(httpAppRouteId(target.name), {
		status: response.status,
		statusText: response.statusText,
		previewPath: target.previewPath,
		sourcePath: target.sourcePath,
		url,
		label: `/.wanix/app/${target.name}`,
		artifacts,
	});
	if (target.dataStore) {
		systemView.dataStorePublished(target.dataStore.label, target.dataStore.path, {
			kind: target.dataStore.kind,
			description: target.dataStore.description,
			sourcePath: target.sourcePath,
			routeLabel: target.dataStore.routeLabel || `/.wanix/app/${target.name}`,
			artifacts: [target.dataStore.path, target.previewPath, ...artifacts.map((artifact) => artifact.path)],
		});
	}
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

async function discoverHttpApps(fsys: any, config: HttpAppDemoConfig): Promise<HttpAppCatalogEntry[]> {
	let rawEntries: unknown[] = [];
	try {
		rawEntries = typeof fsys.readDirEntries === "function"
			? await fsys.readDirEntries(APP_DIR)
			: await fsys.readDir(APP_DIR);
	} catch {
		return [];
	}
	const byName = new Map<string, HttpAppCatalogEntry>();
	for (const rawEntry of rawEntries) {
		const name = appEntryName(rawEntry);
		if (!name || name.endsWith("/")) {
			continue;
		}
		const runtime = httpAppRuntimeForFilename(name);
		if (!runtime) {
			continue;
		}
		const appName = name.replace(/\.(js|wasm)$/i, "");
		const sourcePath = `${APP_DIR}/${name}`;
		const previewPath = httpAppPreviewPath(appName);
		const entry = httpAppCatalogEntry(config, appName, runtime, sourcePath, await wanixPathExists(fsys, previewPath) ? previewPath : undefined);
		const existing = byName.get(appName);
		if (!existing || runtime === "qjs") {
			byName.set(appName, entry);
		}
	}
	return [...byName.values()].sort((left, right) => left.name.localeCompare(right.name, undefined, { numeric: true }));
}

async function resolveHttpAppTarget(
	fsys: any,
	config: HttpAppDemoConfig,
	target?: HttpAppCatalogTarget,
): Promise<HttpAppCatalogEntry> {
	const sourcePath = target?.sourcePath;
	if (sourcePath) {
		const name = target.name || appNameFromPath(sourcePath);
		const runtime = httpAppRuntimeForPath(sourcePath);
		return httpAppCatalogEntry(config, name, runtime, sourcePath);
	}
	if (target?.name) {
		const entries = await discoverHttpApps(fsys, config);
		const entry = entries.find((candidate) => candidate.name === target.name);
		if (entry) {
			return entry;
		}
	}
	throw new Error("No HTTP app handler is available for this route");
}

function httpAppCatalogEntry(config: HttpAppDemoConfig, name: string, runtime: HttpAppRuntime, sourcePath: string, previewPath?: string): HttpAppCatalogEntry {
	return {
		name,
		runtime,
		sourcePath,
		routeLabel: `/.wanix/app/${name}`,
		previewPath,
		url: optionalHttpAppUrl(config, name),
	};
}

function httpAppCatalogMarkdown(generatedAt: Date, config: HttpAppDemoConfig, entries: HttpAppCatalogEntry[]): string {
	return [
		"# Wanix HTTP Apps",
		"",
		`Generated: ${generatedAt.toISOString()}`,
		"Schema: wanix.http-apps.v1",
		`JSON: /${HTTP_APP_CATALOG_JSON_PATH}`,
		`Route Template: ${config.httpApp?.url || "unadvertised"}`,
		"",
		"## Apps",
		"",
		...(entries.length > 0
			? entries.flatMap(httpAppCatalogMarkdownLines)
			: ["- none"]),
	].join("\n");
}

function httpAppCatalogMarkdownLines(entry: HttpAppCatalogEntry): string[] {
	return [
		`- ${entry.name} (${entry.runtime}): ${entry.sourcePath}`,
		`  - route: ${entry.routeLabel}`,
		entry.previewPath ? `  - latest preview: ${entry.previewPath}` : `  - next preview writes: ${httpAppPreviewPath(entry.name)}`,
		entry.url ? `  - url: ${entry.url}` : undefined,
		"",
	].filter((line): line is string => line !== undefined);
}

function httpAppCatalogJson(generatedAt: Date, config: HttpAppDemoConfig, entries: HttpAppCatalogEntry[]): string {
	return `${JSON.stringify({
		schema: "wanix.http-apps.v1",
		generatedAt: generatedAt.toISOString(),
		markdownPath: `/${HTTP_APP_CATALOG_MD_PATH}`,
		jsonPath: `/${HTTP_APP_CATALOG_JSON_PATH}`,
		routeTemplate: config.httpApp?.url,
		appCount: entries.length,
		apps: entries,
	}, null, 2)}\n`;
}

function optionalHttpAppUrl(config: HttpAppDemoConfig, name: string): string | undefined {
	try {
		return httpAppDemoUrl(config, name);
	} catch {
		return undefined;
	}
}

async function nextHttpAppName(fsys: any): Promise<string> {
	for (let index = 1; index < 1000; index += 1) {
		const name = index === 1 ? "app" : `app-${index}`;
		if (!await wanixPathExists(fsys, `${APP_DIR}/${name}.js`) && !await wanixPathExists(fsys, `${APP_DIR}/${name}.wasm`)) {
			return name;
		}
	}
	throw new Error("Could not find a free /apps app name");
}

function starterHttpAppSource(name: string): string {
	return `import * as std from "qjs:std";

const app = std.getenv("WANIX_HTTP_APP") || ${JSON.stringify(name)};
const target = std.getenv("WANIX_HTTP_TARGET") || "/";

std.out.puts("wanix app " + app + "\\n");
std.out.puts("target=" + target + "\\n");
`;
}

function appEntryName(entry: unknown): string | undefined {
	if (typeof entry === "string") {
		return entry;
	}
	if (!entry || typeof entry !== "object") {
		return undefined;
	}
	const candidate = entry as { Name?: string; IsDir?: boolean };
	if (!candidate.Name) {
		return undefined;
	}
	return candidate.IsDir ? `${candidate.Name}/` : candidate.Name;
}

function appNameFromPath(path: string): string {
	const slash = path.lastIndexOf("/");
	const basename = slash >= 0 ? path.slice(slash + 1) : path;
	return basename.replace(/\.(js|wasm)$/i, "");
}

function httpAppPreviewPath(name: string): string {
	return `${APP_DIR}/${name}.response.txt`;
}

function httpAppRuntimeForFilename(name: string): HttpAppRuntime | undefined {
	if (/\.js$/i.test(name)) {
		return "qjs";
	}
	if (/\.wasm$/i.test(name)) {
		return "wasm";
	}
	return undefined;
}

function httpAppRuntimeForPath(path: string): HttpAppRuntime {
	return /\.wasm$/i.test(path) ? "wasm" : "qjs";
}

function httpAppRouteId(name: string): string {
	return `http-app:${name}`;
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
