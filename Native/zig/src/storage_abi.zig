const std = @import("std");
const storage = @import("pager_v2.zig");
const index = @import("index_v2.zig");

// Native Zig-owned pager ABI for trusted Rust and compiled Noqeri callers.
// ABI v2, experimental Linux-only; must not be treated as a public unauthenticated
// SQL, multi-tenant, or database-server interface. Caller owns authorization.
pub const COMPONENT_VERSION = "1.0.0";
pub const ABI_VERSION: u32 = 2;
pub const STATUS_OK: i32 = 0;
pub const STATUS_NOT_FOUND: i32 = 1;
pub const STATUS_INVALID: i32 = -1;
pub const STATUS_FAILURE: i32 = -2;
pub const STATUS_BUFFER_TOO_SMALL: i32 = -3;
pub const STATUS_DUPLICATE: i32 = -4;
pub const STATUS_INDEX_FULL: i32 = -5;
// Retryable storage contention: distinct from corruption, IO and invalid formats.
pub const STATUS_LOCKED: i32 = -6;

pub export fn noesql_zig_page_open(version: u32, name: ?[*:0]const u8, out: ?*?*anyopaque) i32 {
    const dest = out orelse return STATUS_INVALID;
    dest.* = null;
    if (version != ABI_VERSION) return STATUS_INVALID;
    const path = name orelse return STATUS_INVALID;
    const value = std.mem.span(path);
    if (value.len == 0 or value.len > 4000) return STATUS_INVALID;
    const pager = storage.Pager.open(std.heap.page_allocator, value) catch |err| return switch (err) {
        error.Locked => STATUS_LOCKED,
        else => STATUS_FAILURE,
    };
    dest.* = @ptrCast(pager);
    return STATUS_OK;
}
fn cast(handle: ?*anyopaque) ?*storage.Pager {
    const value = handle orelse return null;
    return @ptrCast(@alignCast(value));
}
pub export fn noesql_zig_page_close(handle: ?*anyopaque) void {
    if (cast(handle)) |pager| pager.close();
}
pub export fn noesql_zig_page_stage(handle: ?*anyopaque, page_id: u32, bytes: ?[*]const u8, size: usize) i32 {
    const pager = cast(handle) orelse return STATUS_INVALID;
    const ptr = bytes orelse return STATUS_INVALID;
    if (size != storage.PAGE_BYTES) return STATUS_INVALID;
    pager.stage(page_id, ptr[0..size]) catch return STATUS_FAILURE;
    return STATUS_OK;
}
pub export fn noesql_zig_page_commit(handle: ?*anyopaque) i32 {
    const pager = cast(handle) orelse return STATUS_INVALID;
    pager.commit() catch return STATUS_FAILURE;
    return STATUS_OK;
}
pub export fn noesql_zig_page_rollback(handle: ?*anyopaque) i32 {
    const pager = cast(handle) orelse return STATUS_INVALID;
    pager.rollback();
    return STATUS_OK;
}
pub export fn noesql_zig_page_get(handle: ?*anyopaque, page_id: u32, output: ?[*]u8, capacity: usize) i32 {
    const pager = cast(handle) orelse return STATUS_INVALID;
    const src = pager.read(page_id) orelse return STATUS_NOT_FOUND;
    if (capacity < storage.PAGE_BYTES) return STATUS_BUFFER_TOO_SMALL;
    const dest = output orelse return STATUS_INVALID;
    @memcpy(dest[0..storage.PAGE_BYTES], src);
    return STATUS_OK;
}

 // Native WAL-backed single-page integer-key index for trusted Noqeri callers.
 // The caller must reserve an index page distinct from every data/catalog page,
 // and commit or roll back through the same pager handle. No separate index
 // fsync exists: index edits join the Zig pager's existing atomic transaction.
 // Current capacity is one page (337 entries), NOT a B+tree or SQL optimizer.
fn indexPageValid(id: u32) bool {
    return id > 0 and id < storage.MAX_PAGES;
}
fn indexRead(pager: *storage.Pager, page_id: u32) index.IndexError!index.Index {
    const bytes = pager.readPending(page_id) orelse return index.Index{};
    return index.Index.decode(bytes);
}

