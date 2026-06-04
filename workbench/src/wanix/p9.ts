import {
	AT_REMOVEDIR,
	MODE_DIR,
	MODE_FILE,
	O_RDONLY,
	O_RDWR,
	O_TRUNC,
	P9_TGETATTR,
	P9_TLCREATE,
	P9_TMKDIR,
	P9_TRENAMEAT,
	Writer,
	readAttr,
} from './p9-wire.js';
import { P9Session } from './p9-session.js';
import { baseName, baseNameOrRoot, joinPath, normalizePath, parentPath, splitPath } from './p9-path.js';

type DiscoveryDocument = {
	routes?: {
		p9?: {
			websocket?: string;
		};
	};
};

type RemoteEntry = {
	IsDir: boolean;
	Name: string;
	ModTime: number;
	Size: number;
};

const READ_CHUNK_SIZE = 64 * 1024;

const utf8 = new TextEncoder();
const text = new TextDecoder();

export class WanixP9Handle {
	private log: (...args: unknown[]) => void = () => null;

	private constructor(private session: P9Session) {}

	get logger(): (...args: unknown[]) => void {
		return this.log;
	}

	set logger(next: (...args: unknown[]) => void) {
		this.log = next;
		this.session.logger = next;
	}

	static async fromDiscovery(discoveryUrl = "/.well-known/wanix.json"): Promise<WanixP9Handle> {
		const response = await fetch(discoveryUrl, { cache: "no-store" });
		if (!response.ok) {
			throw new Error(`Wanix discovery failed: HTTP ${response.status}`);
		}
		const discovery = await response.json() as DiscoveryDocument;
		const websocketUrl = discovery.routes?.p9?.websocket;
		if (!websocketUrl) {
			throw new Error("Wanix discovery did not advertise a 9P websocket route");
		}
		return await WanixP9Handle.connect(websocketUrl);
	}

	static async connect(websocketUrl: string): Promise<WanixP9Handle> {
		return new WanixP9Handle(await P9Session.connect(websocketUrl));
	}

	async readDir(name: string): Promise<string[]> {
		this.logger(`readDir ${name}`);
		const fid = await this.session.walkPath(name);
		try {
			await this.session.open(fid, O_RDONLY);
			const entries = await this.session.readdir(fid, READ_CHUNK_SIZE);
			return entries.map((entry) => entry.IsDir ? `${entry.Name}/` : entry.Name);
		} finally {
			await this.session.clunkQuietly(fid);
		}
	}

	async makeDir(name: string): Promise<void> {
		this.logger(`makeDir ${name}`);
		const parent = await this.session.walkPath(parentPath(name));
		try {
			const payload = new Writer();
			payload.u32(parent);
			payload.string(baseName(name));
			payload.u32(MODE_DIR);
			payload.u32(0);
			await this.session.rpc(P9_TMKDIR, payload.done());
		} finally {
			await this.session.clunkQuietly(parent);
		}
	}

	async makeDirAll(name: string): Promise<void> {
		this.logger(`makeDirAll ${name}`);
		let current = "/";
		for (const part of splitPath(name)) {
			current = joinPath(current, part);
			try {
				await this.makeDir(current);
			} catch (error) {
				if (!(await this.stat(current)).IsDir) {
					throw error;
				}
			}
		}
	}

	async readFile(name: string): Promise<Uint8Array> {
		this.logger(`readFile ${name}`);
		const fid = await this.session.walkPath(name);
		const chunks: Uint8Array[] = [];
		let total = 0;
		try {
			await this.session.open(fid, O_RDONLY);
			let offset = 0;
			while (true) {
				const chunk = await this.session.read(fid, offset, READ_CHUNK_SIZE);
				if (chunk.length === 0) {
					break;
				}
				chunks.push(chunk);
				total += chunk.length;
				offset += chunk.length;
				if (chunk.length < READ_CHUNK_SIZE) {
					break;
				}
			}
		} finally {
			await this.session.clunkQuietly(fid);
		}
		const out = new Uint8Array(total);
		let offset = 0;
		for (const chunk of chunks) {
			out.set(chunk, offset);
			offset += chunk.length;
		}
		return out;
	}

	async readText(name: string): Promise<string> {
		return text.decode(await this.readFile(name));
	}

	async waitFor(name: string, timeoutMs = 1000): Promise<void> {
		this.logger(`waitFor ${name} ${timeoutMs}ms`);
		const start = Date.now();
		while (Date.now() - start <= timeoutMs) {
			try {
				await this.stat(name);
				return;
			} catch {
				await delay(25);
			}
		}
		throw new Error(`timed out waiting for ${name}`);
	}

