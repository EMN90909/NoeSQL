const std = @import("std");
const pager = @import("pager_v2.zig");

pub const COMPONENT_VERSION = "1.0.0"; // component API, not a B+tree
// Experimental physical integer-key index page, backed by the Zig WAL pager.
// Single-node index with a strict capacity bound; B+tree fanout/splits, NULL
// ordering, composite values, and SQL planning are not provided yet.
pub const MAGIC = "NQIX0002";
pub const HEADER: usize = 16;
pub const TRAILER: usize = 32;
pub const ENTRY: usize = 12; // signed 64-bit key, unsigned page id
pub const CAPACITY = (pager.PAGE_BYTES - HEADER - TRAILER) / ENTRY;
pub const IndexError = error{ CorruptIndex, Duplicate, IndexFull, NotFound };
pub const Entry = struct { key: i64, page_id: u32 };

fn put32(dst: []u8, value: u32) void {
    for (0..4) |i| dst[i] = @truncate(value >> @as(u5, @intCast(i * 8)));
}
fn put64(dst: []u8, value: u64) void {
    for (0..8) |i| dst[i] = @truncate(value >> @as(u6, @intCast(i * 8)));
}
fn get32(src: []const u8) u32 {
    var x: u32 = 0;
    for (0..4) |i| x |= @as(u32, src[i]) << @as(u5, @intCast(i * 8));
    return x;
}
fn get64(src: []const u8) u64 {
    var x: u64 = 0;
    for (0..8) |i| x |= @as(u64, src[i]) << @as(u6, @intCast(i * 8));
    return x;
}
fn checksum(data: []const u8) [32]u8 {
    var digest: [32]u8 = undefined;
    std.crypto.hash.sha2.Sha256.hash(data, &digest, .{});
    return digest;
}

pub const Index = struct {
    entries: [CAPACITY]Entry = undefined,
    count: usize = 0,

    pub fn decode(page: []const u8) IndexError!Index {
        if (page.len != pager.PAGE_BYTES or !std.mem.eql(u8, page[0..8], MAGIC)) return error.CorruptIndex;
        const hash = checksum(page[0 .. pager.PAGE_BYTES - TRAILER]);
        if (!std.mem.eql(u8, page[pager.PAGE_BYTES - TRAILER ..], &hash)) return error.CorruptIndex;
        const length = get32(page[8..12]);
        if (length > CAPACITY or get32(page[12..16]) != 0) return error.CorruptIndex;
        var result = Index{};
        result.count = length;
        for (0..result.count) |i| {
            const at = HEADER + i * ENTRY;
            const signed: i64 = @bitCast(get64(page[at .. at + 8]));
            const id = get32(page[at + 8 .. at + 12]);
            if (id >= pager.MAX_PAGES or (i != 0 and result.entries[i - 1].key >= signed)) return error.CorruptIndex;
            result.entries[i] = .{ .key = signed, .page_id = id };
        }
        return result;
    }
    pub fn encode(self: *const Index) [pager.PAGE_BYTES]u8 {
        var page: [pager.PAGE_BYTES]u8 = [_]u8{0} ** pager.PAGE_BYTES;
        @memcpy(page[0..8], MAGIC);
        put32(page[8..12], @intCast(self.count));
        for (self.entries[0..self.count], 0..) |entry, i| {
            const at = HEADER + i * ENTRY;
            put64(page[at .. at + 8], @bitCast(entry.key));
            put32(page[at + 8 .. at + 12], entry.page_id);
        }
        const hash = checksum(page[0 .. pager.PAGE_BYTES - TRAILER]);
        @memcpy(page[pager.PAGE_BYTES - TRAILER ..], &hash);
        return page;
    }
    fn lowerBound(self: *const Index, key: i64) usize {
        var lo: usize = 0;
        var hi = self.count;
        while (lo < hi) {
            const mid = lo + (hi - lo) / 2;
            if (self.entries[mid].key < key) lo = mid + 1 else hi = mid;
        }
        return lo;
    }
    pub fn find(self: *const Index, key: i64) ?u32 {
        const at = self.lowerBound(key);
        if (at >= self.count or self.entries[at].key != key) return null;
        return self.entries[at].page_id;
    }
    fn upperBound(self: *const Index, key: i64) usize {
        var lo: usize = 0;
        var hi = self.count;
        while (lo < hi) {
            const mid = lo + (hi - lo) / 2;
            if (self.entries[mid].key <= key) lo = mid + 1 else hi = mid;
        }
        return lo;
    }
    pub fn range(self: *const Index, min: i64, max: i64) []const Entry {
        const first = self.lowerBound(min);
        if (min > max) return self.entries[first..first];
        return self.entries[first..self.upperBound(max)];
    }
    pub fn insert(self: *Index, key: i64, page_id: u32) IndexError!void {
        if (page_id >= pager.MAX_PAGES) return error.CorruptIndex;
        const at = self.lowerBound(key);
        if (at < self.count and self.entries[at].key == key) return error.Duplicate;
        if (self.count >= CAPACITY) return error.IndexFull;
        var i = self.count;
        while (i > at) : (i -= 1) self.entries[i] = self.entries[i - 1];
        self.entries[at] = .{ .key = key, .page_id = page_id };
        self.count += 1;
    }
    pub fn delete(self: *Index, key: i64) IndexError!void {
        const at = self.lowerBound(key);
        if (at >= self.count or self.entries[at].key != key) return error.NotFound;
        var i = at;
        while (i + 1 < self.count) : (i += 1) self.entries[i] = self.entries[i + 1];
        self.count -= 1;
    }
};