pub export fn noesql_zig_index_find(
    handle: ?*anyopaque,
    index_page: u32,
    key: i64,
    out_page: ?*u32,
) i32 {
    const pager = cast(handle) orelse return STATUS_INVALID;
    const output = out_page orelse return STATUS_INVALID;
    output.* = 0;
    if (!indexPageValid(index_page)) return STATUS_INVALID;

    const value = indexRead(pager, index_page) catch return STATUS_FAILURE;
    const found = value.find(key) orelse return STATUS_NOT_FOUND;
    output.* = found;
    return STATUS_OK;
}

pub export fn noesql_zig_index_insert(
    handle: ?*anyopaque,
    index_page: u32,
    key: i64,
    data_page: u32,
) i32 {
    const pager = cast(handle) orelse return STATUS_INVALID;
    if (!indexPageValid(index_page) or data_page == 0 or data_page >= storage.MAX_PAGES or data_page == index_page) {
        return STATUS_INVALID;
    }

    var value = indexRead(pager, index_page) catch return STATUS_FAILURE;
    value.insert(key, data_page) catch |err| return switch (err) {
        error.Duplicate => STATUS_DUPLICATE,
        error.IndexFull => STATUS_INDEX_FULL,
        error.CorruptIndex => STATUS_FAILURE,
        error.NotFound => STATUS_FAILURE,
    };
    const encoded = value.encode();
    pager.stage(index_page, &encoded) catch return STATUS_FAILURE;
    return STATUS_OK;
}

pub export fn noesql_zig_index_delete(
    handle: ?*anyopaque,
    index_page: u32,
    key: i64,
) i32 {
    const pager = cast(handle) orelse return STATUS_INVALID;
    if (!indexPageValid(index_page)) return STATUS_INVALID;

    var value = indexRead(pager, index_page) catch return STATUS_FAILURE;
    value.delete(key) catch |err| return switch (err) {
        error.NotFound => STATUS_NOT_FOUND,
        error.CorruptIndex => STATUS_FAILURE,
        error.Duplicate => STATUS_FAILURE,
        error.IndexFull => STATUS_FAILURE,
    };
    const encoded = value.encode();
    pager.stage(index_page, &encoded) catch return STATUS_FAILURE;
    return STATUS_OK;
}

pub export fn noesql_zig_index_range(
    handle: ?*anyopaque,
    index_page: u32,
    minimum: i64,
    maximum: i64,
    keys: ?[*]i64,
    pages: ?[*]u32,
    capacity: usize,
    written: ?*usize,
) i32 {
    const pager = cast(handle) orelse return STATUS_INVALID;
    const out_len = written orelse return STATUS_INVALID;
    out_len.* = 0;
    if (!indexPageValid(index_page)) return STATUS_INVALID;

    const value = indexRead(pager, index_page) catch return STATUS_FAILURE;
    const items = value.range(minimum, maximum);
    out_len.* = items.len;
    if (capacity < items.len) return STATUS_BUFFER_TOO_SMALL;
    if (items.len == 0) return STATUS_OK;

    const output_keys = keys orelse return STATUS_INVALID;
    const output_pages = pages orelse return STATUS_INVALID;
    for (items, 0..) |item, i| {
        output_keys[i] = item.key;
        output_pages[i] = item.page_id;
    }
    return STATUS_OK;
}

