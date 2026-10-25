const std = @import("std");
const storage = @import("pager_v2.zig");
const index = @import("index_v2.zig");

// Native Zig-owned pager ABI for FUTURE compiled Noqeri callers.
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

pub export fn noesql_zig_page_open(version: u32, name: ?[*:0]const u8, out: ?*?*anyopaque) i32 {
    const dest = out orelse return STATUS_INVALID;
    dest.* = null;
    if (version != ABI_VERSION) return STATUS_INVALID;
    const path = name orelse return STATUS_INVALID;
    const value = std.mem.span(path);
    if (value.len == 0 or value.len > 4000) return STATUS_INVALID;
    const pager = storage.Pager.open(std.heap.page_allocator, value) catch return STATUS_FAILURE;
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
}
}
}
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
        const page=pager.read(@intCast(i+1)) orelse return STATUS_NOT_FOUND;
        const used=@min(remaining,storage.PAGE_BYTES);
        hasher.update(page[0..used]);
        for (page[used..]) |byte| {if (byte!=0) return STATUS_FAILURE;}
        remaining-=used;
    }
    var digest:[32]u8=undefined;
    hasher.final(&digest);
    if (!std.mem.eql(u8,manifest[20..52],&digest)) return STATUS_FAILURE;
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
    if (size == 0) return STATUS_INVALID;
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

    pager.stage(0, &manifest) catch {
        pager.rollback();
        return STATUS_FAILURE;
    };

    var offset: usize = 0;
    var page_index: usize = 0;
    while (page_index < count) : (page_index += 1) {
        var page = [_]u8{0} ** storage.PAGE_BYTES;
        const remaining = size - offset;
        const used = @min(remaining, storage.PAGE_BYTES);
        @memcpy(page[0..used], source_ptr[offset .. offset + used]);
        pager.stage(@intCast(page_index + 1), &page) catch {
            pager.rollback();
            return STATUS_FAILURE;
        };
        offset += used;
    }

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
// Stub replacement logic
pub export fn noesql_zig_index_find(handle: ?*anyopaque, index_page: u32, key: i64, out_page: ?*u32) i32 {
    _ = handle; _ = index_page; _ = key; _ = out_page; return -1;
}
pub export fn noesql_zig_index_insert(handle: ?*anyopaque, index_page: u32, key: i64, data_page: u32) i32 {
    _ = handle; _ = index_page; _ = key; _ = data_page; return -1;
}
pub export fn noesql_zig_index_delete(handle: ?*anyopaque, index_page: u32, key: i64) i32 {
    _ = handle; _ = index_page; _ = key; return -1;
}
pub export fn noesql_zig_index_range(handle: ?*anyopaque, index_page: u32, minimum: i64, maximum: i64, keys: ?[*]i64, pages: ?[*]u32, capacity: usize, written: ?*usize) i32 {
    _ = handle; _ = index_page; _ = minimum; _ = maximum; _ = keys; _ = pages; _ = capacity; _ = written; return -1;
}