	async stat(name: string): Promise<RemoteEntry> {
		this.logger(`stat ${name}`);
		const fid = await this.session.walkPath(name);
		try {
			const payload = new Writer();
			payload.u32(fid);
			payload.u64(0xffff_ffff_ffff_ffffn);
			const response = await this.session.rpc(P9_TGETATTR, payload.done());
			const attr = readAttr(response.payload);
			return {
				IsDir: attr.isDir,
				Name: baseNameOrRoot(name),
				ModTime: attr.mtimeMs,
				Size: attr.size,
			};
		} finally {
			await this.session.clunkQuietly(fid);
		}
	}

	async writeFile(name: string, contents: Uint8Array | string): Promise<void> {
		const data = typeof contents === "string" ? utf8.encode(contents) : contents;
		this.logger(`writeFile ${name} len(${data.length})`);
		const parent = await this.session.walkPath(parentPath(name));
		try {
			const payload = new Writer();
			payload.u32(parent);
			payload.string(baseName(name));
			payload.u32(O_RDWR | O_TRUNC);
			payload.u32(MODE_FILE);
			payload.u32(0);
			await this.session.rpc(P9_TLCREATE, payload.done());
			await this.session.write(parent, 0, data);
		} finally {
			await this.session.clunkQuietly(parent);
		}
	}

	async appendFile(name: string, contents: Uint8Array | string): Promise<void> {
		const previous = await this.readFile(name).catch(() => new Uint8Array());
		const suffix = typeof contents === "string" ? utf8.encode(contents) : contents;
		const combined = new Uint8Array(previous.length + suffix.length);
		combined.set(previous);
		combined.set(suffix, previous.length);
		await this.writeFile(name, combined);
	}

	async rename(oldname: string, newname: string): Promise<void> {
		this.logger(`rename ${oldname} ${newname}`);
		const oldParent = await this.session.walkPath(parentPath(oldname));
		const newParent = await this.session.walkPath(parentPath(newname));
		try {
			const payload = new Writer();
			payload.u32(oldParent);
			payload.string(baseName(oldname));
			payload.u32(newParent);
			payload.string(baseName(newname));
			await this.session.rpc(P9_TRENAMEAT, payload.done());
		} finally {
			await this.session.clunkQuietly(oldParent);
			await this.session.clunkQuietly(newParent);
		}
	}

	async copy(oldname: string, newname: string): Promise<void> {
		this.logger(`copy ${oldname} ${newname}`);
		const source = await this.stat(oldname);
		if (source.IsDir) {
			throw new Error("direct 9P copy currently supports files only");
		}
		await this.writeFile(newname, await this.readFile(oldname));
	}

	async remove(name: string): Promise<void> {
		this.logger(`remove ${name}`);
		const entry = await this.stat(name);
		await this.session.unlink(name, entry.IsDir ? AT_REMOVEDIR : 0);
	}

	async removeAll(name: string): Promise<void> {
		this.logger(`removeAll ${name}`);
		const entry = await this.stat(name);
		if (!entry.IsDir) {
			await this.session.unlink(name, 0);
			return;
		}
		for (const child of await this.readDir(name)) {
			await this.removeAll(joinPath(name, child.replace(/\/$/, "")));
		}
		if (normalizePath(name) !== "/") {
			await this.session.unlink(name, AT_REMOVEDIR);
		}
	}

	async openReadable(name: string): Promise<ReadableStream<Uint8Array>> {
		this.logger(`openReadable ${name}`);
		const data = await this.readFile(name);
		return new ReadableStream<Uint8Array>({
			start(controller) {
				controller.enqueue(data);
				controller.close();
			},
		});
	}

	async openWritable(name: string): Promise<WritableStream<Uint8Array>> {
		this.logger(`openWritable ${name}`);
		const chunks: Uint8Array[] = [];
		return new WritableStream<Uint8Array>({
			write(chunk) {
				chunks.push(chunk);
			},
			close: async () => {
				const length = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
				const out = new Uint8Array(length);
				let offset = 0;
				for (const chunk of chunks) {
					out.set(chunk, offset);
					offset += chunk.length;
				}
				await this.writeFile(name, out);
			},
		});
	}

}

function delay(ms: number): Promise<void> {
	return new Promise((resolve) => setTimeout(resolve, ms));
}