// Cross-runtime reference-state container validator. The Zig pager owns page,
// transaction and checksum semantics; the Noqeri executor owns SQL and schema.
fn little32(bytes: []const u8) u32 {
    return @as(u32,bytes[0]) | (@as(u32,bytes[1])<<8) |
        (@as(u32,bytes[2])<<16) | (@as(u32,bytes[3])<<24);
}
pub export fn noesql_zig_page_validate_reference_state(handle: ?*anyopaque) i32 {
    const pager=cast(handle) orelse return STATUS_INVALID;
    const manifest=pager.read(0) orelse return STATUS_NOT_FOUND;
    if (!std.mem.eql(u8,manifest[0..8],"NQRS0001") or little32(manifest[8..12])!=1) return STATUS_INVALID;
    const count:usize=@intCast(little32(manifest[12..16]));
    const length:usize=@intCast(little32(manifest[16..20]));
    if (count==0 or count>=storage.MAX_TX_PAGES or length==0 or
        length>(count*storage.PAGE_BYTES) or length<=((count-1)*storage.PAGE_BYTES)) return STATUS_INVALID;
    for (manifest[52..]) |byte| {if (byte!=0) return STATUS_INVALID;}
    var hasher=std.crypto.hash.sha2.Sha256.init(.{});
    var remaining=length;
    for (0..count) |i| {
        // Page zero contains a valid manifest; a missing referenced data
        // page is corruption, not an empty database. Returning NOT_FOUND
        // here makes Rust load_or_default silently instantiate a fresh DB.
        const page=pager.read(@intCast(i+1)) orelse return STATUS_FAILURE;
        const used=@min(remaining,storage.PAGE_BYTES);
        hasher.update(page[0..used]);
        for (page[used..]) |byte| {if (byte!=0) return STATUS_FAILURE;}
        remaining-=used;
    }
    var digest:[32]u8=undefined;
    hasher.final(&digest);
    if (!std.mem.eql(u8,manifest[20..52],&digest)) return STATUS_FAILURE;
    // A prior shorter rewrite must not expose orphaned bytes through the
    // physical page API. Unexpected nonzero pages are corruption, even when
    // the manifest payload itself has a valid SHA-256 digest.
    for (count + 1..storage.MAX_PAGES) |page_id| {
        if (pager.read(@intCast(page_id))) |tail| {
            for (tail) |byte| {
                if (byte != 0) return STATUS_FAILURE;
            }
        }
    }
    return STATUS_OK;
}

pub export fn noesql_zig_page_generation(handle: ?*anyopaque) u64 {
    const pager = cast(handle) orelse return 0;
    return pager.generation;
}
pub export fn noesql_zig_page_checkpoint(handle: ?*anyopaque) i32 {
    const pager = cast(handle) orelse return STATUS_INVALID;
    pager.checkpoint() catch return STATUS_FAILURE;
    return STATUS_OK;
}

fn putLittle32(bytes: []u8, value: u32) void {
    bytes[0] = @intCast(value & 0xff);
    bytes[1] = @intCast((value >> 8) & 0xff);
    bytes[2] = @intCast((value >> 16) & 0xff);
    bytes[3] = @intCast((value >> 24) & 0xff);
}

