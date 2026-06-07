import * as vscode from 'vscode';
import { WanixBridge } from './bridge.js';
import { WanixSystemView } from './system-view.js';
import { fetchWorkbenchAsset } from './workbench-assets.js';

const WASM_DIR = "/wasm";
const SHARED_DIR = `${WASM_DIR}/shared`;
const WASM_ASSET = "media/rust-guest.wasm";
const WASM_README_PATH = `${WASM_DIR}/README.md`;
export const WASM_STARTER_PATH = `${WASM_DIR}/starter.wasm`;
export const WASM_STARTER_INPUT_PATH = `${SHARED_DIR}/in.txt`;
export const WASM_STARTER_OUTPUT_PATH = `${SHARED_DIR}/out.txt`;

const STARTER_INPUT = "hello from Wanix wasm starter\n";

const README_MD = `# Wanix WASM Starter

This starter installs a compiled Rust WASI module and a shared input file.

Run starter.wasm as a wasm task. The module reads:

\`\`\`text
/shared/in.txt
\`\`\`

and writes:

\`\`\`text
/shared/out.txt
\`\`\`

The task starts in /wasm, so the guest's /shared directory is visible in Wanix
as /wasm/shared. Edit /wasm/shared/in.txt and run the wasm task again to see the
output change.
`;

export async function installWasmStarter(
	context: vscode.ExtensionContext,
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
	options: { openReadme?: boolean; notify?: boolean } = {},
): Promise<void> {
	const openReadme = options.openReadme ?? true;
	const notify = options.notify ?? true;
	const installed = await ensureWasmStarter(context, fsys, bridge);
	systemView.filesystemActivity(installed ? "wasm starter installed" : "wasm starter opened");
	await refreshWorkbenchFiles();
	if (openReadme) {
		await openWanixFile(WASM_README_PATH);
	}
	if (notify) {
		const message = installed
			? "Installed Wanix WASM starter in /wasm"
			: "Opened Wanix WASM starter in /wasm";
		vscode.window.showInformationMessage(message);
	}
}

export async function ensureWasmStarter(
	context: vscode.ExtensionContext,
	fsys: any,
	bridge: WanixBridge,
): Promise<boolean> {
	let installed = false;
	await fsys.makeDirAll(WASM_DIR);
	await fsys.makeDirAll(SHARED_DIR);
	installed = await writeTextIfMissing(fsys, WASM_README_PATH, README_MD) || installed;
	installed = await writeTextIfMissing(fsys, WASM_STARTER_INPUT_PATH, STARTER_INPUT) || installed;
	if (!await wanixPathExists(fsys, WASM_STARTER_PATH)) {
		await fsys.writeFile(WASM_STARTER_PATH, await fetchWorkbenchAsset(context, WASM_ASSET));
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

async function refreshWorkbenchFiles(): Promise<void> {
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
