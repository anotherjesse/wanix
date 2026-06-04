import { baseName, parentPath, splitPath } from './p9-path.js';
import {
	DT_DIR,
	MESSAGE_NAMES,
	P9_NOFID,
	P9_RLERROR,
	P9_TATTACH,
	P9_TCLUNK,
	P9_TGETATTR,
	P9_TLOPEN,
	P9_TREADLINK,
	P9_TREAD,
	P9_TREADDIR,
	P9_TSYMLINK,
	P9_TUNLINKAT,
	P9_TVERSION,
	P9_TWALK,
	P9_TWALKGETATTR,
	P9_TWRITE,
	P9_VERSION,
	P9_VERSION_GOOGLE_2,
	P9Frame,
	P9RemoteAttr,
	Reader,
	Writer,
	frame,
	readAttr,
	readVersion,
	readWalkGetAttr,
} from './p9-wire.js';

export type P9DirEntry = {
	IsDir: boolean;
	Name: string;
	ModTime: number;
	Offset: bigint;
	Size: number;
};

type PendingRpc = {
	name: string;
	expectedType: number;
	resolve: (frame: P9Frame) => void;
	reject: (error: Error) => void;
};

export class P9Session {
	private rootFid = 1;
	private nextFid = 2;
	private nextTag = 1;
	private pending = new Map<number, PendingRpc>();
	private receiveBuffer = new Uint8Array();
	private negotiatedVersion = P9_VERSION;

	logger: (...args: unknown[]) => void = () => null;

	private constructor(private socket: WebSocket) {
		this.socket.binaryType = "arraybuffer";
		this.socket.addEventListener("message", (event) => this.handleMessage(event));
		this.socket.addEventListener("close", () => this.rejectAll("9P websocket closed"));
		this.socket.addEventListener("error", () => this.rejectAll("9P websocket failed"));
	}

	static async connect(websocketUrl: string, preferredVersion = P9_VERSION_GOOGLE_2): Promise<P9Session> {
		const socket = new WebSocket(websocketUrl);
		const session = new P9Session(socket);
		await new Promise<void>((resolve, reject) => {
			socket.addEventListener("open", () => resolve(), { once: true });
			socket.addEventListener("error", () => reject(new Error("9P websocket failed to open")), { once: true });
		});
		await session.version(preferredVersion);
		await session.attach();
		return session;
	}

	async walkPath(name: string): Promise<number> {
		const fid = this.nextFid++;
		const parts = splitPath(name);
		const payload = new Writer();
		payload.u32(this.rootFid);
		payload.u32(fid);
		payload.u16(parts.length);
		for (const part of parts) {
			payload.string(part);
		}
		const response = await this.rpc(P9_TWALK, payload.done());
		const qids = new Reader(response.payload).u16();
		if (qids !== parts.length) {
			throw new Error(`9P walk resolved ${qids} of ${parts.length} path parts`);
		}
		return fid;
	}

	async walkGetAttrPath(name: string): Promise<{ fid: number; attr: P9RemoteAttr }> {
		if (!this.supportsWalkGetAttr()) {
			const fid = await this.walkPath(name);
			try {
				return { fid, attr: await this.getAttr(fid) };
			} catch (error) {
				await this.clunkQuietly(fid);
				throw error;
			}
		}

		const fid = this.nextFid++;
		const parts = splitPath(name);
		const payload = new Writer();
		payload.u32(this.rootFid);
		payload.u32(fid);
		payload.u16(parts.length);
		for (const part of parts) {
			payload.string(part);
		}
		const response = await this.rpc(P9_TWALKGETATTR, payload.done());
		const walked = readWalkGetAttr(response.payload);
		if (walked.qids.length !== parts.length) {
			await this.clunkQuietly(fid);
			throw new Error(`9P walkgetattr resolved ${walked.qids.length} of ${parts.length} path parts`);
		}
		return { fid, attr: walked.attr };
	}

	async getAttr(fid: number): Promise<P9RemoteAttr> {
		const payload = new Writer();
		payload.u32(fid);
		payload.u64(0xffff_ffff_ffff_ffffn);
		const response = await this.rpc(P9_TGETATTR, payload.done());
		return readAttr(response.payload);
	}

	async open(fid: number, flags: number): Promise<void> {
		const payload = new Writer();
		payload.u32(fid);
		payload.u32(flags);
		await this.rpc(P9_TLOPEN, payload.done());
	}

	async read(fid: number, offset: number, count: number): Promise<Uint8Array> {
		const payload = new Writer();
		payload.u32(fid);
		payload.u64(BigInt(offset));
		payload.u32(count);
		const response = await this.rpc(P9_TREAD, payload.done());
		const reader = new Reader(response.payload);
		return reader.data(reader.u32());
	}

	async write(fid: number, offset: number, data: Uint8Array): Promise<number> {
		const payload = new Writer();
		payload.u32(fid);
		payload.u64(BigInt(offset));
		payload.u32(data.length);
		payload.data(data);
		const response = await this.rpc(P9_TWRITE, payload.done());
		return new Reader(response.payload).u32();
	}

