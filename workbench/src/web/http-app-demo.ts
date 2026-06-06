import * as vscode from 'vscode';
import { WanixBridge } from './bridge.js';
import { WanixSystemView } from './system-view.js';

const APP_DIR = "/apps";
const APP_NAME = "hello";
const APP_PATH = `${APP_DIR}/${APP_NAME}.js`;
const APP_PREVIEW_PATH = `${APP_DIR}/${APP_NAME}.response.txt`;

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
	bridge.refresh();
	systemView.filesystemActivity("http app demo installed");
	await Promise.resolve(vscode.commands.executeCommand("workbench.files.action.refreshFilesExplorer")).catch((error: unknown) => {
		console.warn("Wanix explorer refresh failed", error);
	});
	if (openHandler) {
		await openWanixFile(APP_PATH);
	}
	if (notify) {
		vscode.window.showInformationMessage("Installed Wanix HTTP app demo in /apps");
	}
}

export async function openHttpAppDemo(
	fsys: any,
	bridge: WanixBridge,
	config: HttpAppDemoConfig,
	systemView: WanixSystemView,
): Promise<void> {
	await installHttpAppDemo(fsys, bridge, systemView, { openHandler: false, notify: false });
	const url = `${httpAppDemoUrl(config)}?from=workbench`;
	const response = await fetch(url, { cache: "no-store" });
	const body = await response.text();
	if (!response.ok) {
		throw new Error(`Wanix HTTP app returned ${response.status}: ${body}`);
	}
	await fsys.writeFile(APP_PREVIEW_PATH, body);
	bridge.refresh();
	systemView.filesystemActivity("http app demo previewed");
	await Promise.resolve(vscode.commands.executeCommand("workbench.files.action.refreshFilesExplorer")).catch((error: unknown) => {
		console.warn("Wanix explorer refresh failed", error);
	});
	await openWanixFile(APP_PREVIEW_PATH);
	vscode.window.showInformationMessage("Previewed Wanix HTTP app response in /apps");
}

function httpAppDemoUrl(config: HttpAppDemoConfig): string {
	const template = config.httpApp?.url;
	if (!template || config.httpApp?.status === "disabled") {
		throw new Error("Wanix HTTP app route was not advertised by serve");
	}
	return template.replace("{name}", encodeURIComponent(APP_NAME));
}

async function openWanixFile(path: string): Promise<void> {
	const document = await vscode.workspace.openTextDocument(vscode.Uri.from({
		scheme: WanixBridge.scheme,
		path,
	}));
	await vscode.window.showTextDocument(document, { preview: false });
}