// Atomic application-state snapshot used by the Rust host. The payload format
// is Rust-owned; Zig owns pages, WAL, checksum manifest and commit ordering.
pub export fn noesql_zig_snapshot_write(handle: ?*anyopaque, input: ?[*]const u8, size: usize) i32 {
    const pager = cast(handle) orelse return STATUS_INVALID;
    const source_ptr = input orelse return STATUS_INVALID;
    if (size == 0 or size > (storage.MAX_TX_PAGES - 1) * storage.PAGE_BYTES) return STATUS_INVALID;

    // The exported Zig ABI must enforce format isolation itself. Otherwise a
    // caller bypassing Rust could replace raw-row or index pages with a
    // reference snapshot and permanently lose the original application data.
    // Do not combine a snapshot overwrite with unrelated pending mutations.
    if (pager.pending_count != 0) return STATUS_INVALID;
    var previous_count: usize = 0;
    if (pager.read(0)) |header| {
        if (!std.mem.eql(u8, header[0..8], "NQRS0001")) return STATUS_INVALID;
        // A correct outer WAL checksum does not prove the logical snapshot is
        // uncorrupted. Never overwrite an invalid manifest/payload digest.
        if (noesql_zig_page_validate_reference_state(handle) != STATUS_OK) return STATUS_FAILURE;
        previous_count = @intCast(little32(header[12..16]));
    } else {
        // Unmarked pre-migration physical pages are *not* an empty database.
        for (1..storage.MAX_PAGES) |page_id| {
            if (pager.read(@intCast(page_id)) != null) return STATUS_INVALID;
        }
    }
    const count = (size + storage.PAGE_BYTES - 1) / storage.PAGE_BYTES;
    if (count == 0 or count >= storage.MAX_TX_PAGES) return STATUS_INVALID;

    var manifest = [_]u8{0} ** storage.PAGE_BYTES;
    @memcpy(manifest[0..8], "NQRS0001");
    putLittle32(manifest[8..12], 1);
    putLittle32(manifest[12..16], @intCast(count));
    putLittle32(manifest[16..20], @intCast(size));

    var hasher = std.crypto.hash.sha2.Sha256.init(.{});
    hasher.update(source_ptr[0..size]);
    var digest: [32]u8 = undefined;
    hasher.final(&digest);
    @memcpy(manifest[20..52], &digest);

    var dirty = false;
    // The v2 snapshot manifest stores a whole-payload digest, so stage page
    // zero only if its content really changed. All dirty payload pages and
    // the manifest still share ONE atomic WAL transaction.
    const old_manifest = pager.read(0);
    if (old_manifest == null or !std.mem.eql(u8, old_manifest.?, &manifest)) {
        pager.stage(0, &manifest) catch {
            pager.rollback();
            return STATUS_FAILURE;
        };
        dirty = true;
    }

    var offset: usize = 0;
    var page_index: usize = 0;
    while (page_index < count) : (page_index += 1) {
        var page = [_]u8{0} ** storage.PAGE_BYTES;
        const remaining = size - offset;
        const used = @min(remaining, storage.PAGE_BYTES);
        @memcpy(page[0..used], source_ptr[offset .. offset + used]);
        const id: u32 = @intCast(page_index + 1);
        const prior = pager.read(id);
        if (prior == null or !std.mem.eql(u8, prior.?, &page)) {
            pager.stage(id, &page) catch {
                pager.rollback();
                return STATUS_FAILURE;
            };
            dirty = true;
        }
        offset += used;
    }

    // v2 cannot deallocate physical pages. Scrub now-obsolete payload pages
    // atomically when a snapshot shrinks, but avoid writing already-zero
    // pages. The v3 allocator implements a separate free-space prototype.
    if (previous_count > count) {
        for (count + 1..previous_count + 1) |page_id| {
            const zero = [_]u8{0} ** storage.PAGE_BYTES;
            const id: u32 = @intCast(page_id);
            const prior = pager.read(id);
            if (prior != null and !std.mem.eql(u8, prior.?, &zero)) {
                pager.stage(id, &zero) catch {
                    pager.rollback();
                    return STATUS_FAILURE;
                };
                dirty = true;
            }
        }
    }

    // Identical content must not add generations or consume WAL space.
    if (!dirty) return STATUS_OK;
    pager.commit() catch {
        pager.rollback();
        return STATUS_FAILURE;
    };
    return STATUS_OK;
}

pub export fn noesql_zig_snapshot_read(handle: ?*anyopaque, output: ?[*]u8, capacity: usize, written: ?*usize) i32 {
    const pager = cast(handle) orelse return STATUS_INVALID;
    const out_len = written orelse return STATUS_INVALID;
    out_len.* = 0;

    const valid = noesql_zig_page_validate_reference_state(handle);
    if (valid != STATUS_OK) return valid;

    const manifest = pager.read(0) orelse return STATUS_NOT_FOUND;
    const count: usize = @intCast(little32(manifest[12..16]));
    const length: usize = @intCast(little32(manifest[16..20]));
    out_len.* = length;
    if (capacity < length) return STATUS_BUFFER_TOO_SMALL;
    const dest = output orelse return STATUS_INVALID;

    var remaining = length;
    var offset: usize = 0;
    var page_index: usize = 0;
    while (page_index < count) : (page_index += 1) {
        const page = pager.read(@intCast(page_index + 1)) orelse return STATUS_NOT_FOUND;
        const used = @min(remaining, storage.PAGE_BYTES);
        @memcpy(dest[offset .. offset + used], page[0..used]);
        offset += used;
        remaining -= used;
    }
    return STATUS_OK;
}

test "native ABI rejects untrusted arguments without dereferencing page buffers" {
    var handle: ?*anyopaque = undefined;
    try std.testing.expectEqual(STATUS_INVALID, noesql_zig_page_open(3, null, &handle));
    try std.testing.expect(handle == null);
    try std.testing.expectEqual(STATUS_INVALID, noesql_zig_page_stage(null, 0, null, 0));
    try std.testing.expectEqual(STATUS_INVALID, noesql_zig_page_commit(null));
    try std.testing.expectEqual(STATUS_INVALID, noesql_zig_page_rollback(null));
    try std.testing.expectEqual(STATUS_INVALID, noesql_zig_page_get(null, 0, null, 0));
    try std.testing.expectEqual(STATUS_INVALID, noesql_zig_page_validate_reference_state(null));
}

