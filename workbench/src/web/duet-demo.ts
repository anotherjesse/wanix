import * as vscode from 'vscode';
import { WanixBridge } from './bridge.js';
import { WanixSystemView } from './system-view.js';

const DUET_DIR = "/duet";
const SHARED_DIR = `${DUET_DIR}/shared`;
const PRODUCER_PATH = `${DUET_DIR}/producer.js`;
const VERIFY_PATH = `${DUET_DIR}/verify.js`;
const WASM_PATH = `${DUET_DIR}/transform.wasm`;
const README_PATH = `${DUET_DIR}/README.md`;
const WASM_ASSET = "media/rust-guest.wasm";

export const DUET_OUTPUT_PATH = `${SHARED_DIR}/out.txt`;

export type DuetDemoStep = {
	kind: "qjs" | "wasm";
	path: string;
	label: string;
};

export const DUET_DEMO_STEPS: DuetDemoStep[] = [
	{ kind: "qjs", path: PRODUCER_PATH, label: "producer" },
	{ kind: "wasm", path: WASM_PATH, label: "transform" },
	{ kind: "qjs", path: VERIFY_PATH, label: "verify" },
];

const GENERATED_SHARED_PATHS = [
	`${SHARED_DIR}/in.txt`,
	DUET_OUTPUT_PATH,
];

const PRODUCER_JS = `import * as std from "qjs:std";

const message = "hello from qjs duet";
std.writeFile("shared/in.txt", message + "\\n");
std.out.puts("producer wrote shared/in.txt\\n");
std.out.puts(message + "\\n");
`;

const VERIFY_JS = `import * as std from "qjs:std";

const expected = "rust-wasm saw: hello from qjs duet";
const actual = std.loadFile("shared/out.txt").trim();

if (actual !== expected) {
  std.err.puts("expected: " + expected + "\\n");
  std.err.puts("actual:   " + actual + "\\n");
  std.exit(1);
}

std.out.puts("verified shared/out.txt\\n");
std.out.puts(actual + "\\n");
`;

const README_MD = `# Wanix JS And WASM Duet

This demo proves that qjs and compiled WASM tasks share one Wanix namespace.

1. Run producer.js as a qjs task.
2. Run transform.wasm as a wasm task from Explorer.
3. Run verify.js as a qjs task.

producer.js writes shared/in.txt.
transform.wasm reads shared/in.txt and writes shared/out.txt.
verify.js checks the wasm output.
`;

export async function installDuetDemo(
	context: vscode.ExtensionContext,
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
	options: { openProducer?: boolean; notify?: boolean } = {},
): Promise<void> {
	const openProducer = options.openProducer ?? true;
	const notify = options.notify ?? true;
	const installed = await ensureDuetDemo(context, fsys, bridge);
	systemView.filesystemActivity(installed ? "duet demo installed" : "duet demo opened");
	await refreshWorkbenchViews();
	if (openProducer) {
		await openWanixFile(PRODUCER_PATH);
	}
	if (notify) {
		const message = installed
			? "Installed Wanix JS and WASM duet demo in /duet"
			: "Opened Wanix JS and WASM duet demo in /duet";
		vscode.window.showInformationMessage(message);
	}
}

export async function resetDuetDemo(
	context: vscode.ExtensionContext,
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
	options: { openProducer?: boolean; notify?: boolean } = {},
): Promise<void> {
	const openProducer = options.openProducer ?? true;
	const notify = options.notify ?? true;
	await fsys.makeDirAll(DUET_DIR);
	await fsys.makeDirAll(SHARED_DIR);
	const wasm = await fetchWorkbenchAsset(context, WASM_ASSET);
	await fsys.writeFile(PRODUCER_PATH, PRODUCER_JS);
	await fsys.writeFile(VERIFY_PATH, VERIFY_JS);
	await fsys.writeFile(README_PATH, README_MD);
	await fsys.writeFile(WASM_PATH, wasm);
	for (const path of GENERATED_SHARED_PATHS) {
		await removeWanixPathIfExists(fsys, path);
	}
	bridge.refresh();
	systemView.filesystemActivity("duet demo reset");
	await refreshWorkbenchViews();
	if (openProducer) {
		await openWanixFile(PRODUCER_PATH);
	}
	if (notify) {
		vscode.window.showInformationMessage("Reset Wanix JS and WASM duet demo in /duet");
	}
}

async function ensureDuetDemo(
	context: vscode.ExtensionContext,
	fsys: any,
	bridge: WanixBridge,
): Promise<boolean> {
	let installed = false;
	await fsys.makeDirAll(DUET_DIR);
	await fsys.makeDirAll(SHARED_DIR);
	installed = await writeTextIfMissing(fsys, PRODUCER_PATH, PRODUCER_JS) || installed;
	installed = await writeTextIfMissing(fsys, VERIFY_PATH, VERIFY_JS) || installed;
	installed = await writeTextIfMissing(fsys, README_PATH, README_MD) || installed;
	if (!await wanixPathExists(fsys, WASM_PATH)) {
		const wasm = await fetchWorkbenchAsset(context, WASM_ASSET);
		await fsys.writeFile(WASM_PATH, wasm);
		installed = true;
	}
	if (installed) {
		bridge.refresh();
	}
	return installed;
}

async function writeTextIfMissing(fsys: any, path: string, contents: string): Promise<boolean> {
	if (await wanixPathExists(fsys, path)) {
		return false;
	}
	await fsys.writeFile(path, contents);
	return true;
}

async function wanixPathExists(fsys: any, path: string): Promise<boolean> {
	try {
		await fsys.stat(path);
		return true;
	} catch {
		return false;
	}
}

async function removeWanixPathIfExists(fsys: any, path: string): Promise<void> {
	if (!await wanixPathExists(fsys, path)) {
		return;
	}
	await fsys.remove(path);
}

async function refreshWorkbenchViews(): Promise<void> {
	await Promise.resolve(vscode.commands.executeCommand("workbench.files.action.refreshFilesExplorer")).catch((error: unknown) => {
		console.warn("Wanix explorer refresh failed", error);
	});
	await Promise.resolve(vscode.commands.executeCommand("workbench.view.extension.wanix")).catch((error: unknown) => {
		console.warn("Wanix system view focus failed", error);
	});
}

async function openWanixFile(path: string): Promise<void> {
	const document = await vscode.workspace.openTextDocument(vscode.Uri.from({
		scheme: WanixBridge.scheme,
		path,
	}));
	await vscode.window.showTextDocument(document, { preview: false });
}

async function fetchWorkbenchAsset(context: vscode.ExtensionContext, assetPath: string): Promise<Uint8Array> {
	const urls = assetUrls(context, assetPath);
	let lastError: unknown;
	for (const url of urls) {
		try {
			const response = await fetch(url, { cache: "no-store" });
			if (!response.ok) {
				lastError = new Error(`${url} returned HTTP ${response.status}`);
				continue;
			}
			return new Uint8Array(await response.arrayBuffer());
		} catch (error) {
			lastError = error;
		}
	}
	throw new Error(`Could not load ${assetPath}: ${lastError instanceof Error ? lastError.message : String(lastError)}`);
}

function assetUrls(context: vscode.ExtensionContext, assetPath: string): string[] {
	const base = context.extensionUri.toString();
	const slashBase = base.endsWith("/") ? base : `${base}/`;
	return [...new Set([
		new URL(assetPath, slashBase).toString(),
		new URL(`/workbench/${assetPath}`, base).toString(),
	])];
}
