import {
	CancellationToken,
	Disposable,
	Event,
	EventEmitter,
	FileChangeEvent,
	FileChangeType,
	FileStat,
	FileSystemError,
	FileSystemProvider,
	FileType,
	Progress,
	Range,
	Uri,
	workspace,
} from 'vscode';

interface RemoteEntry {
    IsDir: boolean;
    IsSymlink?: boolean;
    Name: string;
    // Ctime: number;
    ModTime: number;
    Size: number;
}

const DEFAULT_SEARCH_LIMIT = 512;
const MAX_TEXT_SEARCH_BYTES = 1024 * 1024;
const searchDecoder = new TextDecoder();

type WanixSearchWorkspace = typeof workspace & {
	registerFileSearchProvider?: (scheme: string, provider: WanixFileSearchProvider) => Disposable;
	registerTextSearchProvider?: (scheme: string, provider: WanixTextSearchProvider) => Disposable;
};

interface WanixFileSearchProvider {
	provideFileSearchResults(query: WanixFileSearchQuery, options: WanixSearchOptions, token: CancellationToken): Promise<Uri[]>;
}

interface WanixTextSearchProvider {
	provideTextSearchResults(query: WanixTextSearchQuery, options: WanixSearchOptions, progress: Progress<WanixTextSearchResult>, token: CancellationToken): Promise<WanixSearchComplete>;
}

interface WanixFileSearchQuery {
	pattern?: string;
}

interface WanixTextSearchQuery {
	pattern: string;
	isCaseSensitive?: boolean;
	isRegExp?: boolean;
	isWordMatch?: boolean;
	isMultiline?: boolean;
}

interface WanixSearchOptions {
	folder: Uri;
	includes?: string[];
	excludes?: string[];
	maxFileSize?: number;
	maxResults?: number;
}

interface WanixTextSearchResult {
	uri: Uri;
	ranges: Range | Range[];
	preview: {
		text: string;
		matches: Range | Range[];
	};
}

interface WanixSearchComplete {
	limitHit?: boolean;
}

function searchRootUri(options: WanixSearchOptions): Uri {
	const folder = options.folder?.scheme === WanixBridge.scheme
		? options.folder
		: Uri.parse(`${WanixBridge.scheme}:/`);
	return folder.with({ path: normalizeUriPath(folder.path || "/") });
}

function searchLimit(options: WanixSearchOptions): number {
	return options.maxResults && options.maxResults > 0
		? options.maxResults
		: DEFAULT_SEARCH_LIMIT;
}

function textSearchByteLimit(options: WanixSearchOptions): number {
	return options.maxFileSize && options.maxFileSize > 0
		? Math.min(options.maxFileSize, MAX_TEXT_SEARCH_BYTES)
		: MAX_TEXT_SEARCH_BYTES;
}

function normalizeUriPath(path: string): string {
	let normalized = path.replace(/\\/g, "/");
	if (!normalized || normalized === ".") {
		return "/";
	}
	if (!normalized.startsWith("/")) {
		normalized = `/${normalized}`;
	}
	normalized = normalized.replace(/\/+$/, "");
	return normalized || "/";
}

function joinUriPath(parent: string, child: string): string {
	const base = normalizeUriPath(parent);
	return base === "/" ? `/${child}` : `${base}/${child}`;
}

function splitUriPath(path: string): string[] {
	const normalized = normalizeUriPath(path);
	return normalized === "/" ? [] : normalized.slice(1).split("/");
}

function relativeSearchPath(rootPath: string, childPath: string): string {
	const root = normalizeUriPath(rootPath);
	const child = normalizeUriPath(childPath);
	if (root === "/") {
		return child.slice(1);
	}
	if (child === root) {
		return "";
	}
	if (child.startsWith(`${root}/`)) {
		return child.slice(root.length + 1);
	}
	return child.slice(1);
}

function isSearchServicePath(path: string): boolean {
	const [first] = splitUriPath(path);
	return first?.startsWith("#") || false;
}

function pathBaseName(path: string): string {
	const parts = splitUriPath(path);
	return parts[parts.length - 1] || "";
}

function pathMatchesFileSearch(relativePath: string, pattern?: string): boolean {
	const trimmed = (pattern || "").trim().replace(/\\/g, "/");
	if (!trimmed || trimmed === "*" || trimmed === "**/*") {
		return true;
	}
	if (hasGlobSyntax(trimmed)) {
		return globMatches(trimmed, relativePath);
	}
	const needle = trimmed.toLowerCase();
	return relativePath.toLowerCase().includes(needle) || pathBaseName(relativePath).toLowerCase().includes(needle);
}

