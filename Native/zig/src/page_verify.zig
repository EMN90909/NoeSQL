const std = @import("std");

// Read-only bridge for the legacy C++ NQPG0001 page layout.
// Do not write pages with this module: it has no WAL, locking or fsync.
// The legacy decoder still owns B+tree consistency verification.
pub const PAGE_BYTES: usize = 4096;
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
pub const VerifyError = error{ InvalidSize, InvalidMagic, InvalidVersion, InvalidPageSize, InvalidRoot, ChecksumMismatch };
pub const Header = struct {
    page_count: usize,
    root_page: u32,
};

fn readU32(page: []const u8, offset: usize) u32 {
    return @as(u32, page[offset]) |
        (@as(u32, page[offset + 1]) << 8) |
        (@as(u32, page[offset + 2]) << 16) |
        (@as(u32, page[offset + 3]) << 24);
}

fn checkPage(page: []const u8) VerifyError!void {
    var bytes: [PAGE_BYTES]u8 = undefined;
    @memcpy(bytes[0..], page);
    var stored: [32]u8 = undefined;
    @memcpy(stored[0..], page[8..40]);
    @memset(bytes[8..40], 0);
    var digest: [32]u8 = undefined;
    std.crypto.hash.sha2.Sha256.hash(bytes[0..], &digest, .{});
    if (!std.mem.eql(u8, stored[0..], digest[0..])) return error.ChecksumMismatch;
}

pub fn verifyLegacyImage(image: []const u8) VerifyError!Header {
    if (image.len < 2 * PAGE_BYTES or image.len > MAX_IMAGE_BYTES or image.len % PAGE_BYTES != 0) return error.InvalidSize;
    const count = image.len / PAGE_BYTES;
    // Reject corrupt payloads before trusting any page header fields.
    var id: usize = 0;
    while (id < count) : (id += 1) {
        try checkPage(image[id * PAGE_BYTES .. (id + 1) * PAGE_BYTES]);
    }
    const header = image[0..PAGE_BYTES];
    if (!std.mem.eql(u8, header[0..8], "NQPG0001")) return error.InvalidMagic;
    if (readU32(header, 40) != 1) return error.InvalidVersion;
    if (readU32(header, 44) != @as(u32, @intCast(PAGE_BYTES))) return error.InvalidPageSize;
    const root = readU32(header, 56);
    if (root == 0 or @as(usize, root) >= count) return error.InvalidRoot;
    return .{ .page_count = count, .root_page = root };
}

fn putU32(page: []u8, offset: usize, value: u32) void {
    page[offset] = @truncate(value);
    page[offset + 1] = @truncate(value >> 8);
    page[offset + 2] = @truncate(value >> 16);
    page[offset + 3] = @truncate(value >> 24);
}
fn seal(page: []u8) void {
    @memset(page[8..40], 0);
    var digest: [32]u8 = undefined;
    std.crypto.hash.sha2.Sha256.hash(page, &digest, .{});
    @memcpy(page[8..40], digest[0..]);
}
test "legacy NQPG0001 image header and page checksums" {
    var image = [_]u8{0} ** (PAGE_BYTES * 2);
    @memcpy(image[0..8], "NQPG0001");
    putU32(image[0..PAGE_BYTES], 40, 1);
    putU32(image[0..PAGE_BYTES], 44, @intCast(PAGE_BYTES));
    putU32(image[0..PAGE_BYTES], 56, 1);
    seal(image[0..PAGE_BYTES]);
    seal(image[PAGE_BYTES .. PAGE_BYTES * 2]);
    const header = try verifyLegacyImage(&image);
    try std.testing.expectEqual(@as(usize, 2), header.page_count);
    try std.testing.expectEqual(@as(u32, 1), header.root_page);
    image[PAGE_BYTES + 48] = 42;
    try std.testing.expectError(error.ChecksumMismatch, verifyLegacyImage(&image));
    seal(image[PAGE_BYTES .. PAGE_BYTES * 2]);
    image[0] = 'X';
    seal(image[0..PAGE_BYTES]);
    try std.testing.expectError(error.InvalidMagic, verifyLegacyImage(&image));
}
test "truncated or over-budget images are rejected before reads" {
    const too_small = [_]u8{0} ** PAGE_BYTES;
    try std.testing.expectError(error.InvalidSize, verifyLegacyImage(&too_small));
}