test "index ABI stages read-your-writes, rollback, WAL commit, cold reopen and range" {
    const allocator = std.testing.allocator;
    const suffix = std.crypto.random.int(u64);
    const path = try std.fmt.allocPrint(allocator, "/tmp/noesql-index-abi-{x}.noedb", .{suffix});
    defer allocator.free(path);
    var path_buf: [4096]u8 = undefined;
    var wal_buf: [4096]u8 = undefined;
    var lock_buf: [4096]u8 = undefined;
    const data = try std.fmt.bufPrintZ(&path_buf, "{s}", .{path});
    const journal = try std.fmt.bufPrintZ(&wal_buf, "{s}.wal", .{path});
    const mutex = try std.fmt.bufPrintZ(&lock_buf, "{s}.lck", .{path});
    const c = struct { extern "c" fn unlink([*:0]const u8) c_int; };
    defer {
        _ = c.unlink(data.ptr);
        _ = c.unlink(journal.ptr);
        _ = c.unlink(mutex.ptr);
    }
    var handle: ?*anyopaque = null;
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_open(ABI_VERSION, data.ptr, &handle));
    defer noesql_zig_page_close(handle);
    var row: u32 = 0;
    try std.testing.expectEqual(STATUS_INVALID, noesql_zig_index_insert(handle, 0, 5, 8));
    try std.testing.expectEqual(STATUS_OK, noesql_zig_index_insert(handle, 7, 12, 8));
    try std.testing.expectEqual(STATUS_OK, noesql_zig_index_insert(handle, 7, -4, 9));
    try std.testing.expectEqual(STATUS_DUPLICATE, noesql_zig_index_insert(handle, 7, -4, 11));
    try std.testing.expectEqual(STATUS_OK, noesql_zig_index_find(handle, 7, -4, &row));
    try std.testing.expectEqual(@as(u32, 9), row);
    var keys: [2]i64 = undefined;
    var pages: [2]u32 = undefined;
    var count: usize = 0;
    try std.testing.expectEqual(STATUS_BUFFER_TOO_SMALL, noesql_zig_index_range(handle, 7, -10, 20, &keys, &pages, 1, &count));
    try std.testing.expectEqual(@as(usize, 2), count);
    try std.testing.expectEqual(STATUS_OK, noesql_zig_index_range(handle, 7, -10, 20, &keys, &pages, 2, &count));
    try std.testing.expectEqual(@as(i64, -4), keys[0]);
    try std.testing.expectEqual(@as(i64, 12), keys[1]);
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_rollback(handle));
    try std.testing.expectEqual(STATUS_NOT_FOUND, noesql_zig_index_find(handle, 7, 12, &row));
    try std.testing.expectEqual(STATUS_OK, noesql_zig_index_insert(handle, 7, -4, 9));
    try std.testing.expectEqual(STATUS_OK, noesql_zig_index_insert(handle, 7, 12, 8));
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_commit(handle));
    noesql_zig_page_close(handle);
    handle = null;
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_open(ABI_VERSION, data.ptr, &handle));
    try std.testing.expectEqual(STATUS_OK, noesql_zig_index_find(handle, 7, 12, &row));
    try std.testing.expectEqual(@as(u32, 8), row);
    try std.testing.expectEqual(STATUS_OK, noesql_zig_index_delete(handle, 7, -4));
    try std.testing.expectEqual(STATUS_NOT_FOUND, noesql_zig_index_find(handle, 7, -4, &row));
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_commit(handle));
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_checkpoint(handle));
    noesql_zig_page_close(handle);
    handle = null;
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_open(ABI_VERSION, data.ptr, &handle));
    try std.testing.expectEqual(STATUS_NOT_FOUND, noesql_zig_index_find(handle, 7, -4, &row));
    try std.testing.expectEqual(STATUS_OK, noesql_zig_index_find(handle, 7, 12, &row));
}