function pathPassesFilters(relativePath: string, options: WanixSearchOptions): boolean {
	const includes = (options.includes || []).filter(Boolean);
	if (includes.length > 0 && !includes.some((pattern) => globMatches(pattern, relativePath))) {
		return false;
	}
	const excludes = (options.excludes || []).filter(Boolean);
	return !excludes.some((pattern) => globMatches(pattern, relativePath));
}

function globMatches(pattern: string, relativePath: string): boolean {
	const normalizedPattern = normalizeGlobPattern(pattern);
	const normalizedPath = relativePath.replace(/\\/g, "/");
	const patterns = normalizedPattern.startsWith("**/")
		? [normalizedPattern, normalizedPattern.slice(3)]
		: [normalizedPattern];
	return patterns.some((candidate) => {
		const target = candidate.includes("/") ? normalizedPath : pathBaseName(normalizedPath);
		return globToRegExp(candidate).test(target);
	});
}

function normalizeGlobPattern(pattern: string): string {
	let normalized = pattern.trim().replace(/\\/g, "/");
	while (normalized.startsWith("./")) {
		normalized = normalized.slice(2);
	}
	normalized = normalized.replace(/^\/+/, "");
	if (normalized.endsWith("/")) {
		normalized += "**";
	}
	return normalized;
}

function hasGlobSyntax(pattern: string): boolean {
	return /[*?[\]{}]/.test(pattern);
}

function globToRegExp(pattern: string): RegExp {
	let source = "^";
	for (let index = 0; index < pattern.length; index += 1) {
		const char = pattern[index];
		const next = pattern[index + 1];
		if (char === "*" && next === "*") {
			source += ".*";
			index += 1;
		} else if (char === "*") {
			source += "[^/]*";
		} else if (char === "?") {
			source += "[^/]";
		} else {
			source += escapeRegExp(char);
		}
	}
	source += "$";
	return new RegExp(source, "i");
}

function escapeRegExp(value: string): string {
	return value.replace(/[\\^$.*+?()[\]{}|]/g, "\\$&");
}

function textSearchMatcher(query: WanixTextSearchQuery): RegExp | undefined {
	if (!query.pattern) {
		return undefined;
	}
	const flags = query.isCaseSensitive ? "g" : "gi";
	try {
		if (query.isRegExp) {
			return new RegExp(query.pattern, flags);
		}
		const source = query.isWordMatch
			? `\\b${escapeRegExp(query.pattern)}\\b`
			: escapeRegExp(query.pattern);
		return new RegExp(source, flags);
	} catch {
		return undefined;
	}
}

function looksBinary(contents: Uint8Array): boolean {
	const sampleLength = Math.min(contents.length, 8192);
	for (let index = 0; index < sampleLength; index += 1) {
		if (contents[index] === 0) {
			return true;
		}
	}
	return false;
}

function remoteFileType(entry: RemoteEntry): FileType {
	let type = entry.IsDir ? FileType.Directory : FileType.File;
	if (entry.IsSymlink) {
		type |= FileType.SymbolicLink;
	}
	return type;
}

function* textSearchResults(uri: Uri, text: string, matcher: RegExp): Iterable<WanixTextSearchResult> {
	const lines = text.split("\n");
	for (let lineIndex = 0; lineIndex < lines.length; lineIndex += 1) {
		const line = lines[lineIndex].endsWith("\r") ? lines[lineIndex].slice(0, -1) : lines[lineIndex];
		matcher.lastIndex = 0;
		let match: RegExpExecArray | null;
		while ((match = matcher.exec(line)) !== null) {
			const text = match[0] || "";
			if (!text) {
				matcher.lastIndex += 1;
				continue;
			}
			const start = match.index;
			const end = start + text.length;
			const sourceRange = new Range(lineIndex, start, lineIndex, end);
			const previewRange = new Range(0, start, 0, end);
			yield {
				uri,
				ranges: sourceRange,
				preview: {
					text: line,
					matches: previewRange,
				},
			};
		}
	}
}

export class File implements FileStat {

	type: FileType;
	ctime: number;
	mtime: number;
	size: number;

	name: string;

	constructor(public uri: Uri, entry: RemoteEntry) {
		this.type = remoteFileType(entry);
		this.ctime = 0;
		this.mtime = entry.ModTime || 0;
		this.size = entry.Size;
		this.name = entry.Name;
	}
}

export class Directory implements FileStat {

	type: FileType;
	ctime: number;
	mtime: number;
	size: number;

	name: string;

