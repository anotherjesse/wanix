export type P9Frame = {
	type: number;
	tag: number;
	payload: Uint8Array;
};

export const P9_TVERSION = 100;
export const P9_TATTACH = 104;
export const P9_TWALK = 110;
export const P9_TWALKGETATTR = 126;
export const P9_TGETATTR = 24;
export const P9_TLOPEN = 12;
export const P9_TLCREATE = 14;
export const P9_TREAD = 116;
export const P9_TWRITE = 118;
export const P9_TCLUNK = 120;
export const P9_TREADDIR = 40;
export const P9_TMKDIR = 72;
export const P9_TRENAMEAT = 74;
export const P9_TUNLINKAT = 76;
export const P9_RLERROR = 7;
export const P9_NOFID = 0xffff_ffff;
export const P9_VERSION = "9P2000.L";
export const P9_VERSION_GOOGLE_2 = "9P2000.L.Google.2";

export const DT_DIR = 4;
export const AT_REMOVEDIR = 0x200;
export const O_RDONLY = 0;
export const O_RDWR = 0o2;
export const O_TRUNC = 0o1000;
export const MODE_FILE = 0o100664;
export const MODE_DIR = 0o040755;

const S_IFMT = 0o170000;
const S_IFDIR = 0o040000;

export type P9Qid = {
	type: number;
	version: number;
	path: bigint;
};

export type P9RemoteAttr = {
	isDir: boolean;
	size: number;
	mtimeMs: number;
};

type P9AttrBody = {
	mode: number;
	size: number;
	mtimeMs: number;
};

const utf8 = new TextEncoder();
const text = new TextDecoder();

export const MESSAGE_NAMES = new Map<number, string>([
	[P9_TVERSION, "Tversion"],
	[P9_TATTACH, "Tattach"],
	[P9_TWALK, "Twalk"],
	[P9_TWALKGETATTR, "Twalkgetattr"],
	[P9_TGETATTR, "Tgetattr"],
	[P9_TLOPEN, "Tlopen"],
	[P9_TLCREATE, "Tlcreate"],
	[P9_TREAD, "Tread"],
	[P9_TWRITE, "Twrite"],
	[P9_TCLUNK, "Tclunk"],
	[P9_TREADDIR, "Treaddir"],
	[P9_TMKDIR, "Tmkdir"],
	[P9_TRENAMEAT, "Trenameat"],
	[P9_TUNLINKAT, "Tunlinkat"],
]);

export class Writer {
	private parts: number[] = [];

	u8(value: number): void {
		this.parts.push(value & 0xff);
	}

	u16(value: number): void {
		this.parts.push(value & 0xff, (value >>> 8) & 0xff);
	}

	u32(value: number): void {
		this.parts.push(value & 0xff, (value >>> 8) & 0xff, (value >>> 16) & 0xff, (value >>> 24) & 0xff);
	}

	u64(value: bigint): void {
		let remaining = value;
		for (let i = 0; i < 8; i += 1) {
			this.parts.push(Number(remaining & 0xffn));
			remaining >>= 8n;
		}
	}

	string(value: string): void {
		const bytes = utf8.encode(value);
		this.u16(bytes.length);
		this.data(bytes);
	}

	data(bytes: Uint8Array): void {
		for (const byte of bytes) {
			this.u8(byte);
		}
	}

	done(): Uint8Array {
		return Uint8Array.from(this.parts);
	}
}

export class Reader {
	private offset = 0;

	constructor(private bytes: Uint8Array) {}

	u8(): number {
		return this.bytes[this.offset++];
	}

	u16(): number {
		const value = this.bytes[this.offset] | (this.bytes[this.offset + 1] << 8);
		this.offset += 2;
		return value;
	}

	u32(): number {
		const view = new DataView(this.bytes.buffer, this.bytes.byteOffset + this.offset, 4);
		const value = view.getUint32(0, true);
		this.offset += 4;
		return value;
	}

	u64(): bigint {
		const view = new DataView(this.bytes.buffer, this.bytes.byteOffset + this.offset, 8);
		const value = view.getBigUint64(0, true);
		this.offset += 8;
		return value;
	}

	qid(): P9Qid {
		return {
			type: this.u8(),
			version: this.u32(),
			path: this.u64(),
		};
	}

	string(): string {
		const length = this.u16();
		const value = text.decode(this.bytes.slice(this.offset, this.offset + length));
		this.offset += length;
		return value;
	}

	data(count: number): Uint8Array {
		const data = this.bytes.slice(this.offset, this.offset + count);
		this.offset += count;
		return data;
	}

	remaining(): number {
		return this.bytes.length - this.offset;
	}
}

export function frame(type: number, tag: number, payload: Uint8Array): ArrayBuffer {
	const buffer = new ArrayBuffer(7 + payload.length);
	const view = new DataView(buffer);
	view.setUint32(0, buffer.byteLength, true);
	view.setUint8(4, type);
	view.setUint16(5, tag, true);
	new Uint8Array(buffer, 7).set(payload);
	return buffer;
}

export function readVersion(payload: Uint8Array): { msize: number; version: string } {
	const reader = new Reader(payload);
	return {
		msize: reader.u32(),
		version: reader.string(),
	};
}

export function readAttr(payload: Uint8Array): P9RemoteAttr {
	const reader = new Reader(payload);
	reader.u64();
	const qid = reader.qid();
	return remoteAttr(readAttrBody(reader), qid);
}

export function readWalkGetAttr(payload: Uint8Array): { attr: P9RemoteAttr; qids: P9Qid[] } {
	const reader = new Reader(payload);
	reader.u64();
	const body = readAttrBody(reader);
	const qidCount = reader.u16();
	const qids: P9Qid[] = [];
	for (let i = 0; i < qidCount; i += 1) {
		qids.push(reader.qid());
	}
	return {
		attr: remoteAttr(body, qids[qids.length - 1]),
		qids,
	};
}

function readAttrBody(reader: Reader): P9AttrBody {
	const mode = reader.u32();
	reader.u32();
	reader.u32();
	reader.u64();
	reader.u64();
	const size = Number(reader.u64());
	reader.u64();
	reader.u64();
	reader.u64();
	reader.u64();
	const mtimeSeconds = Number(reader.u64());
	const mtimeNanoseconds = Number(reader.u64());
	reader.u64();
	reader.u64();
	reader.u64();
	reader.u64();
	reader.u64();
	reader.u64();
	return {
		mode,
		size,
		mtimeMs: (mtimeSeconds * 1000) + Math.floor(mtimeNanoseconds / 1_000_000),
	};
}

function remoteAttr(body: P9AttrBody, qid: P9Qid | undefined): P9RemoteAttr {
	return {
		isDir: (body.mode & S_IFMT) === S_IFDIR || ((qid?.type ?? 0) & 0x80) !== 0,
		size: body.size,
		mtimeMs: body.mtimeMs,
	};
}