test "snapshot ABI atomically persists and reloads Rust-owned payload bytes" {
    const allocator = std.testing.allocator;
    const suffix = std.crypto.random.int(u64);
    const path = try std.fmt.allocPrint(allocator, "/tmp/noesql-snapshot-abi-{x}.noedb", .{suffix});
    defer allocator.free(path);
    var path_buf: [4096]u8 = undefined;
    var wal_buf: [4096]u8 = undefined;
    var lock_buf: [4096]u8 = undefined;
    const data = try std.fmt.bufPrintZ(&path_buf, "{s}", .{path});
    const journal = try std.fmt.bufPrintZ(&wal_buf, "{s}.wal", .{path});
    const mutex = try std.fmt.bufPrintZ(&lock_buf, "{s}.lck", .{path});
    const c_api = struct { extern "c" fn unlink([*:0]const u8) c_int; };
    defer {
        _ = c_api.unlink(data.ptr);
        _ = c_api.unlink(journal.ptr);
        _ = c_api.unlink(mutex.ptr);
    }

    var handle: ?*anyopaque = null;
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_open(ABI_VERSION, data.ptr, &handle));
    const payload = "NQRP0001-rust-owned-snapshot-payload";
    try std.testing.expectEqual(STATUS_OK, noesql_zig_snapshot_write(handle, payload.ptr, payload.len));

    var required: usize = 0;
    try std.testing.expectEqual(STATUS_BUFFER_TOO_SMALL, noesql_zig_snapshot_read(handle, null, 0, &required));
    try std.testing.expectEqual(payload.len, required);

    var output: [128]u8 = undefined;
    var written: usize = 0;
    try std.testing.expectEqual(STATUS_OK, noesql_zig_snapshot_read(handle, &output, output.len, &written));
    try std.testing.expectEqual(payload.len, written);
    try std.testing.expect(std.mem.eql(u8, payload, output[0..written]));

    noesql_zig_page_close(handle);
    handle = null;
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_open(ABI_VERSION, data.ptr, &handle));
    defer noesql_zig_page_close(handle);
    written = 0;
    try std.testing.expectEqual(STATUS_OK, noesql_zig_snapshot_read(handle, &output, output.len, &written));
    try std.testing.expect(std.mem.eql(u8, payload, output[0..written]));
}
test "snapshot ABI rejects raw stores, pending mutations and logical corruption" {
    const allocator = std.testing.allocator;
    const suffix = std.crypto.random.int(u64);
    const path = try std.fmt.allocPrint(allocator, "/tmp/noesql-snapshot-isolation-{x}.noedb", .{suffix});
    defer allocator.free(path);
    var db_buf: [4096]u8 = undefined;
    var wal_buf: [4096]u8 = undefined;
    var lock_buf: [4096]u8 = undefined;
    const name = try std.fmt.bufPrintZ(&db_buf, "{s}", .{path});
    const journal = try std.fmt.bufPrintZ(&wal_buf, "{s}.wal", .{path});
    const mutex = try std.fmt.bufPrintZ(&lock_buf, "{s}.lck", .{path});
    const libc = struct { extern "c" fn unlink([*:0]const u8) c_int; };
    defer {
        _ = libc.unlink(name.ptr);
        _ = libc.unlink(journal.ptr);
        _ = libc.unlink(mutex.ptr);
    }
    const payload: []const u8 = "a valid reference snapshot";
    var handle: ?*anyopaque = null;
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_open(ABI_VERSION, name.ptr, &handle));

    // A dirty user transaction must never be accidentally incorporated in
    // the snapshot API's atomic commit.
    var page: [storage.PAGE_BYTES]u8 = [_]u8{0} ** storage.PAGE_BYTES;
    page[0] = 1;
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_stage(handle, 5, &page, page.len));
    try std.testing.expectEqual(STATUS_INVALID, noesql_zig_snapshot_write(handle, payload.ptr, payload.len));
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_rollback(handle));

    // Durable legacy pages with no manifest must be preserved.
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_stage(handle, 5, &page, page.len));
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_commit(handle));
    try std.testing.expectEqual(STATUS_INVALID, noesql_zig_snapshot_write(handle, payload.ptr, payload.len));
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_get(handle, 5, &page, page.len));
    try std.testing.expectEqual(@as(u8, 1), page[0]);
    noesql_zig_page_close(handle);
    handle = null;

    // A separate empty store can hold a valid snapshot. If a page subsequently
    // changes without updating the logical manifest, overwrites fail closed.
    const fresh_path = try std.fmt.allocPrint(allocator, "{s}.fresh.noedb", .{path});
    defer allocator.free(fresh_path);
    var fresh_buf: [4096]u8 = undefined;
    var fresh_wal_buf: [4096]u8 = undefined;
    var fresh_lock_buf: [4096]u8 = undefined;
    const fresh = try std.fmt.bufPrintZ(&fresh_buf, "{s}", .{fresh_path});
    const fresh_wal = try std.fmt.bufPrintZ(&fresh_wal_buf, "{s}.wal", .{fresh_path});
    const fresh_lock = try std.fmt.bufPrintZ(&fresh_lock_buf, "{s}.lck", .{fresh_path});
    defer {
        _ = libc.unlink(fresh.ptr);
        _ = libc.unlink(fresh_wal.ptr);
        _ = libc.unlink(fresh_lock.ptr);
    }
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_open(ABI_VERSION, fresh.ptr, &handle));
    defer noesql_zig_page_close(handle);
    try std.testing.expectEqual(STATUS_OK, noesql_zig_snapshot_write(handle, payload.ptr, payload.len));
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_get(handle, 1, &page, page.len));
    page[0] ^= 0x40;
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_stage(handle, 1, &page, page.len));
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_commit(handle));
    try std.testing.expectEqual(STATUS_FAILURE, noesql_zig_page_validate_reference_state(handle));
    try std.testing.expectEqual(STATUS_FAILURE, noesql_zig_snapshot_write(handle, payload.ptr, payload.len));
}