	constructor(public uri: Uri, entry: RemoteEntry) {
		this.type = remoteFileType(entry);
		this.ctime = 0;
		this.mtime = entry.ModTime || 0;
		this.size = entry.Size;
		this.name = entry.Name;
	}
}

export type Entry = File | Directory;

export class WanixBridge implements FileSystemProvider, WanixFileSearchProvider, WanixTextSearchProvider, Disposable {
	static scheme = 'wanix';

	public wfsys: any;
	public readonly ready: Promise<any>;
	private readonly disposable: Disposable;
	private root: string;

	constructor(wanix: Promise<any>, root: string) {
		this.ready = wanix;
		this.ready.then((fsys) => {
			this.wfsys = fsys;
		});
		this.root = root;
		const disposables = [
			workspace.registerFileSystemProvider(WanixBridge.scheme, this, { isCaseSensitive: true }),
		];
		const searchWorkspace = workspace as WanixSearchWorkspace;
		const fileSearch = searchWorkspace.registerFileSearchProvider?.(WanixBridge.scheme, this);
		if (fileSearch) {
			disposables.push(fileSearch);
		}
		const textSearch = searchWorkspace.registerTextSearchProvider?.(WanixBridge.scheme, this);
		if (textSearch) {
			disposables.push(textSearch);
		}
		this.disposable = Disposable.from(...disposables);
	}

	normalizePath(path: string): string {
		let p = this.root + path; 
		if (path === "/") {
			p = this.root || ".";
		}
		if (p === "/") {
			return ".";
		}
		if (p.startsWith("/")) {
			p = p.slice(1);
		}
		return p;
	}

	dispose() {
		this.disposable?.dispose();
	}

	// --- manage file metadata

	stat(uri: Uri): Thenable<FileStat> {
		return this._stat(uri);
	}

	async _stat(uri: Uri): Promise<FileStat> {
		// console.log("stat", uri);
		if (!this.wfsys) {
			
			// if (uri.path !== "/project") {
			// 	if (uri.path.includes(".vscode")) {
			// 		throw FileSystemError.FileNotFound(uri);
			// 	}
			// 	return new File(uri, {
			// 		IsDir: false,
			// 		Name: this._basename(uri.path),
			// 		ModTime: 0,
			// 		Size: 0,
			// 	});
			// }
			// todo: watch root to force reload?
			// return new Directory(uri, {
			// 	IsDir: true,
			// 	Name: uri.path,
			// 	ModTime: 0,
			// 	Size: 0,
			// });
		}
		await this.ready;
		return await this._lookup(uri, false);
	}

	readDirectory(uri: Uri): Thenable<[string, FileType][]> {
		return this._readDirectory(uri);
	}

	async _readDirectory(uri: Uri): Promise<[string, FileType][]> {
        await this.ready;
		if (typeof this.wfsys.readDirEntries === "function") {
			const entries: RemoteEntry[] = await this.wfsys.readDirEntries(this.normalizePath(uri.path));
			return entries.map((entry) => [entry.Name, remoteFileType(entry)]);
		}
		const entries = await this.wfsys.readDir(this.normalizePath(uri.path));
		let result: [string, FileType][] = [];
		for (const entry of entries) {
			result.push([entry.replace(/\/$/, ''), (entry.endsWith('/')) ? FileType.Directory : FileType.File]);
		}
		return result;
	}

	// --- manage file contents

	readFile(uri: Uri): Thenable<Uint8Array> {
		return this._readFile(uri);
	}

	async _readFile(uri: Uri): Promise<Uint8Array> {
		await this.ready;
		return await this.wfsys.readFile(this.normalizePath(uri.path));
	}

	async provideFileSearchResults(query: WanixFileSearchQuery, options: WanixSearchOptions, token: CancellationToken): Promise<Uri[]> {
		await this.ready;
		const base = searchRootUri(options);
		const limit = searchLimit(options);
		const matches: Uri[] = [];
		await this._walkSearchFiles(base, token, async (uri, relativePath) => {
			if (!pathMatchesFileSearch(relativePath, query.pattern)) {
				return false;
			}
			if (!pathPassesFilters(relativePath, options)) {
				return false;
			}
			matches.push(uri);
			return matches.length >= limit;
		});
		return matches;
	}

