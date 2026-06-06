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
): Promise<void> {
	await fsys.makeDirAll(DUET_DIR);
	await fsys.makeDirAll(SHARED_DIR);
	const wasm = await fetchWorkbenchAsset(context, WASM_ASSET);
	await fsys.writeFile(PRODUCER_PATH, PRODUCER_JS);
	await fsys.writeFile(VERIFY_PATH, VERIFY_JS);
	await fsys.writeFile(README_PATH, README_MD);
	await fsys.writeFile(WASM_PATH, wasm);
	bridge.refresh();
	systemView.filesystemActivity("duet demo installed");
	await Promise.resolve(vscode.commands.executeCommand("workbench.files.action.refreshFilesExplorer")).catch((error: unknown) => {
		console.warn("Wanix explorer refresh failed", error);
	});
	await Promise.resolve(vscode.commands.executeCommand("workbench.view.extension.wanix")).catch((error: unknown) => {
		console.warn("Wanix system view focus failed", error);
	});
	await openWanixFile(PRODUCER_PATH);
	vscode.window.showInformationMessage("Installed Wanix JS and WASM duet demo in /duet");
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
