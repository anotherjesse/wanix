import * as vscode from 'vscode';
import { WanixBridge } from './bridge.js';

export const WANIX_INSPECT_SCHEME = "wanix-inspect";

export class WanixServiceInspector implements vscode.TextDocumentContentProvider, vscode.DocumentLinkProvider, vscode.Disposable {
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
				? entries.map((entry) => renderEntry(displayPath, entry))
				: ["- empty"]),
			"",
			"## Note",
			"",
			"Service directories are reachable even when they are hidden from ordinary root listings.",
			"This snapshot lists paths without reading files that allocate resources.",
			"Safe metadata files and child directories are document links.",
			"Allocator, control, and stream files stay plain text with reasons.",
		].join("\n");
	}

	provideDocumentLinks(document: vscode.TextDocument): vscode.DocumentLink[] {
		const links: vscode.DocumentLink[] = [];
		for (let lineNumber = 0; lineNumber < document.lineCount; lineNumber += 1) {
			const line = document.lineAt(lineNumber);
			const entry = serviceEntryFromLine(line.text);
			if (!entry) {
				continue;
			}
			const target = entryTarget(entry);
			if (!target) {
				continue;
			}
			const start = line.text.indexOf(entry.path);
			const range = new vscode.Range(lineNumber, start, lineNumber, start + entry.path.length);
			const link = new vscode.DocumentLink(range, target);
			link.tooltip = entry.kind === "dir" ? `Inspect ${entry.path}` : `Open ${entry.path}`;
			links.push(link);
		}
		return links;
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

type ServiceEntry = {
	kind: "dir" | "file";
	path: string;
};

function serviceEntryFromLine(line: string): ServiceEntry | undefined {
	const match = line.match(/^- (dir|file) (\S+)/);
	if (!match) {
		return undefined;
	}
	return {
		kind: match[1] as "dir" | "file",
		path: match[2].trim(),
	};
}

function entryTarget(entry: ServiceEntry): vscode.Uri | undefined {
	if (entry.kind === "dir") {
		return inspectUri(entry.path);
	}
	if (!isSafeFileLink(entry.path)) {
		return undefined;
	}
	return vscode.Uri.from({
		scheme: WanixBridge.scheme,
		path: uriPath(entry.path),
	});
}

function isSafeFileLink(path: string): boolean {
	return unsafeFileReason(path) === undefined;
}

function unsafeFileReason(path: string): string | undefined {
	const normalized = displayWanixPath(path).replace(/^\/+/, "");
	const parts = normalized.split("/");
	if (!parts[0]?.startsWith("#")) {
		return undefined;
	}
	if (parts[0] === "#task") {
		if (parts.length === 3 && ["cmd", "dir", "env", "exit", "id", "kind"].includes(parts[2])) {
			return undefined;
		}
		if (parts[2] === "ctl") {
			return "control file; write-only operations are not linked";
		}
		if (parts[2] === "fd") {
			return "task fd stream; inspect the fd directory first";
		}
		return "task service file; left plain until its read semantics are explicit";
	}
	if (parts[0] === "#term") {
		if (parts.length === 2 && parts[1] === "new") {
			return "allocator file; reading it creates a terminal";
		}
		if (parts[2] === "ctl") {
			return "control file; write-only operations are not linked";
		}
		if (["data", "program"].includes(parts[2])) {
			return "terminal stream; opening it can consume live I/O";
		}
		if (parts[2] === "winch") {
			return "terminal resize feed; left plain until stream reads are explicit";
		}
		return "terminal service file; left plain until its read semantics are explicit";
	}
	return "service file; left plain until its read semantics are explicit";
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

function renderEntry(parent: string, entry: string): string {
	const path = joinWanixPath(parent, entryName(entry));
	const reason = entry.endsWith("/") ? undefined : unsafeFileReason(path);
	const suffix = reason ? ` -- ${reason}` : "";
	return `- ${entryKind(entry)} ${path}${suffix}`;
}

function joinWanixPath(parent: string, child: string): string {
	const cleanParent = displayWanixPath(parent).replace(/\/+$/, "");
	if (!cleanParent || cleanParent === "/") {
		return `/${child}`;
	}
	return `${cleanParent}/${child}`;
}
