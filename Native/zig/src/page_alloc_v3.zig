const std = @import("std");

// Standalone v3 capacity/free-space prototype, intentionally NOT the v2 pager.
// NQDB0002 data/WAL must never be reinterpreted as this manifest.
// A later migration must be transactional, with shadow files + recovery tests.
pub const MAGIC = "NQDB0003";
pub const VERSION: u32 = 3;
pub const MAX_PAGES: u32 = 65536; // up to 256 MiB of logical 4 KiB pages
const HEADER_BYTES: usize = 32;
const DIGEST_BYTES: usize = 32;

pub const Error = error{
    InvalidCapacity, OutOfMemory, Full, OutOfRange, AlreadyFree,
    AlreadyAllocated, BadManifest,
};

fn putU32(dst: []u8, value: u32) void {
    dst[0] = @truncate(value);
    dst[1] = @truncate(value >> 8);
    dst[2] = @truncate(value >> 16);
    dst[3] = @truncate(value >> 24);
}
fn getU32(src: []const u8) u32 {
    return @as(u32, src[0]) | (@as(u32, src[1]) << 8) |
        (@as(u32, src[2]) << 16) | (@as(u32, src[3]) << 24);
}
fn bitmapBytes(capacity: u32) usize {
    return (@as(usize, capacity) + 7) / 8;
}
pub const PageMap = struct {
    allocator: std.mem.Allocator,
    capacity: u32,
    bits: []u8,
    live: u32,
    hint: u32,

    pub fn init(allocator: std.mem.Allocator, capacity: u32) Error!PageMap {
        if (capacity < 2 or capacity > MAX_PAGES) return error.InvalidCapacity;
        const bits = allocator.alloc(u8, bitmapBytes(capacity)) catch return error.OutOfMemory;
        @memset(bits, 0);
        bits[0] = 1; // page zero belongs to the catalog and cannot be freed
        return .{ .allocator = allocator, .capacity = capacity, .bits = bits, .live = 1, .hint = 1 };
    }

    pub fn deinit(self: *PageMap) void {
        self.allocator.free(self.bits);
    }

    pub fn isUsed(self: *const PageMap, page: u32) bool {
        if (page >= self.capacity) return false;
        return (self.bits[@as(usize, page) / 8] &
            (@as(u8, 1) << @as(u3, @intCast(page % 8)))) != 0;
    }

    fn set(self: *PageMap, page: u32, used: bool) void {
        const mask = @as(u8, 1) << @as(u3, @intCast(page % 8));
        const index = @as(usize, page) / 8;
        if (used) self.bits[index] |= mask else self.bits[index] &= ~mask;
    }

    pub fn reserve(self: *PageMap, page: u32) Error!void {
        if (page == 0 or page >= self.capacity) return error.OutOfRange;
        if (self.isUsed(page)) return error.AlreadyAllocated;
        self.set(page, true);
        self.live += 1;
        if (page == self.hint) self.hint = if (page + 1 < self.capacity) page + 1 else 1;
    }

    pub fn allocPage(self: *PageMap) Error!u32 {
        if (self.live == self.capacity) return error.Full;
        var attempt: u32 = 0;
        var page = self.hint;
        while (attempt < self.capacity - 1) : (attempt += 1) {
            if (!self.isUsed(page)) {
                try self.reserve(page);
                return page;
            }
            page = if (page + 1 < self.capacity) page + 1 else 1;
        }
        return error.Full;
    }

    pub fn freePage(self: *PageMap, page: u32) Error!void {
        if (page == 0 or page >= self.capacity) return error.OutOfRange;
        if (!self.isUsed(page)) return error.AlreadyFree;
        self.set(page, false);
        self.live -= 1;
        self.hint = page;
    }

    // Versioned, bounded, SHA-256 validated bitmap manifest. This is NOT
    // yet the complete page/WAL file format, and is never installed as v2.
    pub fn encodeManifest(self: *const PageMap) Error![]u8 {
        const payload = HEADER_BYTES + self.bits.len;
        const bytes = self.allocator.alloc(u8, payload + DIGEST_BYTES) catch return error.OutOfMemory;
        @memset(bytes, 0);
        @memcpy(bytes[0..8], MAGIC);
        putU32(bytes[8..12], VERSION);
        putU32(bytes[12..16], self.capacity);
        putU32(bytes[16..20], self.live);
        putU32(bytes[20..24], @intCast(self.bits.len));
        @memcpy(bytes[HEADER_BYTES..payload], self.bits);
        var digest: [DIGEST_BYTES]u8 = undefined;
        std.crypto.hash.sha2.Sha256.hash(bytes[0..payload], &digest, .{});
        @memcpy(bytes[payload..], &digest);
        return bytes;
    }

    pub fn decodeManifest(allocator: std.mem.Allocator, encoded: []const u8) Error!PageMap {
        if (encoded.len < HEADER_BYTES + DIGEST_BYTES) return error.BadManifest;
        if (!std.mem.eql(u8, encoded[0..8], MAGIC) or getU32(encoded[8..12]) != VERSION)
            return error.BadManifest;
        const capacity = getU32(encoded[12..16]);
        if (capacity < 2 or capacity > MAX_PAGES) return error.BadManifest;
        const length = bitmapBytes(capacity);
        if (getU32(encoded[20..24]) != length or
            encoded.len != HEADER_BYTES + length + DIGEST_BYTES) return error.BadManifest;
        for (encoded[24..HEADER_BYTES]) |byte| {
            if (byte != 0) return error.BadManifest;
        }
        const digest_offset = HEADER_BYTES + length;
        var digest: [DIGEST_BYTES]u8 = undefined;
        std.crypto.hash.sha2.Sha256.hash(encoded[0..digest_offset], &digest, .{});
        if (!std.mem.eql(u8, encoded[digest_offset..], &digest)) return error.BadManifest;

        var map = try PageMap.init(allocator, capacity);
        errdefer map.deinit();
        @memcpy(map.bits, encoded[HEADER_BYTES..digest_offset]);
        if (!map.isUsed(0)) return error.BadManifest;
        var counted: u32 = 0;
        for (0..@as(usize, capacity)) |id| {
            if (map.isUsed(@intCast(id))) counted += 1;
        }
        // Unused high bits are reserved and must be zero.
        for (@as(usize, capacity)..length * 8) |id| {
            const mask = @as(u8, 1) << @as(u3, @intCast(id % 8));
            if ((map.bits[id / 8] & mask) != 0) return error.BadManifest;
        }
        if (counted != getU32(encoded[16..20])) return error.BadManifest;
        map.live = counted;
        for (1..@as(usize, capacity)) |id| {
            if (!map.isUsed(@intCast(id))) {
                map.hint = @intCast(id);
                break;
            }
        }
        return map;
    }
};

