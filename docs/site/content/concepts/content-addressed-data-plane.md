---
title: Content-Addressed Data Plane (content_hash)
slug: concepts/content-addressed-data-plane
pageType: concept
oneLiner: A FileSystem can expose a BLAKE3 content_hash so CAS-aware clients fetch large files as verified blobs peer-to-peer instead of crawling them through bounded 9P reads.
audience: [developer]
tags: [data-plane, cas, 9p, mesh, shipped, caveat]
sourceRefs:
  - crates/wanix-fs/src/traits.rs:181-205
  - crates/wanix-fs/src/lib.rs:22-31
  - crates/wanix-cas/src/casfs.rs:94-134
  - crates/wanix-fs/src/content_hash.rs:21-29
seeAlso:
  - concepts/the-filesystem-trait
  - concepts/the-9p-contract
  - devices/cas
  - concepts/end-to-end-hash-verification
  - concepts/one-identity-two-planes
  - concepts/wanix-capsule
prerequisites:
  - concepts/the-filesystem-trait
  - concepts/the-9p-contract
usedInFlows: []
honestLimits:
  - "content_hash defaults to Ok(None); most filesystems never override it, and default serve does not wrap its export in a CasFs, so the offload is a wired hook, not a default path."
  - "The hash must be carried out of band; this codebase's Rgetattr decoder rejects trailing bytes, so a hash can never be appended to a fixed-shape 9P response."
  - "Offload only pays off above the 256 KiB threshold; below it, inline 9P reads are cheaper than a blob round-trip."
---

# Content-Addressed Data Plane (content_hash)

A `FileSystem` can expose a BLAKE3 `content_hash` so CAS-aware clients fetch large files as verified blobs peer-to-peer instead of crawling them through bounded 9P reads.

9P is a control plane: it walks paths, opens fids, and pulls bytes back through an `msize`-bounded `Tread` window. That is exactly right for a 4 KiB config file and exactly wrong for a 700 MiB rootfs image. Wanix keeps one file protocol for everything *and* gives bulk content a faster road, with a single hook on the `FileSystem` trait. The hook does not bolt a second protocol onto 9P; it lets a file say "here is the verified name of my bytes — go fetch the blob directly" while the path, the open, and the metadata still flow over 9P. This page is the contract for that hook: what it returns, when it returns nothing, and why the hash never rides inside a 9P response.

## One hook, one threshold

Reading a large file over 9P means looping `Tread` over the connection, one `msize` window at a time. For genuinely large bulk content — a frozen world, a rootfs image, a module input — that is a lot of round-trips through the control plane. The data plane is the escape hatch, and the door into it is one trait method (`crates/wanix-fs/src/traits.rs:181-205`):

```rust
fn content_hash(&self, _path: &NormalizedPath) -> FsResult<Option<ContentHash>> {
    Ok(None)
}
```

`ContentHash` is a 32-byte BLAKE3 digest newtype (`crates/wanix-fs/src/content_hash.rs:21-29`): the stable, verifiable name of a blob, where two byte sequences share a hash if and only if they are equal. When `content_hash` returns `Some(hash)`, a CAS-aware client can fetch the blob from the content-addressed store, verify it against the hash, and skip the `Tread` loop entirely.

The threshold gates *when* that pays off. `CONTENT_HASH_OFFLOAD_THRESHOLD` is 256 KiB (`crates/wanix-fs/src/lib.rs:22-31`). Files at or below it are cheap to read inline over a couple of `msize` windows, so paying a blob round-trip — download plus verify — is pure overhead. The offload only earns its cost on genuinely large content, which is why a CAS-backed filesystem reports `None` for anything small even when it could compute a hash.

## content_hash is the *only* offload door

This is the load-bearing claim, and the trait doc states it flatly: `content_hash` is *the only hook by which the 9P control plane offloads bulk file bytes to the blob plane* (`crates/wanix-fs/src/traits.rs:184-187`). There is no second mechanism. Everything else about a file — opening it, statting it, reading it the ordinary way — still goes through 9P. The hash is the one thing that lets a client step off the control plane for the bytes themselves. Keep the surface that narrow and the control/data split stays a single, auditable seam rather than a sprawl of side channels.