test "persistent index bytes support unique lookup and bounded sorted range" {
    var index = Index{};
    try index.insert(20, 2);
    try index.insert(-2, 7);
    try index.insert(5, 4);
    try std.testing.expectError(error.Duplicate, index.insert(5, 9));
    try std.testing.expectEqual(@as(?u32, 7), index.find(-2));
    const bytes = index.encode();
    var loaded = try Index.decode(&bytes);
    try std.testing.expectEqual(@as(usize, 2), loaded.range(-2, 5).len);
    try std.testing.expectEqual(@as(usize, 0), loaded.range(50, -50).len);
    try std.testing.expectEqual(@as(usize, 1), loaded.range(20, 20).len);
    try std.testing.expectEqual(@as(?u32, 2), loaded.find(20));
    try loaded.delete(20);
    try std.testing.expectEqual(@as(?u32, null), loaded.find(20));
    var corrupt = bytes;
    corrupt[25] ^= 1;
    try std.testing.expectError(error.CorruptIndex, Index.decode(&corrupt));
}


test "index page survives Zig WAL recovery and checkpoint" {
    const a = std.testing.allocator;
    const suffix = std.crypto.random.int(u64);
    const path = try std.fmt.allocPrint(a, "/tmp/noesql-zig-index-{x}.nqdb", .{suffix});
    defer a.free(path);
    var db: [4096]u8 = undefined;
    var wal: [4096]u8 = undefined;
    var lock: [4096]u8 = undefined;
    const datum = try std.fmt.bufPrintZ(&db, "{s}", .{path});
    const journal = try std.fmt.bufPrintZ(&wal, "{s}.wal", .{path});
    const mutex = try std.fmt.bufPrintZ(&lock, "{s}.lck", .{path});
    defer {
        const c = struct { extern "c" fn unlink([*:0]const u8) c_int; };
        _ = c.unlink(datum.ptr);
        _ = c.unlink(journal.ptr);
        _ = c.unlink(mutex.ptr);
    }
    const store = try pager.Pager.open(a, path);
    var index = Index{};
    try index.insert(-100, 20);
    try index.insert(0, 21);
    try index.insert(500, 22);
    const encoded = index.encode();
    try store.stage(2, &encoded);
    try store.commit();
    store.close();
    const restored = try pager.Pager.open(a, path);
    defer restored.close();
    const recovered = try Index.decode(restored.read(2).?);
    try std.testing.expectEqual(@as(?u32, 22), recovered.find(500));
    try restored.checkpoint();
}