test "snapshot ABI repeatedly rewrites and cold-opens WAL before and after checkpoint" {
    const allocator = std.testing.allocator;
    const suffix = std.crypto.random.int(u64);
    const path = try std.fmt.allocPrint(allocator, "/tmp/noesql-snapshot-stress-{x}.noedb", .{suffix});
    defer allocator.free(path);
    var db_buf: [4096]u8 = undefined;
    var wal_buf: [4096]u8 = undefined;
    var lock_buf: [4096]u8 = undefined;
    const name = try std.fmt.bufPrintZ(&db_buf, "{s}", .{path});
    const journal = try std.fmt.bufPrintZ(&wal_buf, "{s}.wal", .{path});
    const mutex = try std.fmt.bufPrintZ(&lock_buf, "{s}.lck", .{path});
    const libc = struct { extern "c" fn unlink([*:0]const u8) c_int; };
    defer {
        _ = libc.unlink(name.ptr);
        _ = libc.unlink(journal.ptr);
        _ = libc.unlink(mutex.ptr);
    }
    var handle: ?*anyopaque = null;
    var payload: [160]u8 = [_]u8{0} ** 160;
    var readback: [160]u8 = undefined;
    for (0..96) |iteration| {
        try std.testing.expectEqual(STATUS_OK, noesql_zig_page_open(ABI_VERSION, name.ptr, &handle));
        for (&payload, 0..) |*byte, i| {
            byte.* = @truncate((iteration * 31 + i) % 251);
        }
        try std.testing.expectEqual(STATUS_OK, noesql_zig_snapshot_write(handle, &payload, payload.len));
        if (iteration % 7 == 0) {
            try std.testing.expectEqual(STATUS_OK, noesql_zig_page_checkpoint(handle));
        }
        noesql_zig_page_close(handle);
        handle = null;

        try std.testing.expectEqual(STATUS_OK, noesql_zig_page_open(ABI_VERSION, name.ptr, &handle));
        var written: usize = 0;
        try std.testing.expectEqual(STATUS_OK, noesql_zig_snapshot_read(handle, &readback, readback.len, &written));
        try std.testing.expectEqual(payload.len, written);
        try std.testing.expect(std.mem.eql(u8, &readback, &payload));
        noesql_zig_page_close(handle);
        handle = null;
    }
}