	async provideTextSearchResults(query: WanixTextSearchQuery, options: WanixSearchOptions, progress: Progress<WanixTextSearchResult>, token: CancellationToken): Promise<WanixSearchComplete> {
		await this.ready;
		const matcher = textSearchMatcher(query);
		if (!matcher) {
			return {};
		}

		const base = searchRootUri(options);
		const limit = searchLimit(options);
		const maxFileBytes = textSearchByteLimit(options);
		let count = 0;
		let limitHit = false;
		await this._walkSearchFiles(base, token, async (uri, relativePath, stat) => {
			if (!pathPassesFilters(relativePath, options)) {
				return false;
			}
			if (stat.Size > maxFileBytes) {
				return false;
			}
			const contents = await this.wfsys.readFile(this.normalizePath(uri.path));
			if (contents.length > maxFileBytes || looksBinary(contents)) {
				return false;
			}
			const text = searchDecoder.decode(contents);
			for (const result of textSearchResults(uri, text, matcher)) {
				if (token.isCancellationRequested) {
					return true;
				}
				progress.report(result);
				count += 1;
				if (count >= limit) {
					limitHit = true;
					return true;
				}
			}
			return false;
		});
		return { limitHit };
	}

	writeFile(uri: Uri, content: Uint8Array, options: { create: boolean, overwrite: boolean }): Thenable<void> {
		return this._writeFile(uri, content, options);
	}

	async _writeFile(uri: Uri, content: Uint8Array, options: { create: boolean, overwrite: boolean }): Promise<void> {
		await this.ready;
		let entry = await this._lookup(uri, true);
		if (entry instanceof Directory) {
			throw FileSystemError.FileIsADirectory(uri);
		}
		if (!entry && !options.create) {
			throw FileSystemError.FileNotFound(uri);
		}
		if (entry && options.create && !options.overwrite) {
			throw FileSystemError.FileExists(uri);
		}

		await this.wfsys.writeFile(this.normalizePath(uri.path), content);
		
		if (!entry) {
			this._fireSoon({ type: FileChangeType.Created, uri });
		} else {
			this._fireSoon({ type: FileChangeType.Changed, uri });
		}
		this._fireSoon(
			{ type: FileChangeType.Changed, uri: uri.with({ path: this._dirname(uri.path) }) }
		);
	}

	// --- manage files/folders

    copy(source: Uri, destination: Uri, options: {overwrite: boolean}): Thenable<void> {
		return this._copy(source, destination, options);
	}

	async _copy(source: Uri, destination: Uri, options: {overwrite: boolean}): Promise<void> {
		await this.ready;
		if (!options.overwrite && await this._lookup(destination, true)) {
			throw FileSystemError.FileExists(destination);
		}

		await this.wfsys.copy(this.normalizePath(source.path), this.normalizePath(destination.path), {
			overwrite: options.overwrite,
		});

		this._fireSoon(
			{ type: FileChangeType.Changed, uri: destination.with({ path: this._dirname(destination.path) }) },
			{ type: FileChangeType.Created, uri: destination }
		);
	}

	rename(oldUri: Uri, newUri: Uri, options: { overwrite: boolean }): Thenable<void> {
		return this._rename(oldUri, newUri, options);
	}

	async _rename(oldUri: Uri, newUri: Uri, options: { overwrite: boolean }): Promise<void> {
		await this.ready;
		if (!options.overwrite && await this._lookup(newUri, true)) {
			throw FileSystemError.FileExists(newUri);
		}

		await this.wfsys.rename(this.normalizePath(oldUri.path), this.normalizePath(newUri.path));

		this._fireSoon(
			{ type: FileChangeType.Changed, uri: oldUri.with({ path: this._dirname(oldUri.path) }) },
			{ type: FileChangeType.Deleted, uri: oldUri },
			{ type: FileChangeType.Changed, uri: newUri.with({ path: this._dirname(newUri.path) }) },
			{ type: FileChangeType.Created, uri: newUri }
		);
	}

	delete(uri: Uri, options: {recursive: boolean}): Thenable<void> {
		return this._delete(uri, options);
	}

	async _delete(uri: Uri, options: {recursive: boolean}): Promise<void> {
		await this.ready;
		if (options.recursive) {
			await this.wfsys.removeAll(this.normalizePath(uri.path));
		} else {
			await this.wfsys.remove(this.normalizePath(uri.path));
		}

		this._fireSoon(
			{ type: FileChangeType.Changed, uri: uri.with({ path: this._dirname(uri.path) }) }, 
			{ uri, type: FileChangeType.Deleted }
		);
	}

	createDirectory(uri: Uri): Promise<void> {
		return this._createDirectory(uri);
	}

	async _createDirectory(uri: Uri): Promise<void> {
		await this.ready;
		await this.wfsys.makeDir(this.normalizePath(uri.path));
		this._fireSoon(
			{ type: FileChangeType.Changed, uri: uri.with({ path: this._dirname(uri.path) }) }, 
			{ type: FileChangeType.Created, uri }
		);
	}

