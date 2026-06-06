import * as vscode from 'vscode';
import { WanixBridge } from './bridge.js';
import { WanixSystemView } from './system-view.js';

const SCRATCH_DIR = "/scratch";
const SCRIPT_PREFIX = "qjs";

const STARTER_JS = `import * as std from "qjs:std";

const message = "hello from Wanix qjs";

std.out.puts(message + "\\n");
std.writeFile("last-run.txt", message + "\\n");
`;

export async function createQjsStarter(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
): Promise<void> {
	await fsys.makeDirAll(SCRATCH_DIR);
	const path = await nextStarterPath(fsys);
	await fsys.writeFile(path, STARTER_JS);
	bridge.refresh(path);
	bridge.refresh(SCRATCH_DIR);
	systemView.filesystemActivity("qjs starter created");
	await Promise.resolve(vscode.commands.executeCommand("workbench.files.action.refreshFilesExplorer")).catch((error: unknown) => {
		console.warn("Wanix explorer refresh failed", error);
	});
	await openWanixFile(path);
	vscode.window.showInformationMessage(`Created ${path}`);
}

async function nextStarterPath(fsys: any): Promise<string> {
	for (let index = 1; index < 1000; index += 1) {
		const path = `${SCRATCH_DIR}/${SCRIPT_PREFIX}-${index}.js`;
		if (!(await wanixPathExists(fsys, path))) {
			return path;
		}
	}
	throw new Error("Could not find an available scratch qjs script path");
}

async function wanixPathExists(fsys: any, path: string): Promise<boolean> {
	try {
		await fsys.stat(path);
		return true;
	} catch {
		return false;
	}
}

async function openWanixFile(path: string): Promise<void> {
	const document = await vscode.workspace.openTextDocument(vscode.Uri.from({
		scheme: WanixBridge.scheme,
		path,
	}));
	await vscode.window.showTextDocument(document, { preview: false });
}