test "v3 page allocator exceeds 256 pages and reuses free space without aliasing" {
    var map = try PageMap.init(std.testing.allocator, 4096);
    defer map.deinit();
    for (0..1024) |_| {
        const id = try map.allocPage();
        try std.testing.expect(id > 0 and id < 4096);
    }
    try std.testing.expectEqual(@as(u32, 1025), map.live);
    try std.testing.expect(map.isUsed(900));
    try map.freePage(900);
    try std.testing.expect(!map.isUsed(900));
    try std.testing.expectEqual(@as(u32, 900), try map.allocPage());
    try std.testing.expectError(error.AlreadyAllocated, map.reserve(900));
    try std.testing.expectError(error.OutOfRange, map.freePage(0));
}

test "v3 manifest preserves capacity and rejects tampering and malformed lengths" {
    var map = try PageMap.init(std.testing.allocator, 1025);
    defer map.deinit();
    try map.reserve(1024);
    const bytes = try map.encodeManifest();
    defer std.testing.allocator.free(bytes);
    var read = try PageMap.decodeManifest(std.testing.allocator, bytes);
    defer read.deinit();
    try std.testing.expectEqual(@as(u32, 1025), read.capacity);
    try std.testing.expect(read.isUsed(1024));

    const corrupted = try std.testing.allocator.dupe(u8, bytes);
    defer std.testing.allocator.free(corrupted);
    corrupted[HEADER_BYTES] ^= 0x04;
    try std.testing.expectError(error.BadManifest, PageMap.decodeManifest(std.testing.allocator, corrupted));
    try std.testing.expectError(error.BadManifest, PageMap.decodeManifest(std.testing.allocator, bytes[0 .. bytes.len - 1]));
}

test "v3 map allocates full capacity and returns full on overflow" {
    var map = try PageMap.init(std.testing.allocator, 300);
    defer map.deinit();
    for (1..300) |_| {
        _ = try map.allocPage();
    }
    try std.testing.expectEqual(@as(u32, 300), map.live);
    try std.testing.expectError(error.Full, map.allocPage());
}