	async readdir(fid: number, offset: bigint, count: number): Promise<P9DirEntry[]> {
		const payload = new Writer();
		payload.u32(fid);
		payload.u64(offset);
		payload.u32(count);
		const response = await this.rpc(P9_TREADDIR, payload.done());
		const reader = new Reader(response.payload);
		const dirBytes = new Reader(reader.data(reader.u32()));
		const entries: P9DirEntry[] = [];
		while (dirBytes.remaining() > 0) {
			const qid = dirBytes.qid();
			const offset = dirBytes.u64();
			const direntType = dirBytes.u8();
			const name = dirBytes.string();
			entries.push({
				IsDir: direntType === DT_DIR || (qid.type & 0x80) !== 0,
				Name: name,
				ModTime: 0,
				Offset: offset,
				Size: 0,
			});
		}
		return entries;
	}

	async readlink(fid: number): Promise<string> {
		const payload = new Writer();
		payload.u32(fid);
		const response = await this.rpc(P9_TREADLINK, payload.done());
		return new Reader(response.payload).string();
	}

	async symlink(dirFid: number, name: string, target: string, gid = 0): Promise<void> {
		const payload = new Writer();
		payload.u32(dirFid);
		payload.string(name);
		payload.string(target);
		payload.u32(gid);
		await this.rpc(P9_TSYMLINK, payload.done());
	}

	async unlink(name: string, flags: number): Promise<void> {
		const parent = await this.walkPath(parentPath(name));
		try {
			const payload = new Writer();
			payload.u32(parent);
			payload.string(baseName(name));
			payload.u32(flags);
			await this.rpc(P9_TUNLINKAT, payload.done());
		} finally {
			await this.clunkQuietly(parent);
		}
	}

	async clunkQuietly(fid: number): Promise<void> {
		try {
			await this.clunk(fid);
		} catch {
			// Best-effort cleanup: preserve the original filesystem error.
		}
	}

	async rpc(type: number, payload: Uint8Array): Promise<P9Frame> {
		const tag = this.nextTag++;
		const name = MESSAGE_NAMES.get(type) ?? String(type);
		const expectedType = type + 1;
		this.logger(`-> ${name}`);
		this.socket.send(frame(type, tag, payload));
		return new Promise((resolve, reject) => {
			this.pending.set(tag, { name, expectedType, resolve, reject });
		});
	}

	private async version(preferredVersion: string): Promise<void> {
		const payload = new Writer();
		payload.u32(131072);
		payload.string(preferredVersion);
		const response = await this.rpc(P9_TVERSION, payload.done());
		this.negotiatedVersion = readVersion(response.payload).version;
		if (this.negotiatedVersion === "unknown") {
			throw new Error(`9P server rejected protocol ${preferredVersion}`);
		}
	}

	private async attach(): Promise<void> {
		const payload = new Writer();
		payload.u32(this.rootFid);
		payload.u32(P9_NOFID);
		payload.string("workbench");
		payload.string("");
		payload.u32(0);
		await this.rpc(P9_TATTACH, payload.done());
	}

	async clunk(fid: number): Promise<void> {
		const payload = new Writer();
		payload.u32(fid);
		await this.rpc(P9_TCLUNK, payload.done());
	}

	private supportsWalkGetAttr(): boolean {
		if (this.negotiatedVersion === P9_VERSION_GOOGLE_2) {
			return true;
		}
		const prefix = "9P2000.L.Google.";
		if (!this.negotiatedVersion.startsWith(prefix)) {
			return false;
		}
		return Number(this.negotiatedVersion.slice(prefix.length)) >= 2;
	}

	private handleMessage(event: MessageEvent): void {
		const incoming = new Uint8Array(event.data as ArrayBuffer);
		const combined = new Uint8Array(this.receiveBuffer.length + incoming.length);
		combined.set(this.receiveBuffer);
		combined.set(incoming, this.receiveBuffer.length);
		this.receiveBuffer = combined;

		while (this.receiveBuffer.length >= 7) {
			const view = new DataView(this.receiveBuffer.buffer, this.receiveBuffer.byteOffset, this.receiveBuffer.byteLength);
			const size = view.getUint32(0, true);
			if (this.receiveBuffer.length < size) {
				return;
			}
			const bytes = this.receiveBuffer.slice(0, size);
			this.receiveBuffer = this.receiveBuffer.slice(size);
			this.resolveFrame(bytes);
		}
	}

	private resolveFrame(bytes: Uint8Array): void {
		const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
		const type = view.getUint8(4);
		const tag = view.getUint16(5, true);
		const payload = bytes.slice(7);
		const pending = this.pending.get(tag);
		if (!pending) {
			return;
		}
		this.pending.delete(tag);
		if (type === P9_RLERROR) {
			const errno = new Reader(payload).u32();
			pending.reject(new Error(`${pending.name} failed with errno ${errno}`));
			return;
		}
		if (type !== pending.expectedType) {
			pending.reject(new Error(`${pending.name} returned unexpected 9P type ${type}`));
			return;
		}
		this.logger(`<- ${type} for ${pending.name}`);
		pending.resolve({ type, tag, payload });
	}

	private rejectAll(message: string): void {
		for (const pending of this.pending.values()) {
			pending.reject(new Error(message));
		}
		this.pending.clear();
	}
}