	private async _walkSearchFiles(base: Uri, token: CancellationToken, visit: (uri: Uri, relativePath: string, stat: RemoteEntry) => Promise<boolean>): Promise<void> {
		const rootPath = normalizeUriPath(base.path || "/");
		const stack = [rootPath];
		while (stack.length > 0) {
			if (token.isCancellationRequested) {
				return;
			}
			const directory = stack.pop()!;
			if (isSearchServicePath(directory)) {
				continue;
			}

			let entries: string[];
			try {
				entries = await this.wfsys.readDir(this.normalizePath(directory));
			} catch {
				continue;
			}
			entries.sort((a, b) => a.localeCompare(b));

			for (const entry of entries) {
				if (token.isCancellationRequested) {
					return;
				}
				const isDirectory = entry.endsWith("/");
				const name = isDirectory ? entry.slice(0, -1) : entry;
				const path = joinUriPath(directory, name);
				if (isSearchServicePath(path)) {
					continue;
				}
				if (isDirectory) {
					stack.push(path);
					continue;
				}

				let stat: RemoteEntry;
				try {
					stat = await this.wfsys.stat(this.normalizePath(path));
				} catch {
					continue;
				}
				const stop = await visit(base.with({ path }), relativeSearchPath(rootPath, path), stat);
				if (stop) {
					return;
				}
			}
		}
	}

	// --- lookup

	private async _lookup(uri: Uri, silent: false): Promise<Entry>;
	private async _lookup(uri: Uri, silent: boolean): Promise<Entry | undefined>;
	private async _lookup(uri: Uri, silent: boolean): Promise<Entry | undefined> {
        try {
            const entry = await this.wfsys.stat(this.normalizePath(uri.path));
            if (entry.IsDir) {
                return new Directory(uri, entry);
            } else {
                return new File(uri, entry);
            }
        } catch (e) {
            if (!silent) {
                // console.error(e);
                throw FileSystemError.FileNotFound(uri);
            } else {
                return undefined;
            }
        }
	}

	private async _lookupAsDirectory(uri: Uri, silent: boolean): Promise<Directory> {
		let entry = await this._lookup(uri, silent);
		if (entry instanceof Directory) {
			return entry;
		}
		throw FileSystemError.FileNotADirectory(uri);
	}

	private async _lookupAsFile(uri: Uri, silent: boolean): Promise<File> {
		let entry = await this._lookup(uri, silent);
		if (entry instanceof File) {
			return entry;
		}
		throw FileSystemError.FileIsADirectory(uri);
	}

	private async _lookupParentDirectory(uri: Uri): Promise<Directory> {
		const dirname = uri.with({ path: this._dirname(uri.path) });
		return await this._lookupAsDirectory(dirname, false);
	}

	// --- manage file events

	private _emitter = new EventEmitter<FileChangeEvent[]>();
	private _bufferedEvents: FileChangeEvent[] = [];
	private _fireSoonHandle?: any;

	readonly onDidChangeFile: Event<FileChangeEvent[]> = this._emitter.event;

	watch(_resource: Uri): Disposable {
		// ignore, fires for all changes...
		return new Disposable(() => { });
	}

	private _fireSoon(...events: FileChangeEvent[]): void {
		this._bufferedEvents.push(...events);

		if (this._fireSoonHandle) {
			clearTimeout(this._fireSoonHandle);
		}

		this._fireSoonHandle = setTimeout(() => {
			this._emitter.fire(this._bufferedEvents);
			this._bufferedEvents.length = 0;
		}, 5);
	}

	// --- path utils

	private _basename(path: string): string {
		path = this._rtrim(path, '/');
		if (!path) {
			return '';
		}

		return path.substr(path.lastIndexOf('/') + 1);
	}

	private _dirname(path: string): string {
		path = this._rtrim(path, '/');
		if (!path) {
			return '/';
		}

		return path.substr(0, path.lastIndexOf('/'));
	}

	private _rtrim(haystack: string, needle: string): string {
		if (!haystack || !needle) {
			return haystack;
		}

		const needleLen = needle.length,
			haystackLen = haystack.length;

		if (needleLen === 0 || haystackLen === 0) {
			return haystack;
		}

		let offset = haystackLen,
			idx = -1;

		while (true) {
			idx = haystack.lastIndexOf(needle, offset - 1);
			if (idx === -1 || idx + needleLen !== offset) {
				break;
			}
			if (idx === 0) {
				return '';
			}
			offset = idx;
		}

		return haystack.substring(0, offset);
	}

}