## Default `Ok(None)`; a `CasFs` decorator overrides it

Most filesystems have no blob backing, so the default is `Ok(None)` — "read this file the ordinary way." `None` is not an error; "resolvable but not content-addressed" is a normal answer (`crates/wanix-fs/src/traits.rs:199-202`). The filesystem that opts in is `CasFs`, a decorator that wraps an inner `FileSystem` and a `ContentStore` (`crates/wanix-cas/src/casfs.rs:94-134`). Every read passes through unchanged; what `CasFs` adds is the override:

```rust
fn content_hash(&self, path: &NormalizedPath) -> FsResult<Option<ContentHash>> {
    let metadata = self.inner.metadata(path)?;
    if !metadata.file_type().is_file_like()
        || metadata.len() <= CONTENT_HASH_OFFLOAD_THRESHOLD
        || metadata.len() > MAX_BLOB_SIZE as u64
        || self.is_being_written(path.as_str())
    {
        return Ok(None);
    }
    let bytes = self.read_full(path)?;
    let hash = self.store.put(&bytes).map_err(|err| FsError::Other(err.to_string()))?;
    Ok(Some(hash))
}
```

Note the four `None` gates before any work: not a regular file, at or below the threshold, larger than the blob cap, or currently being written. Only a regular file in the offloadable size band gets ingested into the store and gets a hash.

The freshness gate is the subtle one. A stale hash is a correctness bug — a client must never fetch a blob that no longer matches a file being written. The 9P server's `FidEntry` does not record open mode, so the design forbids scanning fids for writers (`crates/wanix-cas/src/casfs.rs:11-21`). Instead `CasFs` tracks every path currently open for write in a shared set, returns `None` for any such path, and the write handle re-ingests the bytes and clears the marker on close. Between open-for-write and close the file simply has no offloadable hash and is read inline — never offloaded mid-write.

## The hash is carried out of band

Where does the client *learn* the hash? Not inside a 9P response. This codebase's `Rgetattr` decoder rejects trailing bytes, so a hash can never be appended to an existing fixed-shape 9P frame (`crates/wanix-fs/src/traits.rs:194-197`). The hash is carried out of band — an `xattr`-style synthetic file or a versioned protocol field — leaving the 9P wire shape untouched. The control plane stays the control plane; the data plane lives beside it, named by the hash and verified end to end against it (see [end-to-end hash verification](/concepts/end-to-end-hash-verification)).

## See also

- [The FileSystem trait](/concepts/the-filesystem-trait) — where `content_hash` lives, alongside `open`, `metadata`, and the rest of the surface.
- [The 9P contract](/concepts/the-9p-contract) — the control plane this hook offloads bulk bytes away from.
- [#cas device](/devices/cas) — the content-addressed store (`ingest`, `<hash>` read, `have/<hash>`) that holds the blobs.
- [End-to-end hash verification](/concepts/end-to-end-hash-verification) — why a BLAKE3 name makes a fetched blob self-checking.
- [One identity, two planes](/concepts/one-identity-two-planes) — the control plane and data plane share one node identity.
- [wanix capsule](/concepts/wanix-capsule) — CAS-backed frozen worlds, the archetypal large bulk content this offload exists for.

## Status / honest limits

- **`content_hash` defaults to `Ok(None)`.** Most filesystems never override it, and the default `serve` export is not wrapped in a `CasFs`. The offload is a fully specified, code-backed hook — but reaching it requires composing a `CasFs` over the export, so this is a wired hook, not a default-on path.
- **The hash never rides inside a 9P frame.** The `Rgetattr` decoder rejects trailing bytes, so a hash must travel out of band (a synthetic file or versioned field). A client that does not know to look out of band simply reads the file inline over 9P, which is always correct, just slower for big files.
- **Below 256 KiB, offload is not worth it.** `CasFs` returns `None` for anything at or below `CONTENT_HASH_OFFLOAD_THRESHOLD`, and also for non-files, files above the blob cap, and any path open for write. The fast road exists only for large, settled, regular files.