test "incomplete snapshot is corruption and cannot be interpreted as empty" {
    const allocator = std.testing.allocator;
    const suffix = std.crypto.random.int(u64);
    const path = try std.fmt.allocPrint(allocator, "/tmp/noesql-missing-snapshot-page-{x}.noedb", .{suffix});
    defer allocator.free(path);
    var dbbuf: [4096]u8 = undefined;
    var walbuf: [4096]u8 = undefined;
    var lockbuf: [4096]u8 = undefined;
    const data = try std.fmt.bufPrintZ(&dbbuf, "{s}", .{path});
    const journal = try std.fmt.bufPrintZ(&walbuf, "{s}.wal", .{path});
    const mutex = try std.fmt.bufPrintZ(&lockbuf, "{s}.lck", .{path});
    const libc = struct { extern "c" fn unlink([*:0]const u8) c_int; };
    defer {
        _ = libc.unlink(data.ptr);
        _ = libc.unlink(journal.ptr);
        _ = libc.unlink(mutex.ptr);
    }

    var handle: ?*anyopaque = null;
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_open(ABI_VERSION, data.ptr, &handle));
    defer noesql_zig_page_close(handle);

    // A structurally plausible manifest references two pages, but only the
    // first one is stored. The missing second page must fail closed.
    var manifest = [_]u8{0} ** storage.PAGE_BYTES;
    @memcpy(manifest[0..8], "NQRS0001");
    putLittle32(manifest[8..12], 1);
    putLittle32(manifest[12..16], 2);
    putLittle32(manifest[16..20], storage.PAGE_BYTES + 1);
    const data_page = [_]u8{0} ** storage.PAGE_BYTES;
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_stage(handle, 0, &manifest, manifest.len));
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_stage(handle, 1, &data_page, data_page.len));
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_commit(handle));

    var written: usize = 99;
    try std.testing.expectEqual(STATUS_FAILURE, noesql_zig_page_validate_reference_state(handle));
    try std.testing.expectEqual(STATUS_FAILURE, noesql_zig_snapshot_read(handle, null, 0, &written));
    try std.testing.expectEqual(@as(usize, 0), written);
    const replacement: []const u8 = "x";
    try std.testing.expectEqual(STATUS_FAILURE, noesql_zig_snapshot_write(handle, replacement.ptr, replacement.len));
}

test "shortening a snapshot scrubs stale physical pages atomically" {
    const allocator = std.testing.allocator;
    const suffix = std.crypto.random.int(u64);
    const path = try std.fmt.allocPrint(allocator, "/tmp/noesql-shrink-snapshot-{x}.noedb", .{suffix});
    defer allocator.free(path);
    var dbbuf: [4096]u8 = undefined;
    var walbuf: [4096]u8 = undefined;
    var lockbuf: [4096]u8 = undefined;
    const data = try std.fmt.bufPrintZ(&dbbuf, "{s}", .{path});
    const journal = try std.fmt.bufPrintZ(&walbuf, "{s}.wal", .{path});
    const mutex = try std.fmt.bufPrintZ(&lockbuf, "{s}.lck", .{path});
    const libc = struct { extern "c" fn unlink([*:0]const u8) c_int; };
    defer {
        _ = libc.unlink(data.ptr);
        _ = libc.unlink(journal.ptr);
        _ = libc.unlink(mutex.ptr);
    }

    const large: [3 * storage.PAGE_BYTES]u8 = [_]u8{0xab} ** (3 * storage.PAGE_BYTES);
    const small: []const u8 = "clean";
    var handle: ?*anyopaque = null;
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_open(ABI_VERSION, data.ptr, &handle));
    try std.testing.expectEqual(STATUS_OK, noesql_zig_snapshot_write(handle, &large, large.len));
    try std.testing.expectEqual(STATUS_OK, noesql_zig_snapshot_write(handle, small.ptr, small.len));
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_checkpoint(handle));
    noesql_zig_page_close(handle);
    handle = null;
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_open(ABI_VERSION, data.ptr, &handle));
    defer noesql_zig_page_close(handle);

    var bytes: [storage.PAGE_BYTES]u8 = undefined;
    for ([_]u32{2, 3}) |page_id| {
        try std.testing.expectEqual(STATUS_OK, noesql_zig_page_get(handle, page_id, &bytes, bytes.len));
        for (bytes) |byte| try std.testing.expectEqual(@as(u8, 0), byte);
    }
    try std.testing.expectEqual(STATUS_OK, noesql_zig_page_validate_reference_state(handle));
    var readback: [8]u8 = undefined;
    var written: usize = 0;
    try std.testing.expectEqual(STATUS_OK, noesql_zig_snapshot_read(handle, &readback, readback.len, &written));
    try std.testing.expectEqual(small.len, written);
    try std.testing.expect(std.mem.eql(u8, small, readback[0..written]));
}
