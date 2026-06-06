import * as vscode from 'vscode';
import { WanixBridge } from './bridge.js';

export const WANIX_INSPECT_SCHEME = "wanix-inspect";

export class WanixServiceInspector implements vscode.TextDocumentContentProvider, vscode.Disposable {
	private readonly emitter = new vscode.EventEmitter<vscode.Uri>();
	readonly onDidChange = this.emitter.event;

	constructor(
		private readonly fsys: any,
		private readonly bridge: WanixBridge,
	) {}

	async open(path: string): Promise<void> {
		const uri = inspectUri(path);
		this.emitter.fire(uri);
		const document = await vscode.workspace.openTextDocument(uri);
		await vscode.window.showTextDocument(document, { preview: false });
	}

	async provideTextDocumentContent(uri: vscode.Uri): Promise<string> {
		const path = inspectPath(uri);
		const displayPath = displayWanixPath(path);
		const fsPath = this.bridge.normalizePath(uriPath(path));
		let stat: any;
		try {
			stat = await this.fsys.stat(fsPath);
		} catch (error) {
			return [
				`# Wanix Inspect: ${displayPath}`,
				"",
				`Could not stat ${displayPath}.`,
				"",
				String(error instanceof Error ? error.message : error),
			].join("\n");
		}

		if (!stat?.IsDir) {
			return [
				`# Wanix Inspect: ${displayPath}`,
				"",
				"Kind: file",
				`Size: ${stat?.Size ?? "?"}`,
				"",
				"Open this path through the normal Wanix file provider to read it.",
			].join("\n");
		}

		let entries: string[] = [];
		try {
			entries = normalizeEntries(await this.fsys.readDir(fsPath));
		} catch (error) {
			return [
				`# Wanix Inspect: ${displayPath}`,
				"",
				"Kind: directory",
				"",
				`Could not read directory entries for ${displayPath}.`,
				"",
				String(error instanceof Error ? error.message : error),
			].join("\n");
		}

		return [
			`# Wanix Inspect: ${displayPath}`,
			"",
			"Kind: directory",
			`Backing path: ${fsPath}`,
			"",
			"## Entries",
			"",
			...(entries.length > 0
				? entries.map((entry) => `- ${entryKind(entry)} ${joinWanixPath(displayPath, entryName(entry))}`)
				: ["- empty"]),
			"",
			"## Note",
			"",
			"Service directories are reachable even when they are hidden from ordinary root listings.",
			"This snapshot lists paths without reading files that allocate resources.",
		].join("\n");
	}

	dispose(): void {
		this.emitter.dispose();
	}
}

function inspectUri(path: string): vscode.Uri {
	return vscode.Uri.from({
		scheme: WANIX_INSPECT_SCHEME,
		path: uriPath(path),
	});
}

function inspectPath(uri: vscode.Uri): string {
	return uri.path || "/";
}

function uriPath(path: string): string {
	const trimmed = path.trim();
	if (!trimmed || trimmed === ".") {
		return "/";
	}
	return trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
}

function displayWanixPath(path: string): string {
	const trimmed = path.trim();
	if (!trimmed || trimmed === "." || trimmed === "/") {
		return "/";
	}
	return trimmed.startsWith("/") && trimmed.startsWith("/#") ? trimmed.slice(1) : trimmed;
}

function normalizeEntries(entries: unknown): string[] {
	if (!Array.isArray(entries)) {
		return [];
	}
	return entries
		.map((entry) => typeof entry === "string" ? entry : entryNameFromObject(entry))
		.filter((entry): entry is string => !!entry)
		.sort((left, right) => entryName(left).localeCompare(entryName(right), undefined, { numeric: true }));
}

function entryNameFromObject(entry: unknown): string | undefined {
	if (!entry || typeof entry !== "object") {
		return undefined;
	}
	const candidate = entry as { Name?: string; IsDir?: boolean };
	if (!candidate.Name) {
		return undefined;
	}
	return candidate.IsDir ? `${candidate.Name}/` : candidate.Name;
}

function entryName(entry: string): string {
	return entry.replace(/\/$/, "");
}

function entryKind(entry: string): string {
	return entry.endsWith("/") ? "dir" : "file";
}

function joinWanixPath(parent: string, child: string): string {
	const cleanParent = displayWanixPath(parent).replace(/\/+$/, "");
	if (!cleanParent || cleanParent === "/") {
		return `/${child}`;
	}
	return `${cleanParent}/${child}`;
}
