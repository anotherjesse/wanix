import * as vscode from 'vscode';
import { WanixBridge } from './bridge.js';
import { WanixSystemView } from './system-view.js';

const AGENT_DIR = "/agent";
const AGENT_OUT_DIR = `${AGENT_DIR}/out`;
const VALUE_DECLARATION = `const missingValue = "fixed by Wanix agent";`;

export const AGENT_BROKEN_PATH = `${AGENT_DIR}/broken.js`;
export const AGENT_RESULT_PATH = `${AGENT_OUT_DIR}/result.txt`;

const BROKEN_JS = `import * as std from "qjs:std";
import * as os from "qjs:os";

os.mkdir("out", 0o777);
if (typeof missingValue === "undefined") {
  std.err.puts("ReferenceError: missingValue is not defined\\n");
  std.exit(1);
}
std.writeFile("out/result.txt", missingValue.toUpperCase() + "\\n");
std.out.puts("wrote out/result.txt\\n");
`;

export async function installAgentRepairDemo(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
): Promise<void> {
	await fsys.makeDirAll(AGENT_DIR);
	await removeIfExists(fsys, AGENT_RESULT_PATH);
	await fsys.writeFile(AGENT_BROKEN_PATH, BROKEN_JS);
	bridge.refresh(AGENT_DIR);
	bridge.refresh(AGENT_BROKEN_PATH);
	systemView.filesystemActivity("agent repair demo installed");
	await Promise.resolve(vscode.commands.executeCommand("workbench.files.action.refreshFilesExplorer")).catch((error: unknown) => {
		console.warn("Wanix explorer refresh failed", error);
	});
	await openWanixFile(AGENT_BROKEN_PATH);
	vscode.window.showInformationMessage("Installed Wanix agent repair demo in /agent");
}

export function repairQjsProgram(source: string): string {
	if (source.includes(VALUE_DECLARATION)) {
		return source;
	}
	if (!source.includes("missingValue.toUpperCase()")) {
		throw new Error("Deterministic repair only knows how to fix missingValue.toUpperCase() demos");
	}
	const lines = source.split("\n");
	let insertAt = 0;
	while (insertAt < lines.length && lines[insertAt].startsWith("import ")) {
		insertAt += 1;
	}
	if (lines[insertAt] === "") {
		insertAt += 1;
	}
	lines.splice(insertAt, 0, VALUE_DECLARATION);
	return lines.join("\n");
}

async function removeIfExists(fsys: any, path: string): Promise<void> {
	try {
		await fsys.remove(path);
	} catch {
		// The demo reset path should tolerate first use.
	}
}

async function openWanixFile(path: string): Promise<void> {
	const document = await vscode.workspace.openTextDocument(vscode.Uri.from({
		scheme: WanixBridge.scheme,
		path,
	}));
	await vscode.window.showTextDocument(document, { preview: false });
}
